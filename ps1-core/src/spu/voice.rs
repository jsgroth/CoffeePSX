mod gaussian;

use crate::spu;
use crate::spu::adpcm::{AdpcmHeader, SpuAdpcmBuffer};
use crate::spu::envelope::{AdsrEnvelope, AdsrState, SweepEnvelope};
use crate::spu::{adpcm, multiply_volume, AudioRam};
use bincode::{Decode, Encode};
use std::cmp;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Voice {
    pub volume_l: SweepEnvelope,
    pub volume_r: SweepEnvelope,
    pub sample_rate: u16,
    pub start_address: u32,
    pub repeat_address: u32,
    current_address: u32,
    pub adsr: AdsrEnvelope,
    adpcm_buffer: SpuAdpcmBuffer,
    pitch_counter: u16,
    pub current_sample: (i16, i16),
}

impl Voice {
    pub fn new() -> Self {
        Self {
            volume_l: SweepEnvelope::new(),
            volume_r: SweepEnvelope::new(),
            sample_rate: 0,
            start_address: 0,
            repeat_address: 0,
            current_address: 0,
            adsr: AdsrEnvelope::new(),
            adpcm_buffer: SpuAdpcmBuffer::new(),
            pitch_counter: 0,
            current_sample: (0, 0),
        }
    }

    pub fn clock(&mut self, audio_ram: &AudioRam) {
        self.volume_l.clock();
        self.volume_r.clock();
        self.adsr.clock();

        // Pitch counter cannot step at greater than $4000 per clock (4 * 44100 Hz)
        // TODO pitch modulation
        let pitch_counter_step = cmp::min(0x4000, self.sample_rate);
        self.pitch_counter += pitch_counter_step;
        while self.pitch_counter >= 0x1000 {
            self.pitch_counter -= 0x1000;
            self.adpcm_buffer.advance();
            if self.adpcm_buffer.at_end_of_block() {
                self.decode_adpcm_block(audio_ram);
            }
        }

        self.current_sample = self.sample();
    }

    fn sample(&self) -> (i16, i16) {
        let raw_sample =
            gaussian::interpolate(self.adpcm_buffer.four_most_recent_samples(), self.pitch_counter);
        let sample = multiply_volume(raw_sample, self.adsr.level);

        let sample_l = multiply_volume(sample, self.volume_l.volume);
        let sample_r = multiply_volume(sample, self.volume_r.volume);

        (sample_l, sample_r)
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

    pub fn write_repeat_address(&mut self, value: u32) {
        self.repeat_address = (value & 0xFFFF) << 3;
    }

    pub fn key_on(&mut self, audio_ram: &AudioRam) {
        self.adsr.key_on();

        // Keying on copies start address to repeat address
        self.repeat_address = self.start_address;
        self.current_address = self.start_address;

        // Immediately decode first ADPCM block and reset ADPCM state
        self.adpcm_buffer.reset();
        self.pitch_counter = 0;
        self.decode_adpcm_block(audio_ram);
    }

    fn decode_adpcm_block(&mut self, audio_ram: &AudioRam) {
        // TODO this can wrap since address is in 8-byte units
        let block = &audio_ram[self.current_address as usize..(self.current_address + 16) as usize];
        adpcm::decode_spu_block(block, &mut self.adpcm_buffer);

        let AdpcmHeader { loop_start, loop_end, loop_repeat, .. } = self.adpcm_buffer.header;
        if loop_start {
            self.repeat_address = self.current_address;
        }

        if loop_end {
            self.current_address = self.repeat_address;
            if !loop_repeat {
                self.adsr.state = AdsrState::Release;
                self.adsr.level = 0;
            }
        } else {
            self.current_address = (self.current_address + 16) & spu::AUDIO_RAM_MASK;
        }
    }

    pub fn key_off(&mut self) {
        self.adsr.key_off();
    }
}
