#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{seek, CdController, CommandState, DriveState, SeekNextState};
use cdrom::cdtime::CdTime;
use cdrom::CdRomResult;

impl CdController {
    // $06: ReadN() -> INT3(stat), (INT1(stat), sector)*
    // $1B: ReadS() -> INT3(stat), (INT1(stat), sector)*
    // Commands the drive to start reading data from the position specified by the last SetLoc
    // command. Responds initially with INT3(stat), then generates INT1(stat) every time a new
    // sector is ready for the host to read. The drive continues reading sectors until the host
    // commands it to pause or stop.
    // ReadN reads with retry while ReadS reads without retry. These are emulated the same way.
    pub(super) fn execute_read(&mut self) -> CommandState {
        int3!(self, [stat!(self)]);

        let seek_location = self.seek_location.take().unwrap_or(self.drive_state.current_time());
        self.drive_state =
            seek::determine_drive_state(self.drive_state, seek_location, SeekNextState::Read);

        log::debug!(
            "Executed Read command at {seek_location}, drive state is {:?}",
            self.drive_state
        );

        CommandState::Idle
    }

    pub(super) fn read_data_sector(&mut self, time: CdTime) -> CdRomResult<DriveState> {
        self.read_sector_atime(time)?;

        log::debug!("  Data sector header: {:02X?}", &self.sector_buffer[12..16]);

        Ok(DriveState::Reading {
            time: time + CdTime::new(0, 0, 1),
            int1_generated: false,
            cycles_till_next: self.drive_mode.speed.cycles_between_sectors(),
        })
    }
}
