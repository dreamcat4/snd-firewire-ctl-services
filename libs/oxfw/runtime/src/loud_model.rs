// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2021 Takashi Sakamoto

use {super::*, protocols::loud::*};

#[derive(Default, Debug)]
pub struct LinkFwModel {
    avc: OxfwAvc,
    common_ctl: CommonCtl<OxfwAvc>,
    specific_ctl: SpecificCtl,
}

const FCP_TIMEOUT_MS: u32 = 100;

impl CtlModel<(SndUnit, FwNode)> for LinkFwModel {
    fn load(
        &mut self,
        unit: &mut (SndUnit, FwNode),
        card_cntr: &mut CardCntr,
    ) -> Result<(), Error> {
        self.avc.bind(&unit.1)?;

        self.common_ctl.load(&self.avc, card_cntr, FCP_TIMEOUT_MS)?;
        self.specific_ctl.load(card_cntr)?;

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
        } else if self
            .specific_ctl
            .read(&mut self.avc, elem_id, elem_value, FCP_TIMEOUT_MS)?
        {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn write(
        &mut self,
        unit: &mut (SndUnit, FwNode),
        elem_id: &ElemId,
        _: &ElemValue,
        new: &ElemValue,
    ) -> Result<bool, Error> {
        if self
            .common_ctl
            .write(unit, &self.avc, elem_id, new, FCP_TIMEOUT_MS)?
        {
            Ok(true)
        } else if self
            .specific_ctl
            .write(&mut self.avc, elem_id, new, FCP_TIMEOUT_MS)?
        {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl NotifyModel<(SndUnit, FwNode), bool> for LinkFwModel {
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

#[derive(Default, Debug)]
struct SpecificCtl;

fn input_source_to_str(src: &LinkFwInputSource) -> &str {
    match src {
        LinkFwInputSource::Analog => "Analog-input",
        LinkFwInputSource::Digital => "S/PDIF-input",
    }
}

const CAPTURE_SOURCE_NAME: &str = "capture-source";

impl SpecificCtl {
    const SRCS: [LinkFwInputSource; 2] = [LinkFwInputSource::Analog, LinkFwInputSource::Digital];

    fn load(&self, card_cntr: &mut CardCntr) -> Result<(), Error> {
        let labels: Vec<&str> = Self::SRCS.iter().map(|s| input_source_to_str(s)).collect();
        let elem_id = ElemId::new_by_name(ElemIfaceType::Mixer, 0, 0, CAPTURE_SOURCE_NAME, 0);
        card_cntr
            .add_enum_elems(&elem_id, 1, 1, &labels, None, true)
            .map(|_| ())
    }

    fn read(
        &self,
        avc: &mut OxfwAvc,
        elem_id: &ElemId,
        elem_value: &mut ElemValue,
        timeout_ms: u32,
    ) -> Result<bool, Error> {
        match elem_id.name().as_str() {
            CAPTURE_SOURCE_NAME => {
                let mut src = LinkFwInputSource::default();
                LinkFwProtocol::read_input_source(avc, &mut src, timeout_ms)?;
                let idx = Self::SRCS.iter().position(|src| src.eq(&src)).unwrap();
                elem_value.set_enum(&[idx as u32]);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn write(
        &self,
        avc: &mut OxfwAvc,
        elem_id: &ElemId,
        elem_value: &ElemValue,
        timeout_ms: u32,
    ) -> Result<bool, Error> {
        match elem_id.name().as_str() {
            CAPTURE_SOURCE_NAME => {
                let val = elem_value.enumerated()[0];
                let &src = Self::SRCS.iter().nth(val as usize).ok_or_else(|| {
                    let msg = format!("Invalid value for index of signal source: {}", val);
                    Error::new(FileError::Inval, &msg)
                })?;
                LinkFwProtocol::write_input_source(avc, src, timeout_ms).map(|_| true)
            }
            _ => Ok(false),
        }
    }
}
