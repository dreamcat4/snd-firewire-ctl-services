// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (c) 2021 Takashi Sakamoto

#![doc = include_str!("../README.md")]

pub mod bridgeco;

pub mod apogee;
pub mod behringer;
pub mod digidesign;
pub mod esi;
pub mod focusrite;
pub mod icon;
pub mod maudio;
pub mod presonus;
pub mod roland;
pub mod stanton;
pub mod terratec;
pub mod yamaha_terratec;

use {
    self::bridgeco::{ExtendedStreamFormatSingle, *},
    glib::{Error, FileError, IsA},
    hinawa::{
        prelude::{FwFcpExt, FwFcpExtManual, FwReqExtManual},
        FwFcp, FwNode, FwReq, FwTcode,
    },
    ta1394_avc_audio::{amdtp::*, *},
    ta1394_avc_general::{general::*, *},
    ta1394_avc_stream_format::*,
    ta1394_avc_ccm::*,
};

/// The offset for specific purposes in DM1000/DM1100/DM1500 ASICs.
const DM_APPL_OFFSET: u64 = 0xffc700000000;
const DM_APPL_METER_OFFSET: u64 = DM_APPL_OFFSET + 0x00600000;
const DM_APPL_PARAM_OFFSET: u64 = DM_APPL_OFFSET + 0x00700000;
const DM_BCO_OFFSET: u64 = 0xffffc8000000;
const DM_BCO_BOOTLOADER_INFO_OFFSET: u64 = DM_BCO_OFFSET + 0x00020000;

/// The implementation of AV/C transaction with quirks specific to BeBoB solution.
///
/// It seems a unique quirk that the status code in response frame for some AV/C commands is
/// against AV/C general specification in control operation.
#[derive(Default, Debug)]
pub struct BebobAvc(FwFcp);

impl Ta1394Avc<Error> for BebobAvc {
    fn transaction(&self, command_frame: &[u8], timeout_ms: u32) -> Result<Vec<u8>, Error> {
        let mut resp = vec![0; Self::FRAME_SIZE];
        self.0
            .avc_transaction(&command_frame, &mut resp, timeout_ms)
            .map(|len| {
                resp.truncate(len);
                resp
            })
    }

    fn control<O: AvcOp + AvcControl>(
        &self,
        addr: &AvcAddr,
        op: &mut O,
        timeout_ms: u32,
    ) -> Result<(), Ta1394AvcError<Error>> {
        let mut operands = Vec::new();
        let command_frame = AvcControl::build_operands(op, addr, &mut operands)
            .map_err(|err| Ta1394AvcError::CmdBuild(err))
            .map(|_| {
                Self::compose_command_frame(AvcCmdType::Control, addr, O::OPCODE, &operands)
            })?;
        self.transaction(&command_frame, timeout_ms)
            .map_err(|cause| Ta1394AvcError::CommunicationFailure(cause))
            .and_then(|response_frame| {
                Self::detect_response_operands(&response_frame, addr, O::OPCODE)
                    .and_then(|(rcode, operands)| {
                        let expected = match O::OPCODE {
                            InputPlugSignalFormat::OPCODE
                            | OutputPlugSignalFormat::OPCODE
                            | SignalSource::OPCODE => {
                                // NOTE: quirk.
                                rcode == AvcRespCode::Accepted
                                    || rcode == AvcRespCode::Reserved(0x00)
                            }
                            _ => rcode == AvcRespCode::Accepted,
                        };
                        if !expected {
                            Err(AvcRespParseError::UnexpectedStatus)
                        } else {
                            AvcControl::parse_operands(op, addr, &operands)
                        }
                    })
                    .map_err(|err| Ta1394AvcError::RespParse(err))
            })
    }
}

impl BebobAvc {
    pub fn bind(&self, node: &impl IsA<FwNode>) -> Result<(), Error> {
        self.0.bind(node)
    }

    pub fn control<O: AvcOp + AvcControl>(
        &self,
        addr: &AvcAddr,
        op: &mut O,
        timeout_ms: u32,
    ) -> Result<(), Error> {
        Ta1394Avc::<Error>::control(self, addr, op, timeout_ms).map_err(|err| from_avc_err(err))
    }

    pub fn status<O: AvcOp + AvcStatus>(
        &self,
        addr: &AvcAddr,
        op: &mut O,
        timeout_ms: u32,
    ) -> Result<(), Error> {
        Ta1394Avc::<Error>::status(self, addr, op, timeout_ms).map_err(|err| from_avc_err(err))
    }
}

fn from_avc_err(err: Ta1394AvcError<Error>) -> Error {
    match err {
        Ta1394AvcError::CmdBuild(cause) => Error::new(FileError::Inval, &cause.to_string()),
        Ta1394AvcError::CommunicationFailure(cause) => cause,
        Ta1394AvcError::RespParse(cause) => Error::new(FileError::Io, &cause.to_string()),
    }
}

/// The trait of frequency operation for media clock.
pub trait MediaClockFrequencyOperation {
    const FREQ_LIST: &'static [u32];

    fn read_clk_freq(avc: &BebobAvc, timeout_ms: u32) -> Result<usize, Error> {
        let plug_addr =
            BcoPlugAddr::new_for_unit(BcoPlugDirection::Output, BcoPlugAddrUnitType::Isoc, 0);
        let mut op = ExtendedStreamFormatSingle::new(&plug_addr);

        avc.status(&AvcAddr::Unit, &mut op, timeout_ms)?;

        op.stream_format
            .as_bco_compound_am824_stream()
            .and_then(|format| {
                Self::FREQ_LIST
                    .iter()
                    .position(|&r| r == format.freq)
                    .ok_or_else(|| {
                        let msg = format!("Unexpected entry for source of clock: {}", format.freq);
                        Error::new(FileError::Io, &msg)
                    })
            })
    }

    /// Change frequency of media clock. This operation can involve INTERIM AV/C response to expand
    /// response time of AV/C transaction.
    fn write_clk_freq(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<(), Error> {
        let fdf = Self::FREQ_LIST
            .iter()
            .nth(idx)
            .ok_or_else(|| {
                let msg = format!("Invalid argument for index of frequency: {}", idx);
                Error::new(FileError::Inval, &msg)
            })
            .map(|&freq| AmdtpFdf::new(AmdtpEventType::Am824, false, freq))?;

        let mut op = InputPlugSignalFormat(PlugSignalFormat {
            plug_id: 0,
            fmt: FMT_IS_AMDTP,
            fdf: fdf.into(),
        });
        avc.control(&AvcAddr::Unit, &mut op, timeout_ms)?;

        let mut op = OutputPlugSignalFormat(PlugSignalFormat {
            plug_id: 0,
            fmt: FMT_IS_AMDTP,
            fdf: fdf.into(),
        });
        avc.control(&AvcAddr::Unit, &mut op, timeout_ms)
    }
}

/// The trait of source operation for sampling clock.
pub trait SamplingClockSourceOperation {
    // NOTE: all of bebob models support "SignalAddr::Unit(SignalUnitAddr::Isoc(0x00))" named as
    // "PCR Compound Input" and "SignalAddr::Unit(SignalUnitAddr::Isoc(0x01))" named as
    // "PCR Sync Input" for source of sampling clock. They are available to be synchronized to the
    // series of syt field in incoming packets from the other unit on IEEE 1394 bus. However, the
    // most of models doesn't work with it actually even if configured, therefore useless.
    const DST: SignalAddr;
    const SRC_LIST: &'static [SignalAddr];

    fn read_clk_src(avc: &BebobAvc, timeout_ms: u32) -> Result<usize, Error> {
        let mut op = SignalSource::new(&Self::DST);

        avc.status(&AvcAddr::Unit, &mut op, timeout_ms)?;

        Self::SRC_LIST
            .iter()
            .position(|&s| s == op.src)
            .ok_or_else(|| {
                let label = "Unexpected entry for source of clock";
                Error::new(FileError::Io, &label)
            })
    }

    /// Change source of sampling clock. This operation can involve INTERIM AV/C response to expand
    /// response time of AV/C transaction.
    fn write_clk_src(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<(), Error> {
        let src = Self::SRC_LIST.iter().nth(idx).map(|s| *s).ok_or_else(|| {
            let label = "Invalid value for source of clock";
            Error::new(FileError::Inval, &label)
        })?;

        let mut op = SignalSource::new(&Self::DST);
        op.src = src;

        avc.control(&AvcAddr::Unit, &mut op, timeout_ms)
    }
}

/// The trait of level operation for audio function blocks by AV/C transaction.
pub trait AvcLevelOperation {
    const ENTRIES: &'static [(u8, AudioCh)];

    const LEVEL_MIN: i16 = FeatureCtl::NEG_INFINITY;
    const LEVEL_MAX: i16 = 0;
    const LEVEL_STEP: i16 = 0x100;

    fn read_level(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<i16, Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::Volume(vec![-1]),
        );
        avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;

        if let FeatureCtl::Volume(data) = op.ctl {
            Ok(data[0])
        } else {
            unreachable!();
        }
    }

    fn write_level(avc: &BebobAvc, idx: usize, vol: i16, timeout_ms: u32) -> Result<(), Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::Volume(vec![vol]),
        );
        avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
    }
}

/// The trait of LR balance operation for audio function blocks.
pub trait AvcLrBalanceOperation: AvcLevelOperation {
    const BALANCE_MIN: i16 = FeatureCtl::NEG_INFINITY;
    const BALANCE_MAX: i16 = FeatureCtl::INFINITY;
    const BALANCE_STEP: i16 = 0x80;

    fn read_lr_balance(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<i16, Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::LrBalance(-1),
        );
        avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;

        if let FeatureCtl::LrBalance(balance) = op.ctl {
            Ok(balance)
        } else {
            unreachable!();
        }
    }

    fn write_lr_balance(
        avc: &BebobAvc,
        idx: usize,
        balance: i16,
        timeout_ms: u32,
    ) -> Result<(), Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::LrBalance(balance),
        );
        avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
    }
}

/// The trait of mute operation for audio function blocks.
pub trait AvcMuteOperation: AvcLevelOperation {
    fn read_mute(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<bool, Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::Mute(vec![false]),
        );
        avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;

        if let FeatureCtl::Mute(data) = op.ctl {
            Ok(data[0])
        } else {
            unreachable!();
        }
    }

    fn write_mute(avc: &BebobAvc, idx: usize, mute: bool, timeout_ms: u32) -> Result<(), Error> {
        let &(func_block_id, audio_ch) = Self::ENTRIES.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of function block list: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioFeature::new(
            func_block_id,
            CtlAttr::Current,
            audio_ch,
            FeatureCtl::Mute(vec![mute]),
        );
        avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
    }
}

/// The trait of select operation for audio function block.
pub trait AvcSelectorOperation {
    const FUNC_BLOCK_ID_LIST: &'static [u8];
    const INPUT_PLUG_ID_LIST: &'static [u8];

    fn read_selector(avc: &BebobAvc, idx: usize, timeout_ms: u32) -> Result<usize, Error> {
        let &func_block_id = Self::FUNC_BLOCK_ID_LIST.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of selector: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let mut op = AudioSelector::new(func_block_id, CtlAttr::Current, 0xff);
        avc.status(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)?;

        Self::INPUT_PLUG_ID_LIST
            .iter()
            .position(|&input_plug_id| input_plug_id == op.input_plug_id)
            .ok_or_else(|| {
                let msg = format!(
                    "Unexpected index of input plug number: {}",
                    op.input_plug_id
                );
                Error::new(FileError::Io, &msg)
            })
    }

    fn write_selector(
        avc: &BebobAvc,
        idx: usize,
        val: usize,
        timeout_ms: u32,
    ) -> Result<(), Error> {
        let &func_block_id = Self::FUNC_BLOCK_ID_LIST.iter().nth(idx).ok_or_else(|| {
            let msg = format!("Invalid index of selector: {}", idx);
            Error::new(FileError::Inval, &msg)
        })?;

        let input_plug_id = Self::INPUT_PLUG_ID_LIST
            .iter()
            .nth(val)
            .ok_or_else(|| {
                let msg = format!("Invalid index of input plug number: {}", val);
                Error::new(FileError::Inval, &msg)
            })
            .map(|input_plug_id| *input_plug_id)?;

        let mut op = AudioSelector::new(func_block_id, CtlAttr::Current, input_plug_id);
        avc.control(&AUDIO_SUBUNIT_0_ADDR, &mut op, timeout_ms)
    }
}
