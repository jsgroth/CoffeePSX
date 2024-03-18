#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{seek, CdController, Command, CommandState, DriveState};
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

        if let Some(state) = seek::check_if_spin_up_needed(Command::ReadN, &mut self.drive_state) {
            return state;
        }

        self.read_drive_spun_up()
    }

    pub(super) fn read_drive_spun_up(&mut self) -> CommandState {
        let seek_location = self.seek_location.take().unwrap_or(self.drive_state.current_time());

        let current_time = self.drive_state.current_time();
        if current_time != seek_location {
            let (drive_state, command_state) =
                seek::seek_to_location(Command::ReadN, current_time, seek_location);
            self.drive_state = drive_state;
            return command_state;
        }

        self.read_seek_complete()
    }

    pub(super) fn read_seek_complete(&mut self) -> CommandState {
        // TODO is this right? delay by 5 sectors before first read
        self.drive_state = DriveState::PreparingToRead {
            time: self.drive_state.current_time(),
            cycles_remaining: 5 * self.drive_mode.speed.cycles_between_sectors(),
        };

        CommandState::Idle
    }

    pub(super) fn read_next_sector(&mut self, time: CdTime) -> CdRomResult<DriveState> {
        let Some(disc) = &mut self.disc else {
            // TODO separate state for no disc?
            return Ok(DriveState::Stopped);
        };

        let Some(track) = disc.cue().find_track_by_time(time) else {
            // TODO separate state for disc end
            log::debug!("Read to end of disc");
            return Ok(DriveState::Stopped);
        };

        let track_number = track.number;
        let relative_time = time - track.start_time;

        log::debug!("Reading sector at atime {time}, track {track_number} time {relative_time}");

        disc.read_sector(track_number, relative_time, self.sector_buffer.as_mut())?;

        log::debug!("  Data sector header: {:02X?}", &self.sector_buffer[12..16]);

        Ok(DriveState::Reading {
            time: time + CdTime::new(0, 0, 1),
            int1_generated: false,
            cycles_till_next: self.drive_mode.speed.cycles_between_sectors(),
        })
    }
}
