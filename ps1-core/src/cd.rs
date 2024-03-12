//! PS1 CD-ROM controller and drive

mod macros;
mod stat;

use crate::cd::stat::ErrorFlags;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::num::U8Ext;
#[allow(clippy::wildcard_imports)]
use macros::*;

// CPU clock speed = 44100 Hz * 768
const CD_CPU_DIVIDER: u32 = 768;

// Roughly 23,796 CPU cycles
const RECEIVE_COMMAND_CYCLES_STOPPED: u32 = 31;

const INVALID_COMMAND: u8 = 0x40;

#[derive(Debug, Clone, Copy)]
struct CdInterruptRegisters {
    enabled: u8,
    flags: u8,
    prev_pending: bool,
}

impl CdInterruptRegisters {
    fn new() -> Self {
        Self {
            enabled: 0,
            flags: 0,
            prev_pending: false,
        }
    }

    fn pending(self) -> bool {
        self.enabled & self.flags != 0
    }

    fn read_flags(self) -> u8 {
        // Bits 5-7 apparently always read as 1?
        let flags = 0xE0 | self.flags;
        log::debug!("  Interrupt flags read: {flags:02X}");
        flags
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZeroFill {
    Yes,
    // No,
}

#[derive(Debug, Clone)]
struct Fifo<const MAX_LEN: usize> {
    values: [u8; MAX_LEN],
    idx: usize,
    len: usize,
}

impl<const MAX_LEN: usize> Fifo<MAX_LEN> {
    fn new() -> Self {
        Self {
            values: [0; MAX_LEN],
            idx: 0,
            len: 0,
        }
    }

    fn reset(&mut self, zero_fill: ZeroFill) {
        self.idx = 0;
        self.len = 0;

        if zero_fill == ZeroFill::Yes {
            self.values.fill(0);
        }
    }

    fn push(&mut self, value: u8) {
        if self.len == self.values.len() {
            log::error!("Push to CD-ROM FIFO while full: {value:02X}");
            return;
        }

        self.values[self.len] = value;
        self.len += 1;
    }

    fn pop(&mut self) -> u8 {
        let value = self.values[self.idx];

        self.idx += 1;
        if self.idx == self.values.len() {
            self.idx = 0;
        }

        value
    }

    fn empty(&self) -> bool {
        self.len == 0
    }

    fn full(&self) -> bool {
        self.len == MAX_LEN
    }

    fn fully_consumed(&self) -> bool {
        self.idx >= self.len
    }
}

type ParameterFifo = Fifo<16>;
type ResponseFifo = Fifo<16>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    GetStat,
    Test,
    GetId,
}

#[derive(Debug, Clone, Copy)]
enum CommandState {
    Idle,
    ReceivingCommand {
        command: Command,
        cycles_remaining: u32,
    },
    GeneratingSecondResponse {
        command: Command,
        cycles_remaining: u32,
    },
}

impl Default for CommandState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone)]
enum DriveState {
    Stopped,
}

impl Default for DriveState {
    fn default() -> Self {
        Self::Stopped
    }
}

#[derive(Debug, Clone)]
pub struct CdController {
    index: u8,
    interrupts: CdInterruptRegisters,
    parameter_fifo: ParameterFifo,
    response_fifo: ResponseFifo,
    command_state: CommandState,
    drive_state: DriveState,
    cpu_cycles: u32,
}

impl CdController {
    pub fn new() -> Self {
        Self {
            index: 0,
            interrupts: CdInterruptRegisters::new(),
            parameter_fifo: ParameterFifo::new(),
            response_fifo: ResponseFifo::new(),
            command_state: CommandState::default(),
            drive_state: DriveState::default(),
            cpu_cycles: 0,
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32, interrupt_registers: &mut InterruptRegisters) {
        self.cpu_cycles += cpu_cycles;
        while self.cpu_cycles >= CD_CPU_DIVIDER {
            self.cpu_cycles -= CD_CPU_DIVIDER;
            self.clock(interrupt_registers);
        }
    }

    // 44100 Hz clock
    fn clock(&mut self, interrupt_registers: &mut InterruptRegisters) {
        self.advance_command_state();

        let interrupt_pending = self.interrupts.pending();
        if !self.interrupts.prev_pending && interrupt_pending {
            // Flag a CD-ROM interrupt on any 0->1 transition
            // TODO apparently there should be a small delay before the interrupt flag is set in I_STAT?
            interrupt_registers.set_interrupt_flag(InterruptType::CdRom);
            log::debug!(
                "CD-ROM INT{} generated",
                self.interrupts.enabled & self.interrupts.flags
            );
        }
        self.interrupts.prev_pending = interrupt_pending;
    }

    fn advance_command_state(&mut self) {
        self.command_state = match self.command_state {
            CommandState::Idle => CommandState::Idle,
            CommandState::ReceivingCommand {
                command,
                cycles_remaining,
            } => {
                if cycles_remaining == 1 {
                    if !self.interrupts.pending() {
                        self.execute_command(command)
                    } else {
                        // If an interrupt is pending, the controller waits until it is cleared
                        CommandState::ReceivingCommand {
                            command,
                            cycles_remaining: 1,
                        }
                    }
                } else {
                    CommandState::ReceivingCommand {
                        command,
                        cycles_remaining: cycles_remaining - 1,
                    }
                }
            }
            CommandState::GeneratingSecondResponse {
                command,
                cycles_remaining,
            } => {
                if cycles_remaining == 1 {
                    if !self.interrupts.pending() {
                        self.generate_second_response(command)
                    } else {
                        // If an interrupt is pending, the controller waits until it is cleared
                        CommandState::GeneratingSecondResponse {
                            command,
                            cycles_remaining: 1,
                        }
                    }
                } else {
                    CommandState::GeneratingSecondResponse {
                        command,
                        cycles_remaining: cycles_remaining - 1,
                    }
                }
            }
        };
    }

    fn execute_command(&mut self, command: Command) -> CommandState {
        log::debug!("Executing command {command:?}");

        let new_state = match command {
            Command::GetStat => self.execute_get_stat(),
            Command::Test => self.execute_test(),
            Command::GetId => self.execute_get_id(),
        };

        self.parameter_fifo.reset(ZeroFill::Yes);

        new_state
    }

    // $19: Test(sub_function) -> varies based on sub-function
    // Only sub-function $20 (get BIOS version) is implemented
    fn execute_test(&mut self) -> CommandState {
        if self.parameter_fifo.len != 1 {
            let stat = self.status_code(ErrorFlags::ERROR);
            int5!(self, [stat, INVALID_COMMAND]);
            return CommandState::Idle;
        }

        match self.parameter_fifo.values[0] {
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
            Command::GetId => self.get_id_second_response(),
            _ => panic!("Invalid state, command {command:?} should not send a second response"),
        }
    }

    pub fn read_port(&mut self, address: u32) -> u8 {
        log::debug!("CD-ROM register read: {address:08X}.{}", self.index);

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
        log::debug!(
            "CD-ROM register write: {address:08X}.{} {value:02X}",
            self.index
        );

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

        log::debug!("  Status read: {status:02X}");

        status
    }

    fn write_index_register(&mut self, value: u8) {
        // Only bits 0-1 (index) are writable
        self.index = value & 3;
        log::debug!("  Index changed to {}", self.index);
    }

    fn write_command(&mut self, command: u8) {
        let std_receive_cycles = match self.drive_state {
            DriveState::Stopped => RECEIVE_COMMAND_CYCLES_STOPPED,
        };

        let (command, cycles) = match command {
            0x01 => (Command::GetStat, std_receive_cycles),
            0x1A => (Command::GetId, std_receive_cycles),
            0x19 => (Command::Test, std_receive_cycles),
            _ => todo!("Command byte {command:02X}"),
        };
        self.command_state = CommandState::ReceivingCommand {
            command,
            cycles_remaining: cycles,
        };

        log::debug!("  Received command, new state: {:?}", self.command_state);
    }

    fn read_response_fifo(&mut self) -> u8 {
        let value = self.response_fifo.pop();
        log::debug!("  Response FIFO read: {value:02X}");
        value
    }

    fn write_parameter_fifo(&mut self, value: u8) {
        self.parameter_fifo.push(value);
        log::debug!(
            "  Parameter FIFO write (idx {}): {value:02X}",
            self.parameter_fifo.len - 1
        );
    }

    fn write_interrupts_enabled(&mut self, value: u8) {
        self.interrupts.enabled = value & 0x1F;
        log::debug!("  Interrupts enabled: {:02X}", self.interrupts.enabled);
    }

    fn write_interrupt_flags(&mut self, value: u8) {
        // Bits 0-4 acknowledge interrupts if set
        self.interrupts.flags &= !(value & 0x1F);
        log::debug!("  Acknowledged CD-ROM interrupts: {:02X}", value & 0x1F);

        // Bit 6 resets the parameter FIFO if set
        if value.bit(6) {
            self.parameter_fifo.reset(ZeroFill::Yes);
            log::debug!("  Reset parameter FIFO");
        }
    }
}
