use crate::num::I16Ext;
use bincode::{Decode, Encode};
use std::cmp;

#[derive(Debug, Clone, Encode, Decode)]
pub struct NoiseGenerator {
    pub output: i16,
    pub step: u8,
    pub shift: u8,
    timer: i32,
}

impl NoiseGenerator {
    pub fn new() -> Self {
        Self { output: 0, step: 0, shift: 0, timer: 0 }
    }

    pub fn clock(&mut self) {
        self.timer -= i32::from(self.step + 4);
        if self.timer >= 0 {
            return;
        }

        let parity = self.output.bit(15)
            ^ self.output.bit(12)
            ^ self.output.bit(11)
            ^ self.output.bit(10)
            ^ true;
        self.output = (self.output << 1) | i16::from(parity);

        while self.timer < 0 {
            self.timer += 0x20000 >> self.shift;
        }
    }

    pub fn write_shift(&mut self, shift: u8) {
        self.shift = shift;

        // Final Fantasy 7 has broken sound effects if the shift is not immediately applied to the timer
        self.timer = cmp::min(self.timer, 0x20000 >> shift);
    }
}
