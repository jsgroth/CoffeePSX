use crate::spu::envelope::{AdsrEnvelope, SweepEnvelope};

#[derive(Debug, Clone)]
pub struct Voice {
    pub volume_l: SweepEnvelope,
    pub volume_r: SweepEnvelope,
    pub sample_rate: u16,
    pub start_address: u32,
    pub repeat_address: u32,
    pub adsr: AdsrEnvelope,
}

impl Voice {
    pub fn new() -> Self {
        Self {
            volume_l: SweepEnvelope::new(),
            volume_r: SweepEnvelope::new(),
            sample_rate: 0,
            start_address: 0,
            repeat_address: 0,
            adsr: AdsrEnvelope::new(),
        }
    }

    pub fn clock(&mut self) {
        self.volume_l.clock();
        self.volume_r.clock();
        self.adsr.clock();
    }

    pub fn write_volume_l(&mut self, value: u32) {
        self.volume_l.write(value);
    }

    pub fn write_volume_r(&mut self, value: u32) {
        self.volume_r.write(value);
    }

    pub fn write_sample_rate(&mut self, value: u32) {
        self.sample_rate = value as u16;
    }

    pub fn write_start_address(&mut self, value: u32) {
        // Address is in 8-byte units
        self.start_address = (value & 0xFFFF) << 3;
    }

    pub fn write_adsr_low(&mut self, value: u32) {
        self.adsr.settings.write_low(value);
    }

    pub fn write_adsr_high(&mut self, value: u32) {
        self.adsr.settings.write_high(value);
    }

    pub fn read_adsr_level(&self) -> u32 {
        self.adsr.level as u32
    }

    pub fn key_on(&mut self) {
        self.adsr.key_on();

        // Keying on copies start address to repeat address
        self.repeat_address = self.start_address;
    }

    pub fn key_off(&mut self) {
        self.adsr.key_off();
    }
}
