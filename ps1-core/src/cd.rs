//! PS1 CD-ROM controller and drive

mod fifo;
mod macros;
mod seek;
mod status;

use crate::cd::fifo::{ParameterFifo, ResponseFifo, ZeroFill};
use crate::cd::status::ErrorFlags;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::num::U8Ext;
use cdrom::cdtime::CdTime;
use cdrom::reader::CdRom;
#[allow(clippy::wildcard_imports)]
use macros::*;

// Roughly 23,796 CPU cycles
const RECEIVE_COMMAND_CYCLES_STOPPED: u32 = 31;

// Roughly 50,401 CPU cycles
const RECEIVE_COMMAND_CYCLES_RUNNING: u32 = 65;

// Roughly 81,102 CPU cycles
const INIT_COMMAND_CYCLES: u32 = 105;

// Roughly half a second
// TODO is this too fast?
const SPIN_UP_CYCLES: u32 = 22_050;

#[derive(Debug, Clone, Copy)]
struct CdInterruptRegisters {
    enabled: u8,
    flags: u8,
    prev_pending: bool,
}

impl CdInterruptRegisters {
    fn new() -> Self {
        Self { enabled: 0, flags: 0, prev_pending: false }
    }

    fn pending(self) -> bool {
        self.enabled & self.flags != 0
    }

    fn read_flags(self) -> u8 {
        // Bits 5-7 apparently always read as 1?
        let flags = 0xE0 | self.flags;
        log::trace!("Interrupt flags read: {flags:02X}");
        flags
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    // $01
    GetStat,
    // $02
    SetLoc,
    // $15
    SeekL,
    // $16
    SeekP,
    // $19
    Test,
    // $1A
    GetId,
    // $1E
    ReadToc,
}

#[derive(Debug, Clone, Copy)]
enum CommandState {
    Idle,
    ReceivingCommand { command: Command, cycles_remaining: u32 },
    GeneratingSecondResponse { command: Command, cycles_remaining: u32 },
    WaitingForSpinUp(Command),
    WaitingForSeek(Command),
}

impl Default for CommandState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveState {
    Stopped,
    SpinningUp { cycles_remaining: u32 },
    Paused(CdTime),
    Seeking { destination: CdTime, cycles_remaining: u32 },
}

impl Default for DriveState {
    fn default() -> Self {
        Self::Stopped
    }
}

impl DriveState {
    fn current_time(self) -> CdTime {
        match self {
            Self::Stopped | Self::SpinningUp { .. } => CdTime::ZERO,
            Self::Paused(time) | Self::Seeking { destination: time, .. } => time,
        }
    }
}

#[derive(Debug)]
pub struct CdController {
    index: u8,
    disc: Option<CdRom>,
    interrupts: CdInterruptRegisters,
    parameter_fifo: ParameterFifo,
    response_fifo: ResponseFifo,
    command_state: CommandState,
    drive_state: DriveState,
    seek_location: CdTime,
}

impl CdController {
    pub fn new(disc: Option<CdRom>) -> Self {
        Self {
            index: 0,
            disc,
            interrupts: CdInterruptRegisters::new(),
            parameter_fifo: ParameterFifo::new(),
            response_fifo: ResponseFifo::new(),
            command_state: CommandState::default(),
            drive_state: DriveState::default(),
            seek_location: CdTime::ZERO,
        }
    }

    // 44100 Hz clock
    pub fn clock(&mut self, interrupt_registers: &mut InterruptRegisters) {
        self.advance_drive_state();
        self.advance_command_state();

        let interrupt_pending = self.interrupts.pending();
        if !self.interrupts.prev_pending && interrupt_pending {
            // Flag a CD-ROM interrupt on any 0->1 transition
            // TODO apparently there should be a small delay before the interrupt flag is set in I_STAT?
            interrupt_registers.set_interrupt_flag(InterruptType::CdRom);
            log::debug!("CD-ROM INT{} generated", self.interrupts.enabled & self.interrupts.flags);
        }
        self.interrupts.prev_pending = interrupt_pending;
    }

    fn advance_drive_state(&mut self) {
        self.drive_state = match self.drive_state {
            DriveState::Stopped => DriveState::Stopped,
            DriveState::SpinningUp { cycles_remaining: 1 } => {
                log::debug!("Drive finished spinning up");
                DriveState::Paused(CdTime::ZERO)
            }
            DriveState::SpinningUp { cycles_remaining } => {
                DriveState::SpinningUp { cycles_remaining: cycles_remaining - 1 }
            }
            DriveState::Seeking { destination, cycles_remaining: 1 } => {
                log::debug!("Drive finished seeking to {destination}");
                DriveState::Paused(destination)
            }
            DriveState::Seeking { destination, cycles_remaining } => {
                DriveState::Seeking { destination, cycles_remaining: cycles_remaining - 1 }
            }
            DriveState::Paused(time) => DriveState::Paused(time),
        };
    }

    fn advance_command_state(&mut self) {
        self.command_state = match self.command_state {
            CommandState::Idle => CommandState::Idle,
            CommandState::ReceivingCommand { command, cycles_remaining: 1 } => {
                if !self.interrupts.pending() {
                    self.execute_command(command)
                } else {
                    // If an interrupt is pending, the controller waits until it is cleared
                    CommandState::ReceivingCommand { command, cycles_remaining: 1 }
                }
            }
            CommandState::ReceivingCommand { command, cycles_remaining } => {
                CommandState::ReceivingCommand { command, cycles_remaining: cycles_remaining - 1 }
            }
            CommandState::GeneratingSecondResponse { command, cycles_remaining: 1 } => {
                if !self.interrupts.pending() {
                    self.generate_second_response(command)
                } else {
                    // If an interrupt is pending, the controller waits until it is cleared
                    CommandState::GeneratingSecondResponse { command, cycles_remaining: 1 }
                }
            }
            CommandState::GeneratingSecondResponse { command, cycles_remaining } => {
                CommandState::GeneratingSecondResponse {
                    command,
                    cycles_remaining: cycles_remaining - 1,
                }
            }
            CommandState::WaitingForSpinUp(command) => match self.drive_state {
                DriveState::Stopped => {
                    panic!("Drive is stopped while command {command:?} is waiting for spin-up")
                }
                DriveState::SpinningUp { .. } => CommandState::WaitingForSpinUp(command),
                DriveState::Paused(_) | DriveState::Seeking { .. } => match command {
                    Command::SeekL | Command::SeekP => self.seek_drive_spun_up(command),
                    _ => panic!("Unexpected command waiting for drive spin-up: {command:?}"),
                },
            },
            CommandState::WaitingForSeek(command) => match self.drive_state {
                DriveState::Stopped | DriveState::SpinningUp { .. } => panic!(
                    "Invalid drive state while command is waiting for seek: {:?}",
                    self.drive_state
                ),
                DriveState::Seeking { .. } => CommandState::WaitingForSeek(command),
                DriveState::Paused(_) => match command {
                    Command::SeekL | Command::SeekP => self.seek_second_response(),
                    _ => panic!("Unexpected command waiting for seek: {command:?}"),
                },
            },
        };
    }

    fn execute_command(&mut self, command: Command) -> CommandState {
        log::debug!("Executing command {command:?}");

        let new_state = match command {
            Command::GetStat => self.execute_get_stat(),
            Command::SetLoc => self.execute_set_loc(),
            Command::SeekL | Command::SeekP => self.execute_seek(command),
            Command::Test => self.execute_test(),
            Command::GetId => self.execute_get_id(),
            Command::ReadToc => self.execute_read_toc(),
        };

        self.parameter_fifo.reset(ZeroFill::Yes);

        log::debug!("  New state: {new_state:?}");

        new_state
    }

    // $19: Test(sub_function) -> varies based on sub-function
    // Only sub-function $20 (get CD controller ROM version) is implemented
    fn execute_test(&mut self) -> CommandState {
        if self.parameter_fifo.len() != 1 {
            int5!(self, [self.status_code(ErrorFlags::ERROR), status::INVALID_COMMAND]);
            return CommandState::Idle;
        }

        match self.parameter_fifo.pop() {
            0x20 => {
                // TODO use a different BIOS version?
                int3!(self, [0x95, 0x07, 0x24, 0xC1]);
            }
            other => todo!("Test sub-function {other:02X}"),
        }

        CommandState::Idle
    }

    fn generate_second_response(&mut self, command: Command) -> CommandState {
        log::debug!("Generating second response for command {command:?}");

        match command {
            Command::SeekL | Command::SeekP => self.seek_second_response(),
            Command::GetId => self.get_id_second_response(),
            Command::ReadToc => self.read_toc_second_response(),
            _ => panic!("Invalid state, command {command:?} should not send a second response"),
        }
    }

    pub fn read_port(&mut self, address: u32) -> u8 {
        log::trace!("CD-ROM register read: {address:08X}.{}", self.index);

        match (address & 3, self.index) {
            (0, _) => {
                // $1F801800 R: Index/status register
                self.read_status_register()
            }
            (1, _) => {
                // $1F801801 R: Response FIFO
                self.read_response_fifo()
            }
            (3, 1 | 3) => {
                // $1F801803.1/3 R: Interrupt flags register
                self.interrupts.read_flags()
            }
            _ => todo!("CD-ROM read {address:08X}.{}", self.index),
        }
    }

    pub fn write_port(&mut self, address: u32, value: u8) {
        log::trace!("CD-ROM register write: {address:08X}.{} {value:02X}", self.index);

        match (address & 3, self.index) {
            (0, _) => {
                // $1F801800 W: Index/status register
                self.write_index_register(value);
            }
            (1, 0) => {
                // $1F801801.0 W: Command register
                self.write_command(value);
            }
            (2, 0) => {
                // $1F801802.0 W: Parameter FIFO
                self.write_parameter_fifo(value);
            }
            (2, 1) => {
                // $1F801802.1 W: Interrupts enabled register
                self.write_interrupts_enabled(value);
            }
            (3, 1) => {
                // $1F801803.1 W: Interrupt flags register
                self.write_interrupt_flags(value);
            }
            _ => todo!("CD-ROM write {address:08X}.{} {value:02X}", self.index),
        }
    }

    fn read_status_register(&self) -> u8 {
        // TODO Bit 2: XA-ADPCM FIFO not empty (hardcoded to 0)
        // TODO Bit 6: Data FIFO not empty (hardcoded to 0)
        let receiving_command = matches!(self.command_state, CommandState::ReceivingCommand { .. });
        let status = self.index
            | (u8::from(self.parameter_fifo.empty()) << 3)
            | (u8::from(!self.parameter_fifo.full()) << 4)
            | (u8::from(!self.response_fifo.fully_consumed()) << 5)
            | (u8::from(receiving_command) << 7);

        log::debug!("Status read: {status:02X}");

        status
    }

    fn write_index_register(&mut self, value: u8) {
        // Only bits 0-1 (index) are writable
        self.index = value & 3;
        log::trace!("Index changed to {}", self.index);
    }

    fn write_command(&mut self, command: u8) {
        let std_receive_cycles = match self.drive_state {
            DriveState::Stopped => RECEIVE_COMMAND_CYCLES_STOPPED,
            _ => RECEIVE_COMMAND_CYCLES_RUNNING,
        };

        let (command, cycles) = match command {
            0x01 => (Command::GetStat, std_receive_cycles),
            0x02 => (Command::SetLoc, std_receive_cycles),
            0x15 => (Command::SeekL, std_receive_cycles),
            0x16 => (Command::SeekP, std_receive_cycles),
            0x19 => (Command::Test, std_receive_cycles),
            0x1A => (Command::GetId, std_receive_cycles),
            0x1E => (Command::ReadToc, INIT_COMMAND_CYCLES),
            _ => todo!("Command byte {command:02X}"),
        };
        self.command_state = CommandState::ReceivingCommand { command, cycles_remaining: cycles };

        log::debug!("Received command, new state: {:?}", self.command_state);
    }

    fn read_response_fifo(&mut self) -> u8 {
        let value = self.response_fifo.pop();
        log::debug!("Response FIFO read: {value:02X}");
        value
    }

    fn write_parameter_fifo(&mut self, value: u8) {
        self.parameter_fifo.push(value);
        log::debug!("  Parameter FIFO write (idx {}): {value:02X}", self.parameter_fifo.len() - 1);
    }

    fn write_interrupts_enabled(&mut self, value: u8) {
        self.interrupts.enabled = value & 0x1F;
        log::debug!("Interrupts enabled: {:02X}", self.interrupts.enabled);
    }

    fn write_interrupt_flags(&mut self, value: u8) {
        // Bits 0-4 acknowledge interrupts if set
        self.interrupts.flags &= !(value & 0x1F);
        log::debug!("Acknowledged CD-ROM interrupts: {:02X}", value & 0x1F);

        // Bit 6 resets the parameter FIFO if set
        if value.bit(6) {
            self.parameter_fifo.reset(ZeroFill::Yes);
            log::debug!("  Reset parameter FIFO");
        }
    }
}
