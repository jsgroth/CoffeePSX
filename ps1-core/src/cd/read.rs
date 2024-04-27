//! CD-ROM read commands

#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{seek, CdController, CommandState, DriveState, SeekNextState};
use crate::num::U8Ext;
use bincode::{Decode, Encode};
use cdrom::cdtime::CdTime;
use cdrom::CdRomResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct ReadState {
    pub time: CdTime,
    pub int1_generated: bool,
    pub cycles_till_next_sector: u32,
}

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
        if matches!(self.drive_state, DriveState::Reading(ReadState { time, .. }) if time == seek_location)
        {
            log::debug!("Drive is already reading at desired position of {seek_location}");
            return CommandState::Idle;
        }

        self.drive_state =
            seek::determine_drive_state(self.drive_state, seek_location, SeekNextState::Read);

        log::debug!(
            "Executed Read command at {seek_location}, drive state is {:?}",
            self.drive_state
        );

        CommandState::Idle
    }

    pub(super) fn progress_read_state(
        &mut self,
        ReadState { time, mut int1_generated, cycles_till_next_sector }: ReadState,
    ) -> CdRomResult<DriveState> {
        if let Some((sample_l, sample_r)) = self.xa_adpcm.maybe_output_sample() {
            self.current_audio_sample = (sample_l, sample_r);
        }

        if cycles_till_next_sector == 1 {
            return self.read_data_sector(time);
        }

        if !int1_generated
            && !self.interrupts.int_queued()
            && !matches!(self.command_state, CommandState::ReceivingCommand { .. })
        {
            int1_generated = true;
            int1!(self, [stat!(self)]);

            // TODO should the copy wait until software requests the data sector?
            if self.drive_mode.raw_sectors {
                self.data_fifo.copy_from_slice(&self.sector_buffer[12..2352]);
            } else {
                self.data_fifo.copy_from_slice(&self.sector_buffer[24..24 + 2048]);
            }
        }

        Ok(DriveState::Reading(ReadState {
            time,
            int1_generated,
            cycles_till_next_sector: cycles_till_next_sector - 1,
        }))
    }

    pub(super) fn read_data_sector(&mut self, time: CdTime) -> CdRomResult<DriveState> {
        self.read_sector_atime(time)?;

        log::debug!(
            "  Data sector header: {:02X?} subheader: {:02X?}",
            &self.sector_buffer[12..16],
            &self.sector_buffer[16..20]
        );

        let file = self.sector_buffer[16];
        let channel = self.sector_buffer[17];
        let submode = self.sector_buffer[18];
        let is_real_time_audio = submode.bit(2) && submode.bit(6);

        let mut should_generate_int1 = true;
        if self.drive_mode.adpcm_enabled
            && is_real_time_audio
            && (!self.drive_mode.adpcm_filter_enabled
                || (self.xa_adpcm.file == file && self.xa_adpcm.channel == channel))
        {
            // CD-XA ADPCM sector; send to ADPCM decoder instead of the data FIFO
            should_generate_int1 = false;

            log::debug!("Decoding CD-XA ADPCM sector at {time}");
            self.xa_adpcm.decode_sector(self.sector_buffer.as_ref());
        } else if self.drive_mode.adpcm_filter_enabled && is_real_time_audio {
            // The controller does not send sectors to the data FIFO if ADPCM filtering is enabled
            // and this is a real-time audio sector
            should_generate_int1 = false;
        }

        Ok(DriveState::Reading(ReadState {
            time: time + CdTime::new(0, 0, 1),
            int1_generated: !should_generate_int1,
            cycles_till_next_sector: self.drive_mode.speed.cycles_between_sectors(),
        }))
    }
}
