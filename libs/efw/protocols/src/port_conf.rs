// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (c) 2021 Takashi Sakamoto

//! Protocol about port configuration.
//!
//! The module includes protocol about port configuration defined by Echo Audio Digital Corporation
//! for Fireworks board module.

use super::*;

const CATEGORY_PORT_CONF: u32 = 9;

const CMD_SET_MIRROR: u32 = 0;
const CMD_GET_MIRROR: u32 = 1;
const CMD_SET_DIG_MODE: u32 = 2;
const CMD_GET_DIG_MODE: u32 = 3;
const CMD_SET_PHANTOM: u32 = 4;
const CMD_GET_PHANTOM: u32 = 5;
const CMD_SET_STREAM_MAP: u32 = 6;
const CMD_GET_STREAM_MAP: u32 = 7;

/// Type of audio signal for dignal input and output.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum DigitalMode {
    /// Coaxial interface for S/PDIF signal.
    SpdifCoax,
    /// XLR interface for AES/EBU signal.
    AesebuXlr,
    /// Optical interface for S/PDIF signal.
    SpdifOpt,
    /// Optical interface for ADAT signal.
    AdatOpt,
    Unknown(u32),
}

impl From<u32> for DigitalMode {
    fn from(val: u32) -> Self {
        match val {
            0 => DigitalMode::SpdifCoax,
            1 => DigitalMode::AesebuXlr,
            2 => DigitalMode::SpdifOpt,
            3 => DigitalMode::AdatOpt,
            _ => DigitalMode::Unknown(val),
        }
    }
}

impl From<DigitalMode> for u32 {
    fn from(mode: DigitalMode) -> Self {
        match mode {
            DigitalMode::SpdifCoax => 0,
            DigitalMode::AesebuXlr => 1,
            DigitalMode::SpdifOpt => 2,
            DigitalMode::AdatOpt => 3,
            DigitalMode::Unknown(val) => val,
        }
    }
}

const MAP_SIZE: usize = 70;
const MAP_ENTRY_COUNT: usize = 32;
const MAP_ENTRY_DISABLE: u32 = 0xffffffff;

/// Protocol about port configuration for Fireworks board module.
pub trait PortConfProtocol: EfwProtocolExtManual {
    fn set_control_room_source(&mut self, pair: usize, timeout_ms: u32) -> Result<(), Error> {
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_SET_MIRROR,
            &[(pair * 2) as u32],
            &mut Vec::new(),
            timeout_ms,
        )
    }

    fn get_control_room_source(&mut self, timeout_ms: u32) -> Result<usize, Error> {
        let mut params = vec![0];
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_GET_MIRROR,
            &[],
            &mut params,
            timeout_ms,
        )
        .map(|_| (params[0] / 2) as usize)
    }

    fn set_digital_mode(&mut self, mode: DigitalMode, timeout_ms: u32) -> Result<(), Error> {
        let args = [u32::from(mode)];
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_SET_DIG_MODE,
            &args,
            &mut Vec::new(),
            timeout_ms,
        )
    }

    fn get_digital_mode(&mut self, timeout_ms: u32) -> Result<DigitalMode, Error> {
        let mut params = vec![0];
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_GET_DIG_MODE,
            &[],
            &mut params,
            timeout_ms,
        )
        .map(|_| DigitalMode::from(params[0]))
    }

    fn set_phantom_powering(&mut self, state: bool, timeout_ms: u32) -> Result<(), Error> {
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_SET_PHANTOM,
            &[state as u32],
            &mut Vec::new(),
            timeout_ms,
        )
    }

    fn get_phantom_powering(&mut self, timeout_ms: u32) -> Result<bool, Error> {
        let mut params = vec![0];
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_GET_PHANTOM,
            &[],
            &mut params,
            timeout_ms,
        )
        .map(|_| params[0] > 0)
    }

    fn set_stream_map(
        &mut self,
        rate: u32,
        phys_output_pair_count: usize,
        phys_input_pair_count: usize,
        rx_stream_map: &[Option<usize>],
        tx_stream_map: &[Option<usize>],
        timeout_ms: u32,
    ) -> Result<(), Error> {
        let mut args = [0; MAP_SIZE];
        build_stream_map(
            &mut args,
            rate,
            phys_output_pair_count,
            phys_input_pair_count,
            rx_stream_map,
            tx_stream_map,
        );
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_SET_STREAM_MAP,
            &args,
            &mut Vec::new(),
            timeout_ms,
        )
    }

    fn get_stream_map(
        &mut self,
        rate: u32,
        phys_output_pair_count: usize,
        phys_input_pair_count: usize,
        rx_stream_map: &mut [Option<usize>],
        tx_stream_map: &mut [Option<usize>],
        timeout_ms: u32,
    ) -> Result<(), Error> {
        let args = [rate];
        let mut params = vec![0; MAP_SIZE];
        self.transaction(
            CATEGORY_PORT_CONF,
            CMD_GET_STREAM_MAP,
            &args,
            &mut params,
            timeout_ms,
        )
        .map(|_| {
            let phys_output_count = 2 * phys_output_pair_count as u32;
            let phys_input_count = 2 * phys_input_pair_count as u32;

            rx_stream_map
                .iter_mut()
                .zip(&params[4..])
                .take(params[2] as usize)
                .for_each(|(entry, &val)| {
                    *entry = if val < phys_output_count {
                        Some((val / 2) as usize)
                    } else {
                        None
                    };
                });
            tx_stream_map
                .iter_mut()
                .zip(&params[38..])
                .take(params[36] as usize)
                .for_each(|(entry, &val)| {
                    *entry = if val < phys_input_count {
                        Some((val / 2) as usize)
                    } else {
                        None
                    };
                });
        })
    }
}

fn build_stream_map(
    quads: &mut [u32],
    rate: u32,
    phys_output_pair_count: usize,
    phys_input_pair_count: usize,
    rx_stream_map: &[Option<usize>],
    tx_stream_map: &[Option<usize>],
) {
    assert_eq!(quads.len(), MAP_SIZE);
    assert!(rx_stream_map.len() < MAP_ENTRY_COUNT);
    assert!(tx_stream_map.len() < MAP_ENTRY_COUNT);

    quads[0] = rate;
    // NOTE: This field is filled with clock source bits, however it's not used actually.
    quads[1] = 0;
    quads[2] = rx_stream_map.len() as u32;
    quads[3] = (phys_output_pair_count * 2) as u32;
    quads[4..(4 + MAP_ENTRY_COUNT)]
        .iter_mut()
        .enumerate()
        .for_each(|(i, entry)| {
            *entry = if i < rx_stream_map.len() {
                if let Some(entry) = rx_stream_map[i] {
                    entry as u32
                } else {
                    MAP_ENTRY_DISABLE
                }
            } else {
                MAP_ENTRY_DISABLE
            };
        });
    quads[36] = tx_stream_map.len() as u32;
    quads[37] = (phys_input_pair_count * 2) as u32;
    quads[38..(38 + MAP_ENTRY_COUNT)]
        .iter_mut()
        .enumerate()
        .for_each(|(i, entry)| {
            *entry = if i < tx_stream_map.len() {
                if let Some(entry) = tx_stream_map[i] {
                    entry as u32
                } else {
                    MAP_ENTRY_DISABLE
                }
            } else {
                MAP_ENTRY_DISABLE
            };
        });
}

impl<O: EfwProtocolExtManual> PortConfProtocol for O {}
