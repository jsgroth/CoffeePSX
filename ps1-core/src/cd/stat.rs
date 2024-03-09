#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{CdController, Command, CommandState, DriveState};
use std::ops::BitOr;

// Roughly 18,944 CPU cycles
const GET_ID_SECOND_CYCLES: u32 = 24;

pub struct ErrorFlags(u8);

impl ErrorFlags {
    pub const NONE: Self = Self(0);
    pub const ERROR: Self = Self(1);
    // pub const SEEK_ERROR: Self = Self(1 << 2);
    // pub const ID_ERROR: Self = Self(1 << 3);
}

impl BitOr for ErrorFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl CdController {
    pub(super) fn status_code(&self, errors: ErrorFlags) -> u8 {
        // TODO check more drive states
        let motor_on = !matches!(self.drive_state, DriveState::Stopped);

        // TODO Bit 4 (shell open)
        // TODO Bits 5-7 (Reading/Seeking/Playing)
        errors.0 | (u8::from(motor_on) << 1)
    }

    // $01: GetStat() -> INT3(stat)
    // Simply returns current status code
    pub(super) fn execute_get_stat(&mut self) -> CommandState {
        int3!(self, [self.status_code(ErrorFlags::NONE)]);
        CommandState::Idle
    }

    // $1A: GetID() -> INT3(stat), INT2/5(stat, flags, type, atip, "SCEx")
    // Essentially returns some basic disc metadata: whether the disc is licensed, whether the disc
    // is CD-ROM or an audio CD, and the disc region (if licensed)
    pub(super) fn execute_get_id(&mut self) -> CommandState {
        // TODO return error response if drive is open, spinning up, or "seek busy"

        int3!(self, [self.status_code(ErrorFlags::NONE)]);
        CommandState::GeneratingSecondResponse {
            command: Command::GetId,
            cycles_remaining: GET_ID_SECOND_CYCLES,
        }
    }

    pub(super) fn get_id_second_response(&mut self) -> CommandState {
        // TODO this is a hardcoded "no disc" response
        int5!(self, [0x08, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        CommandState::Idle
    }
}
