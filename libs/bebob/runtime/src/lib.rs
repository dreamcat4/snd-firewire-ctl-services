// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2021 Takashi Sakamoto

mod common_ctls;

mod model;

mod apogee;
mod behringer;
mod digidesign;
mod esi;
mod focusrite;
mod icon;
mod maudio;
mod presonus;
mod roland;
mod stanton;
mod terratec;
mod yamaha_terratec;

use {
    alsa_ctl_tlv_codec::DbInterval,
    alsactl::{prelude::*, *},
    core::{card_cntr::*, dispatcher::*, elem_value_accessor::*, RuntimeOperation},
    firewire_bebob_protocols as protocols,
    glib::{source, Error, FileError},
    hinawa::{
        prelude::{FwNodeExt, FwNodeExtManual},
        FwNode, FwReq,
    },
    hitaki::{prelude::*, *},
    ieee1212_config_rom::ConfigRom,
    model::*,
    nix::sys::signal,
    std::{convert::TryFrom, sync::mpsc},
    ta1394_avc_general::config_rom::*,
};

enum Event {
    Shutdown,
    Disconnected,
    BusReset(u32),
    Elem(ElemId, ElemEventMask),
    Timer,
    StreamLock(bool),
}

pub struct BebobRuntime {
    unit: (SndUnit, FwNode),
    model: BebobModel,
    card_cntr: CardCntr,
    rx: mpsc::Receiver<Event>,
    tx: mpsc::SyncSender<Event>,
    dispatchers: Vec<Dispatcher>,
    timer: Option<Dispatcher>,
}

impl Drop for BebobRuntime {
    fn drop(&mut self) {
        // At first, stop event loop in all of dispatchers to avoid queueing new events.
        for dispatcher in &mut self.dispatchers {
            dispatcher.stop();
        }

        // Next, consume all events in queue to release blocked thread for sender.
        for _ in self.rx.try_iter() {}

        // Finally Finish I/O threads.
        self.dispatchers.clear();
    }
}

impl RuntimeOperation<u32> for BebobRuntime {
    fn new(card_id: u32) -> Result<Self, Error> {
        let path = format!("/dev/snd/hwC{}D0", card_id);
        let unit = SndUnit::new();
        unit.open(&path, 0)?;

        if unit.unit_type() != AlsaFirewireType::Bebob {
            let label = "ALSA bebob driver is not bound to the unit.";
            return Err(Error::new(FileError::Inval, label));
        }

        let path = format!("/dev/{}", unit.node_device().unwrap());
        let node = FwNode::new();
        node.open(&path)?;

        let raw = node.config_rom()?;

        let config_rom = ConfigRom::try_from(raw).map_err(|e| {
            let label = format!("Malformed configuration ROM detected: {}", e);
            Error::new(FileError::Nxio, &label)
        })?;

        let (vendor, model) = config_rom
            .get_vendor()
            .and_then(|vendor| config_rom.get_model().map(|model| (vendor, model)))
            .ok_or(Error::new(
                FileError::Nxio,
                "Configuration ROM is not for 1394TA standard",
            ))?;

        let model = BebobModel::new(vendor.vendor_id, model.model_id, model.model_name)?;

        let card_cntr = CardCntr::default();
        card_cntr.card.open(card_id, 0)?;

        // Use uni-directional channel for communication to child threads.
        let (tx, rx) = mpsc::sync_channel(32);

        Ok(BebobRuntime {
            unit: (unit, node),
            model,
            card_cntr,
            rx,
            tx,
            dispatchers: Vec::new(),
            timer: None,
        })
    }

    fn listen(&mut self) -> Result<(), Error> {
        self.launch_node_event_dispatcher()?;
        self.launch_system_event_dispatcher()?;

        self.model.load(&mut self.unit, &mut self.card_cntr)?;

        if self.model.measure_elem_list.len() > 0 {
            let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, Self::TIMER_NAME, 0);
            let _ = self.card_cntr.add_bool_elems(&elem_id, 1, 1, true)?;
        }

        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let ev = match self.rx.recv() {
                Ok(ev) => ev,
                Err(_) => continue,
            };

            match ev {
                Event::Shutdown => break,
                Event::Disconnected => break,
                Event::BusReset(generation) => {
                    println!("IEEE 1394 bus is updated: {}", generation);
                }
                Event::Elem(elem_id, events) => {
                    if elem_id.name() != Self::TIMER_NAME {
                        let _ = self.model.dispatch_elem_event(
                            &mut self.unit,
                            &mut self.card_cntr,
                            &elem_id,
                            &events,
                        );
                    } else {
                        let mut elem_value = ElemValue::new();
                        if self
                            .card_cntr
                            .card
                            .read_elem_value(&elem_id, &mut elem_value)
                            .is_ok()
                        {
                            let val = elem_value.boolean()[0];
                            if val {
                                let _ = self.start_interval_timer();
                            } else {
                                self.stop_interval_timer();
                            }
                        }
                    }
                }
                Event::Timer => {
                    let _ = self
                        .model
                        .measure_elems(&mut self.unit, &mut self.card_cntr);
                }
                Event::StreamLock(locked) => {
                    let _ = self.model.dispatch_stream_lock(
                        &mut self.unit,
                        &mut self.card_cntr,
                        locked,
                    );
                }
            }
        }
        Ok(())
    }
}

impl<'a> BebobRuntime {
    const NODE_DISPATCHER_NAME: &'a str = "node event dispatcher";
    const SYSTEM_DISPATCHER_NAME: &'a str = "system event dispatcher";
    const TIMER_DISPATCHER_NAME: &'a str = "interval timer dispatcher";

    const TIMER_NAME: &'a str = "metering";
    const TIMER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

    fn launch_node_event_dispatcher(&mut self) -> Result<(), Error> {
        let name = Self::NODE_DISPATCHER_NAME.to_string();
        let mut dispatcher = Dispatcher::run(name)?;

        let tx = self.tx.clone();
        dispatcher.attach_alsa_firewire(&self.unit.0, move |_| {
            let _ = tx.send(Event::Disconnected);
        })?;

        let tx = self.tx.clone();
        dispatcher.attach_fw_node(&self.unit.1, move |_| {
            let _ = tx.send(Event::Disconnected);
        })?;

        let tx = self.tx.clone();
        self.unit.1.connect_bus_update(move |node| {
            let _ = tx.send(Event::BusReset(node.generation()));
        });

        self.dispatchers.push(dispatcher);

        Ok(())
    }

    fn launch_system_event_dispatcher(&mut self) -> Result<(), Error> {
        let name = Self::SYSTEM_DISPATCHER_NAME.to_string();
        let mut dispatcher = Dispatcher::run(name)?;

        let tx = self.tx.clone();
        dispatcher.attach_signal_handler(signal::Signal::SIGINT, move || {
            let _ = tx.send(Event::Shutdown);
            source::Continue(false)
        });

        let tx = self.tx.clone();
        dispatcher.attach_snd_card(&self.card_cntr.card, |_| {})?;
        self.card_cntr
            .card
            .connect_handle_elem_event(move |_, elem_id, events| {
                let elem_id: ElemId = elem_id.clone();
                let _ = tx.send(Event::Elem(elem_id, events));
            });

        let tx = self.tx.clone();
        self.unit.0.connect_is_locked_notify(move |unit| {
            let is_locked = unit.is_locked();
            let t = tx.clone();
            let _ = std::thread::spawn(move || {
                // The notification of stream lock is not strictly corresponding to actual
                // packet streaming. Here, wait for 500 msec to catch the actual packet
                // streaming.
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _ = t.send(Event::StreamLock(is_locked));
            });
        });

        self.dispatchers.push(dispatcher);

        Ok(())
    }

    fn start_interval_timer(&mut self) -> Result<(), Error> {
        let mut dispatcher = Dispatcher::run(Self::TIMER_DISPATCHER_NAME.to_string())?;
        let tx = self.tx.clone();
        dispatcher.attach_interval_handler(Self::TIMER_INTERVAL, move || {
            let _ = tx.send(Event::Timer);
            source::Continue(true)
        });

        self.timer = Some(dispatcher);

        Ok(())
    }

    fn stop_interval_timer(&mut self) {
        if let Some(dispatcher) = &self.timer {
            drop(dispatcher);
            self.timer = None;
        }
    }
}
