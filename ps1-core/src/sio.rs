//! PS1 serial I/O port 0 (SIO0), used to communicate with controllers and memory cards

mod rxfifo;

use crate::num::U32Ext;
use crate::sio::rxfifo::RxFifo;
use std::cmp;

#[derive(Debug, Clone, Copy)]
struct BaudrateTimer {
    timer: u32,
    raw_reload_value: u32,
    reload_factor: u32,
}

impl BaudrateTimer {
    fn new() -> Self {
        Self {
            timer: 0x0088,
            raw_reload_value: 0x0088,
            reload_factor: 2,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Port {
    #[default]
    One = 0,
    Two = 1,
}

impl Port {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Two
        } else {
            Self::One
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxFifoState {
    Empty,
    Queued(u8),
    Transferring {
        value: u8,
        bits_remaining: u8,
        next: Option<u8>,
    },
}

impl TxFifoState {
    fn ready_for_new_byte(self) -> bool {
        matches!(self, Self::Empty | Self::Transferring { next: None, .. })
    }
}

#[derive(Debug, Clone)]
pub struct SerialPort {
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
}

impl SerialPort {
    pub fn new() -> Self {
        Self {
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
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        self.baudrate_timer.tick(cpu_cycles);
    }

    pub fn write_tx_data(&mut self, tx_data: u32) {
        let tx_data = tx_data as u8;

        self.tx_fifo = match self.tx_fifo {
            TxFifoState::Empty | TxFifoState::Queued(_) => TxFifoState::Queued(tx_data),
            TxFifoState::Transferring {
                value,
                bits_remaining,
                ..
            } => TxFifoState::Transferring {
                value,
                bits_remaining,
                next: Some(tx_data),
            },
        };

        log::debug!("SIO0_TX_DATA write: {tx_data:02X}");
    }

    // $1F801044: SIO0_STAT
    pub fn read_status(&self) -> u32 {
        // TODO Bit 7: DSR input level (/ACK)
        // TODO Bit 9: IRQ
        let value = u32::from(self.tx_fifo.ready_for_new_byte())
            | (u32::from(!self.rx_fifo.empty()) << 1)
            | (u32::from(self.tx_fifo == TxFifoState::Empty) << 2)
            | (self.baudrate_timer.timer << 11);

        log::debug!("SIO0_STAT read: {value:08X}");
        value
    }

    // $1F801048: SIO0_MODE
    pub fn write_mode(&mut self, value: u32) {
        self.baudrate_timer.update_reload_factor(value);

        if value & 0xC != 0xC {
            todo!(
                "Expected character length to be 8-bit (3), was {}",
                (value >> 2) & 3
            );
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
            todo!("SIO0_CTRL ACK");
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
}
