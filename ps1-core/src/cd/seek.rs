use crate::cd;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{status, CdController, Command, CommandState, DriveState};
use cdrom::cdtime::CdTime;
use std::cmp;

// The BIOS does not like if a seek finishes too quickly
const MIN_SEEK_CYCLES: u32 = 24;

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
    pub(super) fn execute_seek(&mut self, command: Command) -> CommandState {
        int3!(self, [stat!(self)]);

        if let Some(state) = check_if_spin_up_needed(command, &mut self.drive_state) {
            return state;
        }

        self.seek_drive_spun_up(command)
    }

    pub(super) fn seek_drive_spun_up(&mut self, command: Command) -> CommandState {
        let seek_location = self.seek_location.take().unwrap_or(self.drive_state.current_time());

        let (drive_state, command_state) =
            seek_to_location(command, self.drive_state.current_time(), seek_location);
        self.drive_state = drive_state;
        command_state
    }

    pub(super) fn seek_second_response(&mut self) -> CommandState {
        int2!(self, [stat!(self)]);
        CommandState::Idle
    }
}

pub(super) fn check_if_spin_up_needed(
    command: Command,
    drive_state: &mut DriveState,
) -> Option<CommandState> {
    match *drive_state {
        DriveState::Stopped => {
            *drive_state = DriveState::SpinningUp { cycles_remaining: cd::SPIN_UP_CYCLES };
            Some(CommandState::WaitingForSpinUp(command))
        }
        DriveState::SpinningUp { .. } => Some(CommandState::WaitingForSpinUp(command)),
        _ => None,
    }
}

pub(super) fn seek_to_location(
    command: Command,
    current_time: CdTime,
    seek_location: CdTime,
) -> (DriveState, CommandState) {
    let seek_cycles = estimate_seek_cycles(current_time, seek_location);
    let drive_state = DriveState::Seeking {
        destination: seek_location,
        cycles_remaining: cmp::max(MIN_SEEK_CYCLES, seek_cycles),
    };
    let command_state = CommandState::WaitingForSeek(command);

    (drive_state, command_state)
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
