#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{CdController, Command, CommandState, DriveState};
use cdrom::cue::TrackMode;
use std::ops::BitOr;

pub const WRONG_NUM_PARAMETERS: u8 = 0x20;
pub const INVALID_COMMAND: u8 = 0x40;

// Roughly 18,944 CPU cycles
const GET_ID_SECOND_CYCLES: u32 = 24;

// Roughly a second
const READ_TOC_SECOND_CYCLES: u32 = 44_100;

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
        let motor_on =
            !matches!(self.drive_state, DriveState::Stopped | DriveState::SpinningUp { .. });
        let seeking = matches!(self.drive_state, DriveState::Seeking { .. });
        let reading = matches!(self.drive_state, DriveState::Reading { .. });

        // TODO Bit 4 (shell open)
        // TODO Bits 7 (Playing)
        errors.0 | (u8::from(motor_on) << 1) | (u8::from(reading) << 5) | (u8::from(seeking) << 6)
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
        match &self.disc {
            // TODO don't hardcode region
            Some(disc) => {
                let status = self.status_code(ErrorFlags::NONE);
                let mode_byte = match disc.cue().track(1).mode {
                    TrackMode::Mode2 => 0x20,
                    TrackMode::Mode1 | TrackMode::Audio => 0x00,
                };

                int2!(self, [status, 0x00, mode_byte, 0x00, b'S', b'C', b'E', b'A']);
            }
            None => {
                // "No disc" response
                int5!(self, [0x08, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
            }
        }

        CommandState::Idle
    }

    // $1E: ReadTOC() -> INT3(stat), INT2(stat)
    // Forces the drive to re-read the TOC
    pub(super) fn execute_read_toc(&mut self) -> CommandState {
        int3!(self, [self.status_code(ErrorFlags::NONE)]);

        CommandState::GeneratingSecondResponse {
            command: Command::ReadToc,
            cycles_remaining: READ_TOC_SECOND_CYCLES,
        }
    }

    pub(super) fn read_toc_second_response(&mut self) -> CommandState {
        int2!(self, [self.status_code(ErrorFlags::NONE)]);
        CommandState::Idle
    }
}
