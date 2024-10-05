mod gaussian;

use crate::spu;
use crate::spu::adpcm::{AdpcmHeader, SpuAdpcmBuffer};
use crate::spu::envelope::{AdsrEnvelope, AdsrPhase, SweepEnvelope};
use crate::spu::{SoundRam, adpcm, multiply_volume};
use bincode::{Decode, Encode};
use std::cmp;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Voice {
    voice_number: usize,
    pub volume_l: SweepEnvelope,
    pub volume_r: SweepEnvelope,
    pub sample_rate: u16,
    pub noise_enabled: bool,
    pub pitch_modulation_enabled: bool,
    start_address: u32,
    repeat_address: u32,
    repeat_address_locked: bool,
    current_address: u32,
    restart_pending: bool,
    restart_delay: u8,
    release_pending: bool,
    keyed_on: bool,
    keyed_off: bool,
    soft_reset: bool,
    pub adsr: AdsrEnvelope,
    adpcm_buffer: SpuAdpcmBuffer,
    pitch_counter: u16,
    pub current_amplitude: i16,
    pub current_sample: (i16, i16),
}

impl Voice {
    pub fn new(voice_number: usize) -> Self {
        Self {
            voice_number,
            volume_l: SweepEnvelope::new(),
            volume_r: SweepEnvelope::new(),
            sample_rate: 0,
            noise_enabled: false,
            pitch_modulation_enabled: false,
            start_address: 0,
            repeat_address: 0,
            repeat_address_locked: false,
            current_address: 0,
            restart_pending: false,
            restart_delay: 0,
            release_pending: false,
            keyed_on: false,
            keyed_off: false,
            soft_reset: true,
            adsr: AdsrEnvelope::new(),
            adpcm_buffer: SpuAdpcmBuffer::new(),
            pitch_counter: 0,
            current_amplitude: 0,
            current_sample: (0, 0),
        }
    }

    pub fn clock(&mut self, sound_ram: &SoundRam, noise_output: i16, prev_voice_output: i16) {
        if self.restart_pending {
            self.restart_pending = false;
            self.restart(sound_ram);
            self.current_sample = (0, 0);
            return;
        }

        if self.restart_delay != 0 {
            self.restart_delay -= 1;
            self.current_sample = (0, 0);

            // Not sure this is right but this is how the SNES APU behaves - key off and soft reset
            // flags prevent the voice from starting ADSR, but only if they're still set 2 clocks after
            // key on
            if self.restart_delay <= 3 && (self.release_pending || self.soft_reset) {
                log::debug!(
                    "Voice {} is not starting; key_off={} soft_reset={}",
                    self.voice_number,
                    self.keyed_off,
                    self.soft_reset
                );
                self.adsr.key_off();
            }

            self.release_pending = false;

            return;
        }

        if self.release_pending {
            self.release_pending = false;
            self.adsr.key_off();
        }

        self.volume_l.clock();
        self.volume_r.clock();
        self.adsr.clock();

        let pitch_counter_step = if self.pitch_modulation_enabled {
            apply_pitch_modulation(self.sample_rate, prev_voice_output)
        } else {
            self.sample_rate
        };

        // Step cannot be larger than $4000 (4 * 44100 Hz)
        let pitch_counter_step = cmp::min(0x4000, pitch_counter_step);

        self.pitch_counter += pitch_counter_step;
        while self.pitch_counter >= 0x1000 {
            self.pitch_counter -= 0x1000;
            self.adpcm_buffer.advance();
            if self.adpcm_buffer.at_end_of_block() {
                self.decode_adpcm_block(sound_ram);
            }
        }

        self.sample(noise_output);
    }

    fn sample(&mut self, noise_output: i16) {
        let raw_sample = if !self.noise_enabled {
            gaussian::interpolate(self.adpcm_buffer.four_most_recent_samples(), self.pitch_counter)
        } else {
            noise_output
        };
        let sample = multiply_volume(raw_sample, self.adsr.level);
        self.current_amplitude = sample;

        let sample_l = multiply_volume(sample, self.volume_l.volume);
        let sample_r = multiply_volume(sample, self.volume_r.volume);

        self.current_sample = (sample_l, sample_r);
    }

    pub fn read_start_address(&self) -> u32 {
        self.start_address >> 3
    }

    pub fn write_start_address(&mut self, value: u32) {
        // Address is in 8-byte units
        self.start_address = (value & 0xFFFF) << 3;
    }

    pub fn read_adsr_level(&self) -> u32 {
        (self.adsr.level as u16).into()
    }

    pub fn read_repeat_address(&self) -> u32 {
        self.repeat_address >> 3
    }

    pub fn write_repeat_address(&mut self, value: u32) {
        self.repeat_address = (value & 0xFFFF) << 3;

        // Writing to the repeat address register prevents ADPCM loop start addresses from having
        // any effect until the next key on; The Misadventures of Tron Bonne depends on this or
        // it will freeze during certain cutscenes
        //
        // However, repeat address register writes do _not_ have this effect if they occur
        // while the voice is restarting; Re-Loaded depends on this or music/sound will be glitched
        self.repeat_address_locked |= self.restart_delay == 0;
    }

    pub fn is_keyed_on(&self) -> bool {
        self.keyed_on
    }

    pub fn update_key_on(&mut self, keyed_on: bool) {
        self.keyed_on = keyed_on;

        if keyed_on {
            self.restart_pending = true;
            log::debug!("Keying on voice {}", self.voice_number);
        }
    }

    pub fn is_keyed_off(&self) -> bool {
        self.keyed_off
    }

    pub fn update_key_off(&mut self, keyed_off: bool) {
        self.keyed_off = keyed_off;

        if keyed_off {
            self.release_pending = true;
            log::debug!("Keying off voice {}", self.voice_number);
        }
    }

    fn restart(&mut self, sound_ram: &SoundRam) {
        // Not sure this is right but the SNES APU plays 5 empty samples after a restart
        self.restart_delay = 5;

        self.repeat_address_locked = false;

        self.adsr.key_on();

        self.current_address = self.start_address;

        // Immediately decode first ADPCM block and reset ADPCM state
        self.adpcm_buffer.reset();
        self.pitch_counter = 0;
        self.decode_adpcm_block(sound_ram);
    }

    fn decode_adpcm_block(&mut self, sound_ram: &SoundRam) {
        // TODO this can wrap since address is in 8-byte units
        let block = &sound_ram[self.current_address as usize..(self.current_address + 16) as usize];
        adpcm::decode_spu_block(block, &mut self.adpcm_buffer);

        let AdpcmHeader { loop_start, loop_end, loop_repeat, .. } = self.adpcm_buffer.header;
        if loop_start && !self.repeat_address_locked {
            self.repeat_address = self.current_address;
        }

        if loop_end {
            self.current_address = self.repeat_address;
            if !loop_repeat {
                self.adsr.phase = AdsrPhase::Release;
                self.adsr.level = 0;
            }
        } else {
            self.current_address = (self.current_address + 16) & spu::SOUND_RAM_MASK;
        }
    }

    pub fn update_soft_reset(&mut self, soft_reset: bool) {
        self.soft_reset = soft_reset;

        if soft_reset {
            self.adsr.level = 0;
            self.adsr.key_off();
        }
    }
}

fn apply_pitch_modulation(sample_rate: u16, prev_voice_output: i16) -> u16 {
    let factor = i32::from(prev_voice_output) + 0x8000;

    // Hardware glitch: Sample rates greater than $7FFF are sign extended to 32 bits
    let step: i32 = (sample_rate as i16).into();
    let step = (step * factor) >> 15;

    // Hardware glitch (when sample rate greater than $7FFF): Sign is dropped
    step as u16
}
