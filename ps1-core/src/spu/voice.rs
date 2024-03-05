use crate::spu::envelope::{AdsrSettings, VolumeSetting};

#[derive(Debug, Clone)]
pub struct Voice {
    pub volume_l: VolumeSetting,
    pub volume_r: VolumeSetting,
    pub sample_rate: u16,
    pub start_address: u32,
    pub adsr: AdsrSettings,
}

impl Voice {
    pub fn new() -> Self {
        Self {
            volume_l: VolumeSetting::default(),
            volume_r: VolumeSetting::default(),
            sample_rate: 0,
            start_address: 0,
            adsr: AdsrSettings::new(),
        }
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

    #[allow(clippy::unused_self)]
    pub fn key_on(&mut self) {
        // TODO set ADSR state to attack, set ADSR level to 0, and copy start address to repeat address
    }

    #[allow(clippy::unused_self)]
    pub fn key_off(&mut self) {
        // TODO set ADSR state to release
    }
}
