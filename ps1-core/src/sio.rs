//! PS1 serial I/O port 0 (SIO0), used to communicate with controllers and memory cards

use crate::num::U32Ext;
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

#[derive(Debug, Clone)]
pub struct SerialPort {
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
    pub fn write_control(&mut self, value: u32) {
        if value.bit(6) {
            // Reset bit
            log::debug!("SIO0 reset");
            self.write_mode(0xC);
            self.write_control(0);
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
