use crate::cd;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::status::ErrorFlags;
use crate::cd::{status, CdController, Command, CommandState, DriveState};
use cdrom::cdtime::CdTime;

impl CdController {
    // $02: SetLoc(amm, ass, asect) -> INT3(stat)
    // Sets seek location to the specified absolute time
    pub(super) fn execute_set_loc(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 3 {
            int5!(self, [self.status_code(ErrorFlags::ERROR), status::WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        let minutes = self.parameter_fifo.pop();
        let seconds = self.parameter_fifo.pop();
        let frames = self.parameter_fifo.pop();

        match CdTime::new_checked(minutes, seconds, frames) {
            Some(cd_time) => {
                self.seek_location = cd_time;
                int3!(self, [self.status_code(ErrorFlags::NONE)]);

                log::debug!("Set seek location to {cd_time}");
            }
            None => {
                int5!(self, [self.status_code(ErrorFlags::ERROR), status::INVALID_COMMAND]);

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
        int3!(self, [self.status_code(ErrorFlags::NONE)]);

        if let Some(state) = check_if_spin_up_needed(command, &mut self.drive_state) {
            return state;
        }

        self.seek_drive_spun_up(command)
    }

    pub(super) fn seek_drive_spun_up(&mut self, command: Command) -> CommandState {
        let current_time = self.drive_state.current_time();
        let seek_cycles = estimate_seek_cycles(current_time, self.seek_location);
        self.drive_state =
            DriveState::Seeking { destination: self.seek_location, cycles_remaining: seek_cycles };

        CommandState::WaitingForSeek(command)
    }

    pub(super) fn seek_second_response(&mut self) -> CommandState {
        int2!(self, [self.status_code(ErrorFlags::NONE)]);
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
