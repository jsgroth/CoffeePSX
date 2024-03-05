use crate::num::U32Ext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeMode {
    #[default]
    Linear,
    Exponential,
}

impl EnvelopeMode {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Exponential
        } else {
            Self::Linear
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeDirection {
    #[default]
    Increasing,
    Decreasing,
}

impl EnvelopeDirection {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Decreasing
        } else {
            Self::Increasing
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvelopeSettings {
    pub step: u8,
    pub shift: u8,
    pub direction: EnvelopeDirection,
    pub mode: EnvelopeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepPhase {
    Positive,
    Negative,
}

impl SweepPhase {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Negative
        } else {
            Self::Positive
        }
    }
}

#[derive(Debug, Clone)]
pub enum VolumeSetting {
    Fixed(i16),
    Sweep(i16, EnvelopeSettings, SweepPhase),
}

impl Default for VolumeSetting {
    fn default() -> Self {
        Self::Fixed(0)
    }
}

impl VolumeSetting {
    pub fn current_volume(&self) -> i16 {
        match *self {
            Self::Fixed(volume) | Self::Sweep(volume, ..) => volume,
        }
    }

    pub fn write(&mut self, value: u32) {
        *self = if value.bit(15) {
            Self::parse_sweep(value, self.current_volume())
        } else {
            let volume = (value << 1) as i16;
            Self::Fixed(volume)
        };
    }

    fn parse_sweep(value: u32, initial_volume: i16) -> Self {
        let envelope_settings = EnvelopeSettings {
            step: (value & 3) as u8,
            shift: ((value >> 2) & 0x1F) as u8,
            direction: EnvelopeDirection::from_bit(value.bit(13)),
            mode: EnvelopeMode::from_bit(value.bit(14)),
        };
        let sweep_phase = SweepPhase::from_bit(value.bit(12));

        Self::Sweep(initial_volume, envelope_settings, sweep_phase)
    }
}

#[derive(Debug, Clone)]
pub struct VolumeControl {
    pub main_l: VolumeSetting,
    pub main_r: VolumeSetting,
    pub cd_l: i16,
    pub cd_r: i16,
}

impl VolumeControl {
    pub fn new() -> Self {
        Self {
            main_l: VolumeSetting::default(),
            main_r: VolumeSetting::default(),
            cd_l: 0,
            cd_r: 0,
        }
    }

    // $1F801D80: Main volume left
    pub fn write_main_l(&mut self, value: u32) {
        self.main_l.write(value);
        log::trace!("Main volume L write: {:?}", self.main_l);
    }

    // $1F801D82: Main volume right
    pub fn write_main_r(&mut self, value: u32) {
        self.main_r.write(value);
        log::trace!("Main volume R write: {:?}", self.main_r);
    }

    // $1F801DB0: CD volume left
    pub fn write_cd_l(&mut self, value: u32) {
        self.cd_l = value as i16;
        log::trace!("CD volume L write: {}", self.cd_l);
    }

    // $1F801DB2: CD volume right
    pub fn write_cd_r(&mut self, value: u32) {
        self.cd_r = value as i16;
        log::trace!("CD volume R write: {}", self.cd_r);
    }
}

#[derive(Debug, Clone)]
pub struct AdsrSettings {
    pub attack_step: u8,
    pub attack_shift: u8,
    pub attack_mode: EnvelopeMode,
    pub decay_shift: u8,
    pub sustain_level: u16,
    pub sustain_step: u8,
    pub sustain_shift: u8,
    pub sustain_direction: EnvelopeDirection,
    pub sustain_mode: EnvelopeMode,
    pub release_shift: u8,
    pub release_mode: EnvelopeMode,
}

impl AdsrSettings {
    pub fn new() -> Self {
        Self {
            attack_step: 0,
            attack_shift: 0,
            attack_mode: EnvelopeMode::default(),
            decay_shift: 0,
            sustain_level: parse_sustain_level(0),
            sustain_step: 0,
            sustain_shift: 0,
            sustain_direction: EnvelopeDirection::default(),
            sustain_mode: EnvelopeMode::default(),
            release_shift: 0,
            release_mode: EnvelopeMode::default(),
        }
    }

    // $1F801C08: ADSR settings, low halfword
    pub fn write_low(&mut self, value: u32) {
        self.attack_mode = EnvelopeMode::from_bit(value.bit(15));
        self.attack_shift = ((value >> 10) & 0x1F) as u8;
        self.attack_step = ((value >> 8) & 0x3) as u8;
        self.decay_shift = ((value >> 4) & 0x0F) as u8;
        self.sustain_level = parse_sustain_level(value & 0xF);
    }

    // $1F801C0A: ADSR settings, high halfword
    pub fn write_high(&mut self, value: u32) {
        self.sustain_mode = EnvelopeMode::from_bit(value.bit(15));
        self.sustain_direction = EnvelopeDirection::from_bit(value.bit(14));
        self.sustain_shift = ((value >> 8) & 0x1F) as u8;
        self.sustain_step = ((value >> 6) & 0x3) as u8;
        self.release_mode = EnvelopeMode::from_bit(value.bit(5));
        self.release_shift = (value & 0x1F) as u8;
    }
}

fn parse_sustain_level(value: u32) -> u16 {
    (((value & 0xF) + 1) << 11) as u16
}
