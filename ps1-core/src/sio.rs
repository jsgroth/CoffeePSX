//! PS1 serial I/O port 0 (SIO0), used to communicate with controllers and memory cards

mod rxfifo;

use crate::input::Ps1Inputs;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::num::U32Ext;
use crate::sio::rxfifo::RxFifo;
use bincode::{Decode, Encode};
use std::cmp;

#[derive(Debug, Clone, Copy, Encode, Decode)]
struct BaudrateTimer {
    timer: u32,
    raw_reload_value: u32,
    reload_factor: u32,
}

impl BaudrateTimer {
    fn new() -> Self {
        Self { timer: 0x0088, raw_reload_value: 0x0088, reload_factor: 2 }
    }

    fn tick(&mut self, mut cpu_cycles: u32) {
        // TODO this is terribly inefficient; improve this
        while cpu_cycles != 0 {
            let elapsed = cmp::min(self.timer, cpu_cycles);
            self.timer -= elapsed;
            cpu_cycles -= elapsed;

            if self.timer == 0 {
                self.timer = self.reload_value();
            }
        }
    }

    fn reload_value(&mut self) -> u32 {
        cmp::max(1, self.raw_reload_value * self.reload_factor / 2)
    }

    fn update_reload_value(&mut self, value: u32) {
        self.raw_reload_value = value;

        // Updating reload value triggers an immediate reload
        self.timer = self.reload_value();
    }

    fn update_reload_factor(&mut self, value: u32) {
        self.reload_factor = match value & 3 {
            0 => 1,
            1 => 2,
            2 => 16,
            3 => 64,
            _ => unreachable!("value & 3 is always <= 3"),
        };
        log::debug!("SIO0 Baudrate timer reload factor: {}", self.reload_factor);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum Port {
    #[default]
    One = 0,
    Two = 1,
}

impl Port {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Two } else { Self::One }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum TxFifoState {
    Empty,
    Queued(u8),
    Transferring { value: u8, cycles_remaining: u32, next: Option<u8> },
}

impl TxFifoState {
    fn ready_for_new_byte(self) -> bool {
        matches!(self, Self::Empty | Self::Transferring { next: None, .. })
    }
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
enum PortState {
    Idle,
    ReceivedControllerAddress,
    SentIdLow,
    SentIdHigh,
    SentDigitalLow,
    Disconnected,
    SendingZeroes,
}

const CONTROLLER_TRANSFER_CYCLES: u32 = 400;

#[derive(Debug, Clone, Encode, Decode)]
pub struct SerialPort {
    port_state: PortState,
    tx_fifo: TxFifoState,
    rx_fifo: RxFifo,
    tx_enabled: bool,
    dtr_on: bool,
    rx_enabled: bool,
    rx_interrupt_bytes: u8,
    tx_interrupt_enabled: bool,
    rx_interrupt_enabled: bool,
    dsr_interrupt_enabled: bool,
    selected_port: Port,
    baudrate_timer: BaudrateTimer,
    irq: bool,
    irq_delay_cycles: u16,
}

impl SerialPort {
    pub fn new() -> Self {
        Self {
            port_state: PortState::Idle,
            tx_fifo: TxFifoState::Empty,
            rx_fifo: RxFifo::new(),
            tx_enabled: false,
            dtr_on: false,
            rx_enabled: false,
            rx_interrupt_bytes: 1,
            tx_interrupt_enabled: false,
            rx_interrupt_enabled: false,
            dsr_interrupt_enabled: false,
            selected_port: Port::default(),
            baudrate_timer: BaudrateTimer::new(),
            irq: false,
            irq_delay_cycles: 0,
        }
    }

    pub fn tick(
        &mut self,
        cpu_cycles: u32,
        inputs: Ps1Inputs,
        interrupt_registers: &mut InterruptRegisters,
    ) {
        self.baudrate_timer.tick(cpu_cycles);

        if self.irq_delay_cycles != 0 {
            self.irq_delay_cycles = self.irq_delay_cycles.saturating_sub(cpu_cycles as u16);
            if self.irq_delay_cycles == 0 {
                interrupt_registers.set_interrupt_flag(InterruptType::Sio0);
            }
        }

        match self.tx_fifo {
            TxFifoState::Empty => {}
            TxFifoState::Queued(value) => {
                if self.tx_enabled {
                    self.tx_fifo = TxFifoState::Transferring {
                        value,
                        cycles_remaining: CONTROLLER_TRANSFER_CYCLES,
                        next: None,
                    };
                }
            }
            TxFifoState::Transferring { value, cycles_remaining, next } => {
                let cycles_remaining = cycles_remaining.saturating_sub(cpu_cycles);
                if cycles_remaining == 0 {
                    self.process_tx_write(value, inputs);
                    self.tx_fifo = match (next, self.tx_enabled) {
                        (Some(next_value), true) => TxFifoState::Transferring {
                            value: next_value,
                            cycles_remaining: CONTROLLER_TRANSFER_CYCLES,
                            next: None,
                        },
                        (Some(next_value), false) => TxFifoState::Queued(next_value),
                        (None, _) => TxFifoState::Empty,
                    };
                } else {
                    self.tx_fifo = TxFifoState::Transferring { value, cycles_remaining, next };
                }
            }
        }
    }

    fn process_tx_write(&mut self, value: u8, inputs: Ps1Inputs) {
        log::debug!("Processing SIO0 TX_DATA write {value:02X}");

        self.port_state = match (self.port_state, value) {
            (PortState::Idle, 0x01) if self.selected_port == Port::One => {
                self.rx_fifo.push(0);
                PortState::ReceivedControllerAddress
            }
            // TODO memory cards and P2
            (PortState::Idle | PortState::SendingZeroes, _) => {
                self.rx_fifo.push(0);
                PortState::SendingZeroes
            }
            // TODO ID hardcoded to $5A41 (digital controller)
            (PortState::ReceivedControllerAddress, 0x42) => {
                self.rx_fifo.push(0x41);
                PortState::SentIdLow
            }
            (PortState::ReceivedControllerAddress, _) => {
                self.rx_fifo.push(0x41);
                PortState::Disconnected
            }
            // TODO memory cards
            // (PortState::ReceivedControllerAddress, _) => {
            //     todo!("SIO0 controller, second byte was {value:02X}")
            // }
            (PortState::SentIdLow, _) => {
                self.rx_fifo.push(0x5A);
                PortState::SentIdHigh
            }
            (PortState::SentIdHigh, _) => {
                self.rx_fifo.push((!u16::from(inputs.p1)) as u8);
                PortState::SentDigitalLow
            }
            (PortState::SentDigitalLow, _) => {
                self.rx_fifo.push((!(u16::from(inputs.p1) >> 8)) as u8);
                PortState::Idle
            }
            (PortState::Disconnected, _) => PortState::Disconnected,
        };

        if self.dsr_interrupt_enabled && !self.irq && !matches!(self.port_state, PortState::Idle) {
            self.irq = true;
            self.irq_delay_cycles = 100;
        }
    }

    pub fn write_tx_data(&mut self, tx_data: u32) {
        let tx_data = tx_data as u8;

        self.tx_fifo = match self.tx_fifo {
            TxFifoState::Empty | TxFifoState::Queued(_) => {
                if self.tx_enabled {
                    TxFifoState::Transferring {
                        value: tx_data,
                        cycles_remaining: CONTROLLER_TRANSFER_CYCLES,
                        next: None,
                    }
                } else {
                    TxFifoState::Queued(tx_data)
                }
            }
            TxFifoState::Transferring { value, cycles_remaining, .. } => {
                TxFifoState::Transferring { value, cycles_remaining, next: Some(tx_data) }
            }
        };

        log::debug!("SIO0_TX_DATA write: {tx_data:02X}");
    }

    pub fn read_rx_data(&mut self) -> u32 {
        let value = self.rx_fifo.pop();
        log::debug!("RX_DATA read: {value:02X}");
        value.into()
    }

    // $1F801044: SIO0_STAT
    pub fn read_status(&self) -> u32 {
        // TODO Bit 7: DSR input level (/ACK)
        let value = u32::from(self.tx_fifo.ready_for_new_byte())
            | (u32::from(!self.rx_fifo.empty()) << 1)
            | (u32::from(self.tx_fifo == TxFifoState::Empty) << 2)
            | (u32::from(matches!(self.port_state, PortState::Idle | PortState::Disconnected))
                << 7)
            | (u32::from(self.irq) << 9)
            | (self.baudrate_timer.timer << 11);

        log::debug!("SIO0_STAT read: {value:08X}");
        value
    }

    // $1F801048: SIO0_MODE
    pub fn write_mode(&mut self, value: u32) {
        self.baudrate_timer.update_reload_factor(value);

        if value & 0xC != 0xC {
            todo!("Expected character length to be 8-bit (3), was {}", (value >> 2) & 3);
        }

        if value.bit(4) {
            todo!("Expected parity bit to be clear, was set");
        }

        if value.bit(8) {
            todo!("Expected clock polarity to be high-when-idle (0), was low-when-idle (1)");
        }
    }

    // $1F80104A: SIO0_CTRL
    pub fn read_control(&self) -> u32 {
        let rx_mode = match self.rx_interrupt_bytes {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => panic!("Unexpected RX IRQ FIFO length: {}", self.rx_interrupt_bytes),
        };

        let value = u32::from(self.tx_enabled)
            | (u32::from(self.dtr_on) << 1)
            | (u32::from(self.rx_enabled) << 2)
            | (rx_mode << 8)
            | (u32::from(self.tx_interrupt_enabled) << 10)
            | (u32::from(self.rx_interrupt_enabled) << 11)
            | (u32::from(self.dsr_interrupt_enabled) << 12)
            | ((self.selected_port as u32) << 13);

        log::debug!("SIO0_CTRL read: {value:04X}");
        value
    }

    // $1F80104A: SIO0_CTRL
    pub fn write_control(&mut self, value: u32) {
        if value.bit(6) {
            // Reset bit
            log::debug!("SIO0 reset");
            self.write_mode(0xC);
            self.write_control(0);
            self.rx_fifo.clear();
            return;
        }

        self.tx_enabled = value.bit(0);
        self.dtr_on = value.bit(1);
        self.rx_enabled = value.bit(2);
        self.rx_interrupt_bytes = 1 << ((value >> 8) & 3);
        self.rx_interrupt_enabled = value.bit(10);
        self.tx_interrupt_enabled = value.bit(11);
        self.dsr_interrupt_enabled = value.bit(12);
        self.selected_port = Port::from_bit(value.bit(13));

        if value.bit(4) {
            self.irq = false;
        }

        if !self.dtr_on {
            self.tx_fifo = TxFifoState::Empty;
            self.port_state = PortState::Idle;
        }

        log::debug!("SIO0_CTRL write: {value:04X}");
        log::debug!("  TX enabled: {}", self.tx_enabled);
        log::debug!("  DTR output on: {}", self.dtr_on);
        log::debug!("  RX enabled: {}", self.rx_enabled);
        log::debug!("  RX IRQ mode (FIFO length): {}", self.rx_interrupt_bytes);
        log::debug!("  RX IRQ enabled: {}", self.rx_interrupt_enabled);
        log::debug!("  TX IRQ enabled: {}", self.tx_interrupt_enabled);
        log::debug!("  DSR IRQ enabled: {}", self.dsr_interrupt_enabled);
        log::debug!("  Selected port: {:?}", self.selected_port);
    }

    // $1F80104E: SIO0_BAUD (Baudrate timer reload value)
    pub fn write_baudrate_reload(&mut self, value: u32) {
        self.baudrate_timer.update_reload_value(value);

        log::debug!("SIO0 Baudrate timer reload value: {value:04X}");
    }

    pub fn read_baudrate_reload(&self) -> u32 {
        self.baudrate_timer.raw_reload_value
    }
}
