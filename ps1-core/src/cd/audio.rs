//! CD-ROM audio commands

use crate::cd;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{seek, CdController, CommandState, DriveState, SeekNextState};
use bincode::{Decode, Encode};
use cdrom::cdtime::CdTime;
use cdrom::CdRomResult;

pub const CD_DA_SAMPLES_PER_SECTOR: u16 = 588;
const SECTORS_BETWEEN_REPORTS: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PlayState {
    pub time: CdTime,
    pub sample_idx: u16,
    pub sectors_till_report: u8,
}

impl PlayState {
    pub fn new(time: CdTime) -> Self {
        Self { time, sample_idx: 0, sectors_till_report: SECTORS_BETWEEN_REPORTS }
    }
}

impl CdController {
    // $0C: Demute() -> INT3(stat)
    // Demutes CD audio output, both CD-DA and ADPCM
    pub(super) fn execute_demute(&mut self) -> CommandState {
        log::warn!("Demute command not yet implemented");

        int3!(self, [stat!(self)]);

        CommandState::Idle
    }

    // $03: Play(track?) -> INT3(stat), INT1(report)*
    // Begins audio playback from the specified location.
    // If track parameter is present and non-zero, begins playback from the start of that track.
    // If track parameter is zero or not present, begins playback from the last SetLoc location, or
    // the current time if there is no unprocessed SetLoc location.
    pub(super) fn execute_play(&mut self) -> CommandState {
        int3!(self, [stat!(self)]);

        let Some(disc) = &self.disc else {
            // TODO generate error response
            todo!("Play command issued with no disc");
        };

        let track_number = if self.parameter_fifo.empty() {
            0
        } else {
            cd::bcd_to_binary(self.parameter_fifo.pop())
        };

        let current_time = self.drive_state.current_time();
        let num_tracks = disc.cue().last_track().number;
        let track_start_time = if track_number == 0 {
            // Track number 0 (or not set) means use SetLoc location
            self.seek_location.take().unwrap_or(current_time)
        } else {
            // Track numbers above the last track number should wrap back around to 1
            let wrapped_track_number = ((track_number - 1) % num_tracks) + 1;
            disc.cue().track(wrapped_track_number).effective_start_time()
        };

        log::debug!("Executing Play command: track {track_number}, start time {track_start_time}");

        self.drive_state =
            seek::determine_drive_state(self.drive_state, track_start_time, SeekNextState::Play);

        CommandState::Idle
    }

    pub(super) fn read_audio_sector(
        &mut self,
        PlayState { time, sectors_till_report, .. }: PlayState,
    ) -> CdRomResult<DriveState> {
        // TODO check if at end of track
        // TODO generate audio report

        self.read_sector_atime(time)?;

        Ok(DriveState::Playing(PlayState {
            time: time + CdTime::new(0, 0, 1),
            sample_idx: 0,
            sectors_till_report: sectors_till_report - 1,
        }))
    }

    pub(super) fn progress_play_state(
        &mut self,
        PlayState { time, mut sample_idx, sectors_till_report }: PlayState,
    ) -> CdRomResult<DriveState> {
        if self.drive_mode.cd_da_enabled {
            let sample_addr = (sample_idx * 4) as usize;

            let sample_l = i16::from_le_bytes([
                self.sector_buffer[sample_addr],
                self.sector_buffer[sample_addr + 1],
            ]);
            let sample_r = i16::from_le_bytes([
                self.sector_buffer[sample_addr + 2],
                self.sector_buffer[sample_addr + 3],
            ]);
            self.current_audio_sample = (sample_l, sample_r);
        }

        sample_idx += 1;
        if sample_idx == CD_DA_SAMPLES_PER_SECTOR {
            self.read_audio_sector(PlayState { time, sample_idx: 0, sectors_till_report })
        } else {
            Ok(DriveState::Playing(PlayState { time, sample_idx, sectors_till_report }))
        }
    }
}
