//! CD-ROM seek commands

use crate::cd;
use crate::cd::audio::PlayState;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::read::ReadState;
use crate::cd::{status, CdController, CommandState, DriveState, SeekNextState, SpinUpNextState};
use cdrom::cdtime::CdTime;
use std::cmp;

// The BIOS does not like if a seek finishes too quickly
pub const MIN_SEEK_CYCLES: u32 = 24;

impl CdController {
    // $02: SetLoc(amm, ass, asect) -> INT3(stat)
    // Sets seek location to the specified absolute time
    pub(super) fn execute_set_loc(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 3 {
            int5!(self, [stat!(self, ERROR), status::WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        let minutes = cd::bcd_to_binary(self.parameter_fifo.pop());
        let seconds = cd::bcd_to_binary(self.parameter_fifo.pop());
        let frames = cd::bcd_to_binary(self.parameter_fifo.pop());

        match CdTime::new_checked(minutes, seconds, frames) {
            Some(cd_time) => {
                self.seek_location = Some(cd_time);
                int3!(self, [stat!(self)]);

                log::debug!("Set seek location to {cd_time}");
            }
            None => {
                int5!(self, [stat!(self, ERROR), status::INVALID_COMMAND]);

                log::warn!("Invalid seek location: {minutes:02}:{seconds:02}:{frames:02}");
            }
        }

        CommandState::Idle
    }

    // $15: SeekL() -> INT3(stat), INT2(stat)
    // $16: SeekP() -> INT3(stat), INT2(stat)
    // Seeks to the location specified by the most recent SetLoc command.
    // SeekL seeks in data mode (uses data sector headers for positioning)
    // SeekP seeks in audio mode (uses Subchannel Q for positioning)
    // TODO do SeekL and SeekP need to behave differently?
    pub(super) fn execute_seek(&mut self) -> CommandState {
        int3!(self, [stat!(self)]);

        let seek_location = self.seek_location.take().unwrap_or(self.drive_state.current_time());
        self.drive_state =
            determine_drive_state(self.drive_state, seek_location, SeekNextState::Pause);

        log::debug!(
            "Executed Seek command to {seek_location}, drive state is {:?}",
            self.drive_state
        );

        CommandState::Idle
    }
}

pub(super) fn determine_drive_state(
    drive_state: DriveState,
    destination: CdTime,
    next: SeekNextState,
) -> DriveState {
    match drive_state {
        DriveState::Stopped => DriveState::SpinningUp {
            cycles_remaining: cd::SPIN_UP_CYCLES,
            next: SpinUpNextState::Seek(destination, next),
        },
        DriveState::SpinningUp { cycles_remaining, .. } => DriveState::SpinningUp {
            cycles_remaining,
            next: SpinUpNextState::Seek(destination, next),
        },
        DriveState::Seeking { destination: time, .. }
        | DriveState::PreparingToRead { time, .. }
        | DriveState::Reading(ReadState { time, .. })
        | DriveState::PreparingToPlay { time, .. }
        | DriveState::Playing(PlayState { time, .. })
        | DriveState::Paused(time) => {
            let seek_cycles = cmp::max(MIN_SEEK_CYCLES, estimate_seek_cycles(time, destination));
            DriveState::Seeking { destination, cycles_remaining: seek_cycles, next }
        }
    }
}

pub(super) fn estimate_seek_cycles(current: CdTime, destination: CdTime) -> u32 {
    if current == destination {
        return 1;
    }

    let diff = if current < destination { destination - current } else { current - destination };
    let diff_sectors = diff.to_sector_number();

    // Assume that it takes about a second to seek 60 minutes
    // TODO this is not accurate, but accurate seek timings are possibly not known?
    let sectors_per_cycle = 270000.0 / 44100.0;
    (f64::from(diff_sectors) / sectors_per_cycle).ceil() as u32
}
