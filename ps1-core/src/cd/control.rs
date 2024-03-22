use crate::cd;
#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::{status, CdController, Command, CommandState, DriveState, SpinUpNextState};
use crate::num::U8Ext;
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum DriveSpeed {
    #[default]
    Normal,
    Double,
}

impl DriveSpeed {
    pub fn from_bit(bit: bool) -> Self {
        if bit { Self::Double } else { Self::Normal }
    }

    pub fn cycles_between_sectors(self) -> u32 {
        match self {
            // 44100 Hz / 75 Hz
            Self::Normal => 588,
            // 44100 Hz / (2 * 75 Hz)
            Self::Double => 294,
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DriveMode {
    pub speed: DriveSpeed,
    pub audio_report_interrupts: bool,
    pub auto_pause_audio: bool,
    pub cd_da_enabled: bool,
}

impl DriveMode {
    pub fn new() -> Self {
        Self::from(0)
    }
}

impl From<u8> for DriveMode {
    fn from(mode: u8) -> Self {
        let speed = DriveSpeed::from_bit(mode.bit(7));
        let audio_report_interrupts = mode.bit(2);
        let auto_pause_audio = mode.bit(1);
        let cd_da_enabled = mode.bit(0);

        if mode.bit(6) {
            log::warn!("CD-XA ADPCM enabled via SetMode; not implemented");
        }

        if mode.bit(5) {
            todo!("2340-byte sector mode enabled via SetMode");
        }

        if mode.bit(4) {
            todo!("SetMode 'ignore bit' was set");
        }

        if mode.bit(3) {
            log::warn!("CD-XA filtering enabled via SetMode; not implemented");
        }

        Self { speed, audio_report_interrupts, auto_pause_audio, cd_da_enabled }
    }
}

// Roughly a second
const STOP_SECOND_RESPONSE_CYCLES: u32 = 44_100;

impl CdController {
    // $0A: Init() -> INT3(stat), INT2(stat)
    // Resets mode, aborts any in-progress commands, and activates the drive motor if it is stopped
    pub(super) fn execute_init(&mut self) -> CommandState {
        // TODO should set mode to $20 (everything 0 except 2340-byte sectors)
        self.drive_mode = DriveMode::from(0);

        if !self.drive_state.is_stopped_or_spinning_up() {
            self.drive_state = DriveState::Paused(self.drive_state.current_time());
        }

        int3!(self, [stat!(self)]);

        match self.drive_state {
            DriveState::Stopped => {
                self.drive_state = DriveState::SpinningUp {
                    cycles_remaining: cd::SPIN_UP_CYCLES,
                    next: SpinUpNextState::Pause,
                };
                CommandState::Idle
            }
            DriveState::SpinningUp { cycles_remaining, .. } => {
                self.drive_state =
                    DriveState::SpinningUp { cycles_remaining, next: SpinUpNextState::Pause };
                CommandState::Idle
            }
            _ => CommandState::GeneratingSecondResponse {
                command: Command::Init,
                cycles_remaining: status::GET_ID_SECOND_CYCLES,
            },
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

        self.drive_mode = mode.into();

        log::debug!("Parsed mode: {:?}", self.drive_mode);

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

        let cycles_till_second_response = 5 * self.drive_mode.speed.cycles_between_sectors();
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
        if !self.drive_state.is_stopped_or_spinning_up() {
            self.drive_state = DriveState::Paused(self.drive_state.current_time());
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

    // $0D: SetFilter(file, channel) -> INT3(stat)
    // Sets the file and channel for CD-XA ADPCM filtering
    pub(super) fn execute_set_filter(&mut self) -> CommandState {
        if self.parameter_fifo.len() < 2 {
            int5!(self, [stat!(self, ERROR), status::WRONG_NUM_PARAMETERS]);
            return CommandState::Idle;
        }

        self.adpcm_file = self.parameter_fifo.pop();
        self.adpcm_channel = self.parameter_fifo.pop();

        log::debug!("SetFilter executed: file={}, channel={}", self.adpcm_file, self.adpcm_channel);

        int3!(self, [stat!(self)]);

        CommandState::Idle
    }
}
