// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2020 Takashi Sakamoto

use {super::*, protocols::griffin::*};

#[derive(Default, Debug)]
pub struct GriffinModel {
    avc: OxfwAvc,
    common_ctl: CommonCtl<OxfwAvc>,
    voluntary: bool,
}

const FCP_TIMEOUT_MS: u32 = 100;

const VOL_NAME: &str = "PCM Playback Volume";
const MUTE_NAME: &str = "PCM Playback Switch";

impl CtlModel<(SndUnit, FwNode)> for GriffinModel {
    fn load(
        &mut self,
        unit: &mut (SndUnit, FwNode),
        card_cntr: &mut CardCntr,
    ) -> Result<(), Error> {
        self.avc.bind(&unit.1)?;

        self.common_ctl.load(&self.avc, card_cntr, FCP_TIMEOUT_MS)?;

        // NOTE: I have a plan to remove control functionality from ALSA oxfw driver for future.
        let elem_id_list = card_cntr.card.elem_id_list()?;
        self.voluntary = elem_id_list
            .iter()
            .find(|elem_id| elem_id.name().as_str() == VOL_NAME)
            .is_none();
        if self.voluntary {
            let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, VOL_NAME, 0);
            let _ = card_cntr.add_int_elems(
                &elem_id,
                1,
                FirewaveProtocol::VOLUME_MIN as i32,
                FirewaveProtocol::VOLUME_MAX as i32,
                FirewaveProtocol::VOLUME_STEP as i32,
                FirewaveProtocol::PLAYBACK_COUNT,
                None,
                true,
            )?;

            let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, MUTE_NAME, 0);
            let _ = card_cntr.add_bool_elems(&elem_id, 1, 1, true)?;
        }

        Ok(())
    }

    fn read(
        &mut self,
        _: &mut (SndUnit, FwNode),
        elem_id: &ElemId,
        elem_value: &mut ElemValue,
    ) -> Result<bool, Error> {
        if self
            .common_ctl
            .read(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)?
        {
            Ok(true)
        } else if self.voluntary {
            match elem_id.name().as_str() {
                VOL_NAME => {
                    ElemValueAccessor::<i32>::set_vals(elem_value, 6, |idx| {
                        let mut vol = 0;
                        FirewaveProtocol::read_volume(&mut self.avc, idx, &mut vol, FCP_TIMEOUT_MS)
                            .map(|_| vol as i32)
                    })?;
                    Ok(true)
                }
                MUTE_NAME => {
                    ElemValueAccessor::<bool>::set_val(elem_value, || {
                        let mut mute = false;
                        FirewaveProtocol::read_mute(&mut self.avc, &mut mute, FCP_TIMEOUT_MS)
                            .map(|_| mute)
                    })?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }

    fn write(
        &mut self,
        unit: &mut (SndUnit, FwNode),
        elem_id: &ElemId,
        old: &ElemValue,
        new: &ElemValue,
    ) -> Result<bool, Error> {
        if self
            .common_ctl
            .write(unit, &self.avc, elem_id, new, FCP_TIMEOUT_MS)?
        {
            Ok(true)
        } else if self.voluntary {
            match elem_id.name().as_str() {
                VOL_NAME => {
                    ElemValueAccessor::<i32>::get_vals(new, old, 6, |idx, val| {
                        FirewaveProtocol::write_volume(
                            &mut self.avc,
                            idx,
                            val as i16,
                            FCP_TIMEOUT_MS,
                        )
                    })?;
                    Ok(true)
                }
                MUTE_NAME => {
                    ElemValueAccessor::<bool>::get_val(new, |val| {
                        FirewaveProtocol::write_mute(&mut self.avc, val, FCP_TIMEOUT_MS)
                    })?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        } else {
            Ok(false)
        }
    }
}

impl NotifyModel<(SndUnit, FwNode), bool> for GriffinModel {
    fn get_notified_elem_list(&mut self, elem_id_list: &mut Vec<ElemId>) {
        elem_id_list.extend_from_slice(&self.common_ctl.notified_elem_list);
    }

    fn parse_notification(&mut self, _: &mut (SndUnit, FwNode), _: &bool) -> Result<(), Error> {
        Ok(())
    }

    fn read_notified_elem(
        &mut self,
        _: &(SndUnit, FwNode),
        elem_id: &ElemId,
        elem_value: &mut ElemValue,
    ) -> Result<bool, Error> {
        self.common_ctl
            .read(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)
    }
}
