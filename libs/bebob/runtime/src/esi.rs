// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2020 Takashi Sakamoto

use glib::Error;

use hinawa::{SndUnit, SndUnitExt, FwFcpExt};
use alsactl::{ElemId, ElemIfaceType, ElemValue};

use alsa_ctl_tlv_codec::items::{DbInterval, CTL_VALUE_MUTE};

use core::card_cntr::*;
use core::elem_value_accessor::ElemValueAccessor;

use ta1394::{*, audio::*};

use bebob_protocols::{*, esi::*};

use super::common_ctls::*;

const FCP_TIMEOUT_MS: u32 = 100;

#[derive(Default)]
pub struct Quatafire610Model {
    avc: BebobAvc,
    clk_ctl: ClkCtl,
    input_ctl: InputCtl,
    output_ctl: OutputCtl,
}

#[derive(Default)]
struct ClkCtl(Vec<ElemId>);

impl MediaClkFreqCtlOperation<Quatafire610ClkProtocol> for ClkCtl {}

impl SamplingClkSrcCtlOperation<Quatafire610ClkProtocol> for ClkCtl {
    const SRC_LABELS: &'static [&'static str] = &[
        "Internal",
    ];
}

impl CtlModel<SndUnit> for Quatafire610Model {
    fn load(&mut self, unit: &mut SndUnit, card_cntr: &mut CardCntr) -> Result<(), Error> {
        self.avc.as_ref().bind(&unit.get_node())?;

        self.clk_ctl.load_freq(card_cntr)
            .map(|mut elem_id_list| self.clk_ctl.0.append(&mut elem_id_list))?;

        self.clk_ctl.load_src(card_cntr)
            .map(|mut elem_id_list| self.clk_ctl.0.append(&mut elem_id_list))?;

        self.input_ctl.load(card_cntr)?;
        self.output_ctl.load(card_cntr)?;
        Ok(())
    }

    fn read(&mut self, _: &mut SndUnit, elem_id: &ElemId, elem_value: &mut ElemValue)
        -> Result<bool, Error>
    {
        if self.clk_ctl.read_freq(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else if self.clk_ctl.read_src(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else if self.input_ctl.read(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else if self.output_ctl.read(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn write(&mut self, unit: &mut SndUnit, elem_id: &ElemId, old: &ElemValue, new: &ElemValue)
        -> Result<bool, Error>
    {
        if self.clk_ctl.write_freq(unit, &self.avc, elem_id, old, new, FCP_TIMEOUT_MS * 3)? {
            Ok(true)
        } else if self.clk_ctl.write_src(unit, &self.avc, elem_id, old, new, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else if self.input_ctl.write(&self.avc, elem_id, old, new, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else if self.output_ctl.write(&self.avc, elem_id, old, new, FCP_TIMEOUT_MS)? {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl NotifyModel<SndUnit, bool> for Quatafire610Model {
    fn get_notified_elem_list(&mut self, elem_id_list: &mut Vec<ElemId>) {
        elem_id_list.extend_from_slice(&self.clk_ctl.0);
    }

    fn parse_notification(&mut self, _: &mut SndUnit, _: &bool) -> Result<(), Error> {
        Ok(())
    }

    fn read_notified_elem(&mut self, _: &SndUnit, elem_id: &ElemId, elem_value: &mut ElemValue)
        -> Result<bool, Error>
    {
        self.clk_ctl.read_freq(&self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)
    }
}

#[derive(Default, Debug)]
struct InputCtl;

const INPUT_GAIN_NAME: &str = "input-gain";
const INPUT_BALANCE_NAME: &str = "input-pan";

const GAIN_MIN: i32 = FeatureCtl::NEG_INFINITY as i32;
const GAIN_MAX: i32 = 0;
const GAIN_STEP: i32 = 1;
const GAIN_TLV: DbInterval = DbInterval{min: -12800, max: 0, linear: false, mute_avail: false};

const BALANCE_MIN: i32 = FeatureCtl::NEG_INFINITY as i32;
const BALANCE_MAX: i32 = FeatureCtl::INFINITY as i32;
const BALANCE_STEP: i32 = 1;

const INPUT_LABELS: [&str;6] = [
    "mic-input-1", "mic-input-2",
    "line-input-1", "line-input-2",
    "S/PDIF-input-1", "S/PDIF-input-2",
];

const INPUT_FB_IDS: [u8;3] = [1, 2, 3];

impl InputCtl {
    fn load(&mut self, card_cntr: &mut CardCntr) -> Result<(), Error> {
        let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, INPUT_GAIN_NAME, 0);
        let _ = card_cntr.add_int_elems(&elem_id, 1, GAIN_MIN, GAIN_MAX, GAIN_STEP, INPUT_LABELS.len(),
                                        Some(&Into::<Vec<u32>>::into(GAIN_TLV)), true)?;

        let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, INPUT_BALANCE_NAME, 0);
        let _ = card_cntr.add_int_elems(&elem_id, 1, BALANCE_MIN, BALANCE_MAX, BALANCE_STEP, 2,
                                        None, true)?;

        Ok(())
    }

    fn read(&mut self, avc: &BebobAvc, elem_id: &ElemId, elem_value: &mut ElemValue,
            timeout_ms: u32)
        -> Result<bool, Error>
    {
        match elem_id.get_name().as_str() {
            INPUT_GAIN_NAME => {
                ElemValueAccessor::<i32>::set_vals(elem_value, INPUT_LABELS.len(), |idx| {
                    let func_blk_id = INPUT_FB_IDS[idx / 2];
                    let audio_ch_num = AudioCh::Each((idx % 2) as u8);
                    let mut op = AudioFeature::new(func_blk_id, CtlAttr::Current, audio_ch_num,
                                                   FeatureCtl::Volume(vec![-1]));
                    avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;
                    if let FeatureCtl::Volume(data) = op.ctl {
                        let val = if data[0] == FeatureCtl::NEG_INFINITY { CTL_VALUE_MUTE } else { data[0] as i32 };
                        Ok(val)
                    } else {
                        unreachable!();
                    }
                })
                .map(|_| true)
            }
            INPUT_BALANCE_NAME => {
                ElemValueAccessor::<i32>::set_vals(elem_value, 2, |idx| {
                    let func_blk_id = INPUT_FB_IDS[idx / 2];
                    let audio_ch_num = AudioCh::Each((idx % 2) as u8);
                    let mut op = AudioFeature::new(func_blk_id, CtlAttr::Current, audio_ch_num,
                                                   FeatureCtl::LrBalance(-1));
                    avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;
                    if let FeatureCtl::LrBalance(val) = op.ctl {
                        Ok(val as i32)
                    } else {
                        unreachable!();
                    }
                })
                .map(|_| true)
            }
            _ => Ok(false),
        }
    }

    fn write(&mut self, avc: &BebobAvc, elem_id: &ElemId, old: &ElemValue, new: &ElemValue,
             timeout_ms: u32)
        -> Result<bool, Error>
    {
        match elem_id.get_name().as_str() {
            INPUT_GAIN_NAME => {
                ElemValueAccessor::<i32>::get_vals(new, old, INPUT_LABELS.len(), |idx, val| {
                    let func_blk_id = INPUT_FB_IDS[idx / 2];
                    let audio_ch_num = AudioCh::Each((idx % 2) as u8);
                    let v = if val == CTL_VALUE_MUTE { FeatureCtl::NEG_INFINITY } else { val as i16 };
                    let mut op = AudioFeature::new(func_blk_id, CtlAttr::Current, audio_ch_num,
                                                   FeatureCtl::Volume(vec![v]));
                    avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
                })
                .map(|_| true)
            }
            INPUT_BALANCE_NAME => {
                ElemValueAccessor::<i32>::get_vals(new, old, 2, |idx, val| {
                    let func_blk_id = INPUT_FB_IDS[idx / 2];
                    let audio_ch_num = AudioCh::Each((idx % 2) as u8);
                    let mut op = AudioFeature::new(func_blk_id, CtlAttr::Current, audio_ch_num,
                                                   FeatureCtl::LrBalance(val as i16));
                    avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
                })
                .map(|_| true)
            }
            _ => Ok(false),
        }
    }
}

#[derive(Default)]
struct OutputCtl;

const OUTPUT_VOL_NAME: &str = "output-volume";

const VOL_MIN: i32 = FeatureCtl::NEG_INFINITY as i32;
const VOL_MAX: i32 = 0;
const VOL_STEP: i32 = 1;
const VOL_TLV: DbInterval = DbInterval{min: -12800, max: 0, linear: false, mute_avail: false};

const OUTPUT_COUNT: usize = 8;
const OUTPUT_FB_ID: u8 = 4;

impl OutputCtl {
    fn load(&mut self, card_cntr: &mut CardCntr) -> Result<(), Error> {
        let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, OUTPUT_VOL_NAME, 0);
        let _ = card_cntr.add_int_elems(&elem_id, 1, VOL_MIN, VOL_MAX, VOL_STEP, INPUT_LABELS.len(),
                                        Some(&Into::<Vec<u32>>::into(VOL_TLV)), true)?;

        Ok(())
    }

    fn read(&mut self, avc: &BebobAvc, elem_id: &ElemId, elem_value: &mut ElemValue,
            timeout_ms: u32)
        -> Result<bool, Error>
    {
        match elem_id.get_name().as_str() {
            OUTPUT_VOL_NAME => {
                ElemValueAccessor::<i32>::set_vals(elem_value, OUTPUT_COUNT, |idx| {
                    let mut op = AudioFeature::new(OUTPUT_FB_ID, CtlAttr::Current, AudioCh::Each(idx as u8),
                                                   FeatureCtl::Volume(vec![-1]));
                    avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;
                    if let FeatureCtl::Volume(data) = op.ctl {
                        let val = if data[0] == FeatureCtl::NEG_INFINITY { CTL_VALUE_MUTE } else { data[0] as i32 };
                        Ok(val)
                    } else {
                        unreachable!();
                    }
                })
                .map(|_| true)
            }
            _ => Ok(false),
        }
    }

    fn write(&mut self, avc: &BebobAvc, elem_id: &ElemId, old: &ElemValue, new: &ElemValue,
             timeout_ms: u32)
        -> Result<bool, Error>
    {
        match elem_id.get_name().as_str() {
            OUTPUT_VOL_NAME => {
                ElemValueAccessor::<i32>::get_vals(new, old, OUTPUT_COUNT, |idx, val| {
                    let v = if val == CTL_VALUE_MUTE { FeatureCtl::NEG_INFINITY } else { val as i16 };
                    let mut op = AudioFeature::new(OUTPUT_FB_ID, CtlAttr::Current, AudioCh::Each(idx as u8),
                                                   FeatureCtl::Volume(vec![v]));
                    avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
                })
                .map(|_| true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use alsactl::CardError;

    #[test]
    fn test_clk_ctl_definition() {
        let mut card_cntr = CardCntr::new();
        let mut ctl = ClkCtl::default();

        let error = ctl.load_freq(&mut card_cntr).unwrap_err();
        assert_eq!(error.kind::<CardError>(), Some(CardError::Failed));
    }
}
