// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (c) 2021 Takashi Sakamoto

//! Protocol about robot guitar.
//!
//! The module includes protocol about robot guitar defined by Echo Audio Digital Corporation for
//! Fireworks board module.

use super::*;

const CATEGORY_ROBOT_GUITAR: u32 = 10;

const CMD_SET_CHARGE_STATE: u32 = 7;
const CMD_GET_CHARGE_STATE: u32 = 8;

/// State of charging for Robot Guitar.
#[derive(Debug)]
pub struct GuitarChargeState {
    pub manual_charge: bool,
    pub auto_charge: bool,
    pub suspend_to_charge: u32,
}

/// Protocol about robot guitar for Fireworks board module.
pub trait RobotGuitarProtocol: EfwProtocolExtManual {
    fn get_charge_state(&mut self, timeout_ms: u32) -> Result<GuitarChargeState, Error> {
        let mut params = vec![0; 3];
        self.transaction(
            CATEGORY_ROBOT_GUITAR,
            CMD_GET_CHARGE_STATE,
            &[],
            &mut params,
            timeout_ms,
        )
        .map(|_| GuitarChargeState {
            manual_charge: params[0] > 0,
            auto_charge: params[1] > 0,
            suspend_to_charge: params[2],
        })
    }

    fn set_charge_state(
        &mut self,
        state: &GuitarChargeState,
        timeout_ms: u32,
    ) -> Result<(), Error> {
        let args = [
            state.manual_charge as u32,
            state.auto_charge as u32,
            state.suspend_to_charge,
        ];
        let mut params = vec![0; 3];
        self.transaction(
            CATEGORY_ROBOT_GUITAR,
            CMD_SET_CHARGE_STATE,
            &args,
            &mut params,
            timeout_ms,
        )
    }
}

impl<O: EfwProtocolExtManual> RobotGuitarProtocol for O {}
