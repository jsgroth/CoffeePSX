#[derive(Debug, Clone)]
pub struct Timer {
    pub counter: u16,
}

impl Timer {
    pub fn write_mode(&mut self, _value: u32) {
        // TODO actually configure timer
        self.counter = 0;
    }

    pub fn increment(&mut self) {
        self.counter = self.counter.wrapping_add(1);
    }
}

#[derive(Debug, Clone)]
pub struct Timers {
    // Horizontal retrace timer
    pub timer_1: Timer,
}

impl Timers {
    pub fn new() -> Self {
        Self {
            timer_1: Timer { counter: 0 },
        }
    }

    pub fn write_register(&mut self, address: u32, value: u32) {
        let timer_idx = (address >> 4) & 3;
        if timer_idx != 1 {
            log::warn!("Unhandled timer {timer_idx} write: {address:08X} {value:08X}");
            return;
        }

        match address & 0xF {
            0x0 => {
                self.timer_1.counter = value as u16;
                log::trace!("Timer 1 counter write: {:04X}", self.timer_1.counter);
            }
            0x4 => {
                self.timer_1.write_mode(value);
                log::trace!("Timer 1 mode write: {value:08X}");
            }
            0x8 => {
                log::warn!("Unhandled timer 1 target write: {value:08X}");
            }
            _ => todo!("timer register write {address:08X} {value:08X}"),
        }
    }
}
