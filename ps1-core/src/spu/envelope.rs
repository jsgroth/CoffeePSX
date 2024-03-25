use crate::num::U32Ext;
use bincode::{Decode, Encode};
use std::cmp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum EnvelopeMode {
    #[default]
    Linear = 0,
    Exponential = 1,
}

impl EnvelopeMode {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Exponential } else { Self::Linear }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum EnvelopeDirection {
    #[default]
    Increasing = 0,
    Decreasing = 1,
}

impl EnvelopeDirection {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Decreasing } else { Self::Increasing }
    }
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct EnvelopeSettings {
    pub step: u8,
    pub shift: u8,
    pub direction: EnvelopeDirection,
    pub mode: EnvelopeMode,
}

impl EnvelopeSettings {
    pub fn wait_cycles(self, current_volume_magnitude: u16) -> u32 {
        let cycles = 1 << self.shift.saturating_sub(11);

        // Exponential increase is faked by increasing volume at a 4x slower rate when volume is
        // greater than $6000 out of $7FFF
        if self.mode == EnvelopeMode::Exponential
            && self.direction == EnvelopeDirection::Increasing
            && current_volume_magnitude > 0x6000
        {
            4 * cycles
        } else {
            cycles
        }
    }

    pub fn next_step(self, current_volume_magnitude: u16) -> i16 {
        let base_step = match self.direction {
            // 0-3 maps to +7 to +4
            EnvelopeDirection::Increasing => i32::from(7 - self.step),
            // 0-3 maps to -8 to -5
            EnvelopeDirection::Decreasing => -i32::from(8 - self.step),
        };
        let step = base_step << 11_u8.saturating_sub(self.shift);

        if self.mode == EnvelopeMode::Exponential && self.direction == EnvelopeDirection::Decreasing
        {
            ((step * i32::from(current_volume_magnitude)) >> 15) as i16
        } else {
            step as i16
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SweepPhase {
    Positive = 0,
    Negative = 1,
}

impl SweepPhase {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Negative } else { Self::Positive }
    }
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub enum SweepSetting {
    Fixed,
    Sweep(EnvelopeSettings, SweepPhase),
}

impl Default for SweepSetting {
    fn default() -> Self {
        Self::Fixed
    }
}

impl SweepSetting {
    fn parse(value: u32) -> Self {
        if !value.bit(15) {
            return Self::Fixed;
        }

        let envelope_settings = EnvelopeSettings {
            step: (value & 3) as u8,
            shift: ((value >> 2) & 0x1F) as u8,
            direction: EnvelopeDirection::from_bit(value.bit(13)),
            mode: EnvelopeMode::from_bit(value.bit(14)),
        };
        let sweep_phase = SweepPhase::from_bit(value.bit(12));

        Self::Sweep(envelope_settings, sweep_phase)
    }
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct SweepEnvelope {
    pub volume: i16,
    pub setting: SweepSetting,
    pub wait_cycles_remaining: u32,
    pub next_step: i16,
}

impl SweepEnvelope {
    pub fn new() -> Self {
        Self { volume: 0, setting: SweepSetting::default(), wait_cycles_remaining: 1, next_step: 0 }
    }

    pub fn write(&mut self, value: u32) {
        self.setting = SweepSetting::parse(value);

        // Writing a fixed volume (bit 15 = 0) also sets current volume
        if !value.bit(15) {
            self.volume = (value << 1) as i16;
        }

        self.wait_cycles_remaining = 1;
        self.next_step = 0;
    }

    pub fn read(&self) -> u32 {
        match self.setting {
            SweepSetting::Fixed => u32::from(self.volume as u16) >> 1,
            SweepSetting::Sweep(envelope, phase) => {
                (1 << 15)
                    | ((envelope.mode as u32) << 14)
                    | ((envelope.direction as u32) << 13)
                    | ((phase as u32) << 12)
                    | (u32::from(envelope.shift) << 2)
                    | u32::from(envelope.step)
            }
        }
    }

    pub fn clock(&mut self) {
        let SweepSetting::Sweep(envelope_settings, sweep_phase) = self.setting else {
            return;
        };

        self.wait_cycles_remaining -= 1;
        if self.wait_cycles_remaining == 0 {
            self.volume = match sweep_phase {
                SweepPhase::Positive => {
                    (i32::from(self.volume) + i32::from(self.next_step)).clamp(0, 0x7FFF) as i16
                }
                SweepPhase::Negative => {
                    (i32::from(self.volume) - i32::from(self.next_step)).clamp(-0x7FFF, 0) as i16
                }
            };

            let current_volume_magnitude = self.volume.saturating_abs() as u16;
            self.wait_cycles_remaining = envelope_settings.wait_cycles(current_volume_magnitude);
            self.next_step = envelope_settings.next_step(current_volume_magnitude);
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct VolumeControl {
    pub main_l: SweepEnvelope,
    pub main_r: SweepEnvelope,
    pub cd_l: i16,
    pub cd_r: i16,
}

impl VolumeControl {
    pub fn new() -> Self {
        Self { main_l: SweepEnvelope::new(), main_r: SweepEnvelope::new(), cd_l: 0, cd_r: 0 }
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

#[derive(Debug, Clone, Encode, Decode)]
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

    // $1F801C08 + N*$10: ADSR settings, low halfword
    pub fn write_low(&mut self, value: u32) {
        self.attack_mode = EnvelopeMode::from_bit(value.bit(15));
        self.attack_shift = ((value >> 10) & 0x1F) as u8;
        self.attack_step = ((value >> 8) & 0x3) as u8;
        self.decay_shift = ((value >> 4) & 0x0F) as u8;
        self.sustain_level = parse_sustain_level(value & 0xF);
    }

    pub fn read_low(&self) -> u32 {
        reverse_sustain_level(self.sustain_level)
            | (u32::from(self.decay_shift) << 4)
            | (u32::from(self.attack_step) << 8)
            | (u32::from(self.attack_shift) << 10)
            | ((self.attack_mode as u32) << 15)
    }

    // $1F801C0A + N*$10: ADSR settings, high halfword
    pub fn write_high(&mut self, value: u32) {
        self.sustain_mode = EnvelopeMode::from_bit(value.bit(15));
        self.sustain_direction = EnvelopeDirection::from_bit(value.bit(14));
        self.sustain_shift = ((value >> 8) & 0x1F) as u8;
        self.sustain_step = ((value >> 6) & 0x3) as u8;
        self.release_mode = EnvelopeMode::from_bit(value.bit(5));
        self.release_shift = (value & 0x1F) as u8;
    }

    pub fn read_high(&self) -> u32 {
        u32::from(self.release_shift)
            | ((self.release_mode as u32) << 5)
            | (u32::from(self.sustain_step) << 6)
            | (u32::from(self.sustain_shift) << 8)
            | ((self.sustain_direction as u32) << 14)
            | ((self.sustain_mode as u32) << 15)
    }
}

fn parse_sustain_level(value: u32) -> u16 {
    (((value & 0xF) + 1) << 11) as u16
}

fn reverse_sustain_level(value: u16) -> u32 {
    (u32::from(value) >> 11) - 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum AdsrState {
    Attack,
    Decay,
    Sustain,
    #[default]
    Release,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct AdsrEnvelope {
    pub level: i16,
    pub settings: AdsrSettings,
    pub state: AdsrState,
    pub wait_cycles_remaining: u32,
    pub next_step: i16,
}

impl AdsrEnvelope {
    pub fn new() -> Self {
        Self {
            level: 0,
            settings: AdsrSettings::new(),
            state: AdsrState::default(),
            wait_cycles_remaining: 1,
            next_step: 0,
        }
    }

    pub fn clock(&mut self) {
        self.wait_cycles_remaining -= 1;
        if self.wait_cycles_remaining != 0 {
            return;
        }

        let min_level = match self.state {
            AdsrState::Decay => cmp::min(i16::MAX as u16, self.settings.sustain_level) as i16,
            AdsrState::Attack | AdsrState::Sustain | AdsrState::Release => 0,
        };

        self.level = (i32::from(self.level) + i32::from(self.next_step))
            .clamp(min_level.into(), i16::MAX.into()) as i16;

        if self.state == AdsrState::Attack && self.level == i16::MAX {
            self.state = AdsrState::Decay;
        }

        if self.state == AdsrState::Decay && self.level == min_level {
            self.state = AdsrState::Sustain;
        }

        let envelope_settings = match self.state {
            AdsrState::Attack => EnvelopeSettings {
                step: self.settings.attack_step,
                shift: self.settings.attack_shift,
                direction: EnvelopeDirection::Increasing,
                mode: self.settings.attack_mode,
            },
            AdsrState::Decay => EnvelopeSettings {
                step: 0,
                shift: self.settings.decay_shift,
                direction: EnvelopeDirection::Decreasing,
                mode: EnvelopeMode::Exponential,
            },
            AdsrState::Sustain => EnvelopeSettings {
                step: self.settings.sustain_step,
                shift: self.settings.sustain_shift,
                direction: self.settings.sustain_direction,
                mode: self.settings.sustain_mode,
            },
            AdsrState::Release => EnvelopeSettings {
                step: 0,
                shift: self.settings.release_shift,
                direction: EnvelopeDirection::Decreasing,
                mode: self.settings.release_mode,
            },
        };
        self.wait_cycles_remaining = envelope_settings.wait_cycles(self.level as u16);
        self.next_step = envelope_settings.next_step(self.level as u16);
    }

    pub fn key_on(&mut self) {
        self.state = AdsrState::Attack;
        self.level = 0;

        self.wait_cycles_remaining = 1;
        self.next_step = 0;
    }

    pub fn key_off(&mut self) {
        self.state = AdsrState::Release;

        self.wait_cycles_remaining = 1;
        self.next_step = 0;
    }
}
