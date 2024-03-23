//! CD-ROM status commands

use crate::cd;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{CdController, Command, CommandState, DriveState};
use cdrom::cdtime::CdTime;
use cdrom::cue::TrackMode;
use std::ops::BitOr;

pub const INVALID_PARAMETER: u8 = 0x10;
pub const WRONG_NUM_PARAMETERS: u8 = 0x20;
pub const INVALID_COMMAND: u8 = 0x40;

// Roughly 18,944 CPU cycles
pub const GET_ID_SECOND_CYCLES: u32 = 24;

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
        let motor_on = !self.drive_state.is_stopped_or_spinning_up();
        let seeking = matches!(self.drive_state, DriveState::Seeking { .. });
        let reading = matches!(
            self.drive_state,
            DriveState::PreparingToRead { .. } | DriveState::Reading { .. }
        );
        let playing = matches!(
            self.drive_state,
            DriveState::PreparingToPlay { .. } | DriveState::Playing { .. }
        );

        // TODO Bit 4 (shell open)
        errors.0
            | (u8::from(motor_on) << 1)
            | (u8::from(reading) << 5)
            | (u8::from(seeking) << 6)
            | (u8::from(playing) << 7)
    }

    // $01: GetStat() -> INT3(stat)
    // Simply returns current status code
    pub(super) fn execute_get_stat(&mut self) -> CommandState {
        int3!(self, [stat!(self)]);
        CommandState::Idle
    }

    // $1A: GetID() -> INT3(stat), INT2/5(stat, flags, type, atip, "SCEx")
    // Essentially returns some basic disc metadata: whether the disc is licensed, whether the disc
    // is CD-ROM or an audio CD, and the disc region (if licensed)
    pub(super) fn execute_get_id(&mut self) -> CommandState {
        // TODO return error response if drive is open, spinning up, or "seek busy"

        int3!(self, [stat!(self)]);
        CommandState::GeneratingSecondResponse {
            command: Command::GetId,
            cycles_remaining: GET_ID_SECOND_CYCLES,
        }
    }

    pub(super) fn get_id_second_response(&mut self) -> CommandState {
        match &self.disc {
            // TODO don't hardcode region
            Some(disc) => {
                let status = stat!(self);
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
        int3!(self, [stat!(self)]);

        CommandState::GeneratingSecondResponse {
            command: Command::ReadToc,
            cycles_remaining: READ_TOC_SECOND_CYCLES,
        }
    }

    pub(super) fn read_toc_second_response(&mut self) -> CommandState {
        int2!(self, [stat!(self)]);
        CommandState::Idle
    }

    // $13: GetTN() -> INT3(stat, first, last)
    // Returns the first and last track numbers on the disc
    pub(super) fn execute_get_tn(&mut self) -> CommandState {
        let (first, last) = match &self.disc {
            Some(disc) => (1_u8, cd::binary_to_bcd(disc.cue().last_track().number)),
            None => {
                // TODO this should be an INT5 error?
                (1, 1)
            }
        };

        int3!(self, [stat!(self), first, last]);

        CommandState::Idle
    }

    // $14: GetTD(track) -> INT3(stat, mm, ss)
    // Return the start time for the specified track
    pub(super) fn execute_get_td(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 1 {
            int5!(self, [stat!(self, ERROR), WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        let Some(disc) = &self.disc else {
            // TODO this should be an INT5 error?
            todo!("GetTD commmand with no disc in the drive");
        };

        let last_track = disc.cue().last_track().number;

        let mut track = cd::bcd_to_binary(self.parameter_fifo.pop());
        if track == 0 {
            // 0 means last track
            track = last_track;
        }

        if track > last_track {
            int5!(self, [stat!(self, ERROR), INVALID_PARAMETER]);
            return CommandState::Idle;
        }

        let start_time = disc.cue().track(track).effective_start_time();
        let minutes = cd::binary_to_bcd(start_time.minutes);
        let seconds = cd::binary_to_bcd(start_time.seconds);
        int3!(self, [stat!(self), minutes, seconds]);

        CommandState::Idle
    }

    // $11: GetLocP() -> INT3(track, index, mm, ss, sect, amm, ass, asect)
    // Returns position data from Subchannel Q
    pub(super) fn execute_get_loc_p(&mut self) -> CommandState {
        let Some(disc) = &self.disc else {
            todo!("GetLocP executed with no disc in the drive");
        };

        // TODO better handle if this is executed while seeking
        let absolute_time = self.drive_state.current_time();
        let track = disc.cue().find_track_by_time(absolute_time);

        let (track_number, index, relative_time) =
            track.map_or((0xAA, 0x00, CdTime::ZERO), |track| {
                let track_number = cd::binary_to_bcd(track.number);
                let index = u8::from(absolute_time >= track.effective_start_time());
                let relative_time = absolute_time.saturating_sub(track.effective_start_time());

                (track_number, index, relative_time)
            });

        int3!(
            self,
            [
                track_number,
                index,
                cd::binary_to_bcd(relative_time.minutes),
                cd::binary_to_bcd(relative_time.seconds),
                cd::binary_to_bcd(relative_time.frames),
                cd::binary_to_bcd(absolute_time.minutes),
                cd::binary_to_bcd(absolute_time.seconds),
                cd::binary_to_bcd(absolute_time.frames),
            ]
        );

        CommandState::Idle
    }
}
