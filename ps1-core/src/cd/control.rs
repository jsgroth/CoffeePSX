#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{seek, status, CdController, Command, CommandState, DriveSpeed, DriveState};
use crate::num::U8Ext;
use cdrom::cdtime::CdTime;

// Roughly a second
const STOP_SECOND_RESPONSE_CYCLES: u32 = 44_100;

impl CdController {
    // $0A: Init() -> INT3(stat), INT2(stat)
    // Resets mode, aborts any in-progress commands, and activates the drive motor if it is stopped
    pub(super) fn execute_init(&mut self) -> CommandState {
        int3!(self, [stat!(self)]);

        self.drive_speed = DriveSpeed::Normal;

        // TODO other SetMode bits

        if let Some(state) = seek::check_if_spin_up_needed(Command::Init, &mut self.drive_state) {
            return state;
        }

        self.init_drive_spun_up()
    }

    pub(super) fn init_drive_spun_up(&mut self) -> CommandState {
        self.drive_state = DriveState::Paused(CdTime::ZERO);

        CommandState::GeneratingSecondResponse {
            command: Command::Init,
            cycles_remaining: status::GET_ID_SECOND_CYCLES,
        }
    }

    pub(super) fn init_second_response(&mut self) -> CommandState {
        int2!(self, [stat!(self)]);
        CommandState::Idle
    }

    // $0E: SetMode(mode) -> INT3(stat)
    // Configures drive mode
    pub(super) fn execute_set_mode(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 1 {
            int5!(self, [stat!(self, ERROR), status::WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        let mode = self.parameter_fifo.pop();
        log::debug!("Mode: {mode:02X}");

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

        int3!(self, [stat!(self)]);
        CommandState::Idle
    }

    // $09: Pause() -> INT3(stat), INT2(stat)
    // Aborts any in-progress read or play command and leaves the motor running, with the drive
    // staying in roughly the same position
    pub(super) fn execute_pause(&mut self) -> CommandState {
        // Generate INT3 before pausing the drive
        int3!(self, [stat!(self)]);

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
        int2!(self, [stat!(self)]);
        CommandState::Idle
    }

    // $08: Stop() -> INT3(stat), INT2(stat)
    // Stops the drive motor
    pub(super) fn execute_stop(&mut self) -> CommandState {
        // Pause drive before generating INT3 stat
        // TODO also check playing states
        match self.drive_state {
            DriveState::PreparingToRead { time, .. } | DriveState::Reading { time, .. } => {
                self.drive_state = DriveState::Paused(time);
            }
            _ => {}
        }

        int3!(self, [stat!(self)]);

        CommandState::GeneratingSecondResponse {
            command: Command::Stop,
            cycles_remaining: STOP_SECOND_RESPONSE_CYCLES,
        }
    }

    pub(super) fn stop_second_response(&mut self) -> CommandState {
        self.drive_state = DriveState::Stopped;
        int2!(self, [stat!(self)]);
        CommandState::Idle
    }
}
