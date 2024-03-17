#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::status::ErrorFlags;
use crate::cd::{status, CdController, Command, CommandState, DriveSpeed, DriveState};
use crate::num::U8Ext;

impl CdController {
    // $0E: SetMode(mode) -> INT3(stat)
    // Configures drive mode
    pub(super) fn execute_set_mode(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 1 {
            int5!(self, [self.status_code(ErrorFlags::ERROR), status::WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        let mode = self.parameter_fifo.pop();

        self.drive_speed = DriveSpeed::from_bit(mode.bit(7));

        if mode.bit(6) {
            todo!("CD-XA ADPCM enabled via SetMode");
        }

        if mode.bit(5) {
            todo!("2340-byte sector mode enabled via SetMode");
        }

        if mode.bit(4) {
            todo!("SetMode 'ignore bit' was set");
        }

        if mode.bit(3) {
            todo!("CD-XA ADPCM SetFilter enabled via SetMode");
        }

        if mode.bit(2) {
            todo!("Audio report interrupts enabled via SetMode");
        }

        if mode.bit(1) {
            todo!("Auto-pause enabled via SetMode");
        }

        if mode.bit(0) {
            todo!("CD-DA mode enabled via SetMode");
        }

        log::debug!("Drive speed: {:?}", self.drive_speed);

        int3!(self, [self.status_code(ErrorFlags::NONE)]);
        CommandState::Idle
    }

    // $09: Pause() -> INT3(stat), INT2(stat)
    // Aborts any in-progress read or play command and leaves the motor running, with the drive
    // staying in roughly the same position
    pub(super) fn execute_pause(&mut self) -> CommandState {
        // Generate INT3 before pausing the drive
        int3!(self, [self.status_code(ErrorFlags::NONE)]);

        // TODO check if motor is stopped

        self.drive_state = DriveState::Paused(self.drive_state.current_time());

        log::debug!("Paused drive at {}", self.drive_state.current_time());

        let cycles_till_second_response = 5 * self.drive_speed.cycles_between_sectors();
        CommandState::GeneratingSecondResponse {
            command: Command::Pause,
            cycles_remaining: cycles_till_second_response,
        }
    }

    pub(super) fn pause_second_response(&mut self) -> CommandState {
        int2!(self, [self.status_code(ErrorFlags::NONE)]);
        CommandState::Idle
    }
}
