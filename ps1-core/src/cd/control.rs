#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::status::ErrorFlags;
use crate::cd::{status, CdController, CommandState, DriveSpeed};
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
}
