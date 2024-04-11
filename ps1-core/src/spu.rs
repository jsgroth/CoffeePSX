//! PS1 SPU (Sound Processing Unit)
//!
//! The SPU is a 24-channel ADPCM playback chip with 512KB of sound RAM.

pub mod adpcm;
mod envelope;
mod noise;
mod reverb;
mod voice;

use crate::cd::CdController;
use crate::cpu::OpSize;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::num::U32Ext;
use crate::spu::envelope::VolumeControl;
use crate::spu::noise::NoiseGenerator;
use crate::spu::reverb::ReverbUnit;
use crate::spu::voice::Voice;
use bincode::{Decode, Encode};
use std::array;
use std::cell::Cell;
use std::ops::{Index, IndexMut, Range};

const SOUND_RAM_LEN: usize = 512 * 1024;
const SOUND_RAM_MASK: u32 = (SOUND_RAM_LEN - 1) as u32;

const NUM_VOICES: usize = 24;

#[derive(Debug, Clone, Encode, Decode)]
struct SoundRam {
    ram: Box<[u8; SOUND_RAM_LEN]>,
    irq_enabled: bool,
    irq_address: usize,
    irq: Cell<bool>,
}

impl SoundRam {
    fn new() -> Self {
        Self {
            ram: vec![0; SOUND_RAM_LEN].into_boxed_slice().try_into().unwrap(),
            irq_enabled: false,
            irq_address: 0,
            irq: Cell::new(false),
        }
    }

    fn read_irq_address(&self) -> u32 {
        (self.irq_address >> 3) as u32
    }

    fn write_irq_address(&mut self, value: u32) {
        self.irq_address = (value << 3) as usize;

        log::debug!("SPU IRQ address: {:05X}", self.irq_address);
    }
}

impl Index<usize> for SoundRam {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        if self.irq_enabled && index == self.irq_address {
            log::debug!("SPU IRQ set");
            self.irq.set(true);
        }

        &self.ram[index]
    }
}

impl Index<Range<usize>> for SoundRam {
    type Output = [u8];

    fn index(&self, index: Range<usize>) -> &Self::Output {
        if self.irq_enabled && index.contains(&self.irq_address) {
            log::debug!("SPU IRQ set");
            self.irq.set(true);
        }

        &self.ram[index]
    }
}

impl IndexMut<usize> for SoundRam {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        if self.irq_enabled && index == self.irq_address {
            log::debug!("SPU IRQ set");
            self.irq.set(true);
        }

        &mut self.ram[index]
    }
}

trait I32Ext {
    fn clamp_to_i16(self) -> i16;
}

impl I32Ext for i32 {
    fn clamp_to_i16(self) -> i16 {
        self.clamp(i16::MIN.into(), i16::MAX.into()) as i16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum DataPortMode {
    #[default]
    Off = 0,
    ManualWrite = 1,
    DmaWrite = 2,
    DmaRead = 3,
}

impl DataPortMode {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Off,
            1 => Self::ManualWrite,
            2 => Self::DmaWrite,
            3 => Self::DmaRead,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }

    const fn is_dma(self) -> bool {
        matches!(self, Self::DmaWrite | Self::DmaRead)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
struct DataPort {
    mode: DataPortMode,
    start_address: u32,
    current_address: u32,
}

impl DataPort {
    fn new() -> Self {
        Self { mode: DataPortMode::default(), start_address: 0, current_address: 0 }
    }

    fn read_start_address(&self) -> u32 {
        self.start_address >> 3
    }

    // $1F801DA6: Sound RAM data transfer address
    fn write_transfer_address(&mut self, value: u32) {
        // Address is in 8-byte units
        // Writing start address also sets an internal current address register
        self.start_address = (value & 0xFFFF) << 3;
        self.current_address = self.start_address;

        log::debug!("Sound RAM data transfer address: {:05X}", self.start_address);
    }
}

#[derive(Debug, Clone, Encode, Decode)]
struct ControlRegisters {
    spu_enabled: bool,
    amplifier_enabled: bool,
    external_audio_enabled: bool,
    cd_audio_enabled: bool,
    external_audio_reverb_enabled: bool,
    cd_audio_reverb_enabled: bool,
    // Recorded in case software reads the KON or KOFF registers
    last_key_on_write: u32,
    last_key_off_write: u32,
}

impl ControlRegisters {
    fn new() -> Self {
        Self {
            spu_enabled: false,
            amplifier_enabled: false,
            external_audio_enabled: false,
            cd_audio_enabled: false,
            external_audio_reverb_enabled: false,
            cd_audio_reverb_enabled: false,
            last_key_on_write: 0,
            last_key_off_write: 0,
        }
    }

    // $1F801DAA: SPU control register (SPUCNT)
    fn read_spucnt(
        &self,
        sound_ram: &SoundRam,
        data_port: &DataPort,
        reverb: &ReverbUnit,
        noise: &NoiseGenerator,
    ) -> u32 {
        (u32::from(self.spu_enabled) << 15)
            | (u32::from(self.amplifier_enabled) << 14)
            | (u32::from(noise.shift) << 10)
            | (u32::from(noise.step) << 8)
            | (u32::from(reverb.writes_enabled) << 7)
            | (u32::from(sound_ram.irq_enabled) << 6)
            | ((data_port.mode as u32) << 4)
            | (u32::from(self.external_audio_reverb_enabled) << 3)
            | (u32::from(reverb.cd_enabled) << 2)
            | (u32::from(self.external_audio_enabled) << 1)
            | u32::from(self.cd_audio_enabled)
    }

    // $1F801DAA: SPU control register (SPUCNT)
    fn write_spucnt(
        &mut self,
        value: u32,
        sound_ram: &mut SoundRam,
        data_port: &mut DataPort,
        reverb: &mut ReverbUnit,
        noise: &mut NoiseGenerator,
    ) {
        self.spu_enabled = value.bit(15);
        self.amplifier_enabled = value.bit(14);
        noise.write_shift(((value >> 10) & 0xF) as u8);
        noise.step = ((value >> 8) & 3) as u8;
        reverb.writes_enabled = value.bit(7);
        sound_ram.irq_enabled = value.bit(6);
        data_port.mode = DataPortMode::from_bits(value >> 4);
        self.external_audio_reverb_enabled = value.bit(3);
        reverb.cd_enabled = value.bit(2);
        self.external_audio_enabled = value.bit(1);
        self.cd_audio_enabled = value.bit(0);

        if !sound_ram.irq_enabled {
            sound_ram.irq.set(false);
        }

        log::debug!("SPUCNT write");
        log::debug!("  SPU enabled: {}", self.spu_enabled);
        log::debug!("  Amplifier enabled: {}", self.amplifier_enabled);
        log::debug!("  Noise shift: {}", noise.shift);
        log::debug!("  Noise step: {}", noise.step + 4);
        log::debug!("  Reverb writes enabled: {}", reverb.writes_enabled);
        log::debug!("  IRQ enabled: {}", sound_ram.irq_enabled);
        log::debug!("  Data port mode: {:?}", data_port.mode);
        log::debug!("  External audio reverb enabled: {}", self.external_audio_reverb_enabled);
        log::debug!("  CD audio reverb enabled: {}", reverb.cd_enabled);
        log::debug!("  External audio enabled: {}", self.external_audio_enabled);
        log::debug!("  CD audio enabled: {}", self.cd_audio_enabled);
    }

    fn record_kon_low_write(&mut self, value: u32) {
        self.last_key_on_write = (self.last_key_on_write & !0xFFFF) | (value & 0xFFFF);
    }

    fn record_kon_high_write(&mut self, value: u32) {
        self.last_key_on_write = (self.last_key_on_write & 0xFFFF) | (value << 16);
    }

    fn record_koff_low_write(&mut self, value: u32) {
        self.last_key_off_write = (self.last_key_off_write & !0xFFFF) | (value & 0xFFFF);
    }

    fn record_koff_high_write(&mut self, value: u32) {
        self.last_key_off_write = (self.last_key_off_write & 0xFFFF) | (value << 16);
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct Spu {
    sound_ram: SoundRam,
    voices: [Voice; NUM_VOICES],
    control: ControlRegisters,
    volume: VolumeControl,
    data_port: DataPort,
    reverb: ReverbUnit,
    noise: NoiseGenerator,
    last_irq_bit: bool,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            sound_ram: SoundRam::new(),
            voices: array::from_fn(|_| Voice::new()),
            control: ControlRegisters::new(),
            volume: VolumeControl::new(),
            data_port: DataPort::new(),
            reverb: ReverbUnit::default(),
            noise: NoiseGenerator::new(),
            last_irq_bit: false,
        }
    }

    pub fn clock(
        &mut self,
        cd_controller: &CdController,
        interrupt_registers: &mut InterruptRegisters,
    ) -> (f64, f64) {
        self.volume.main_l.clock();
        self.volume.main_r.clock();
        self.noise.clock();

        let mut prev_voice_output = 0;
        for voice in &mut self.voices {
            voice.clock(&self.sound_ram, self.noise.output, prev_voice_output);
            prev_voice_output = voice.current_amplitude;
        }

        // Grab current CD audio samples
        let (cd_l, cd_r) = if self.control.cd_audio_enabled {
            apply_volume_matrix(
                cd_controller.current_audio_sample(),
                cd_controller.spu_volume_matrix(),
            )
        } else {
            (0, 0)
        };

        self.reverb.clock(&self.voices, (cd_l, cd_r), &mut self.sound_ram);

        // Mix voice samples together
        let mut sample_l = 0;
        let mut sample_r = 0;
        for voice in &self.voices {
            let (voice_sample_l, voice_sample_r) = voice.current_sample;
            sample_l += i32::from(voice_sample_l);
            sample_r += i32::from(voice_sample_r);
        }

        // Apply main volume
        let sample_l = multiply_volume(sample_l.clamp_to_i16(), self.volume.main_l.volume);
        let sample_r = multiply_volume(sample_r.clamp_to_i16(), self.volume.main_r.volume);

        // Mix in reverb unit output
        let sample_l =
            (i32::from(sample_l) + i32::from(self.reverb.current_output.0)).clamp_to_i16();
        let sample_r =
            (i32::from(sample_r) + i32::from(self.reverb.current_output.1)).clamp_to_i16();

        let cd_l = multiply_volume(cd_l, self.volume.cd_l);
        let cd_r = multiply_volume(cd_r, self.volume.cd_r);
        let sample_l = (i32::from(sample_l) + i32::from(cd_l)).clamp_to_i16();
        let sample_r = (i32::from(sample_r) + i32::from(cd_r)).clamp_to_i16();

        // Convert from i16 to f64
        let sample_l = f64::from(sample_l) / -f64::from(i16::MIN);
        let sample_r = f64::from(sample_r) / -f64::from(i16::MIN);

        let sound_ram_irq = self.sound_ram.irq.get();
        if !self.last_irq_bit && sound_ram_irq {
            interrupt_registers.set_interrupt_flag(InterruptType::Spu);
        }
        self.last_irq_bit = sound_ram_irq;

        (sample_l, sample_r)
    }

    pub fn read_register(&mut self, address: u32, size: OpSize) -> u32 {
        log::trace!("SPU register read: {address:08X}");

        if size == OpSize::Word {
            let low_halfword = self.read_register(address, OpSize::HalfWord);
            let high_halfword = self.read_register(address | 2, OpSize::HalfWord);
            return low_halfword | (high_halfword << 16);
        }

        let value = match address & 0xFFFE {
            0x1C00..=0x1D7F => self.read_voice_register(address),
            0x1D80 => self.volume.main_l.read(),
            0x1D82 => self.volume.main_r.read(),
            0x1D84 => self.reverb.read_output_volume_l(),
            0x1D86 => self.reverb.read_output_volume_r(),
            // KON/KOFF are normally write-only, but reads return the last written value
            0x1D88 => self.control.last_key_on_write & 0xFFFF,
            0x1D8A => self.control.last_key_on_write >> 16,
            0x1D8C => self.control.last_key_off_write & 0xFFFF,
            0x1D8E => self.control.last_key_off_write >> 16,
            0x1D90 => self.reduce_voices_low(|voice| voice.pitch_modulation_enabled),
            0x1D92 => self.reduce_voices_high(|voice| voice.pitch_modulation_enabled),
            0x1D94 => self.reduce_voices_low(|voice| voice.noise_enabled),
            0x1D96 => self.reduce_voices_high(|voice| voice.noise_enabled),
            0x1D98 => self.reverb.read_reverb_on_low(),
            0x1D9A => self.reverb.read_reverb_on_high(),
            0x1D9C => todo!("ENDX voices 0-15 read"),
            0x1D9E => todo!("ENDX voices 16-23 read"),
            0x1DA2 => self.reverb.read_buffer_start_address(),
            0x1DA4 => self.sound_ram.read_irq_address(),
            0x1DA6 => self.data_port.read_start_address(),
            0x1DA8 => todo!("SPU data port read"),
            0x1DAA => self.control.read_spucnt(
                &self.sound_ram,
                &self.data_port,
                &self.reverb,
                &self.noise,
            ),
            // TODO return an actual value for sound RAM data transfer control?
            0x1DAC => 0x0004,
            0x1DAE => self.read_status_register(),
            0x1DB0 => (self.volume.cd_l as u16).into(),
            0x1DB2 => (self.volume.cd_r as u16).into(),
            0x1DB4 => {
                log::warn!("External audio volume L read");
                0
            }
            0x1DB6 => {
                log::warn!("External audio volume R read");
                0
            }
            0x1DB8 => (self.volume.main_l.volume as u16).into(),
            0x1DBA => (self.volume.main_r.volume as u16).into(),
            0x1E00..=0x1E5F => {
                // Current voice volume
                let voice = (address & 0xFF) >> 2;
                if !address.bit(1) {
                    (self.voices[voice as usize].volume_l.volume as u16).into()
                } else {
                    (self.voices[voice as usize].volume_r.volume as u16).into()
                }
            }
            _ => todo!("SPU read register {address:08X}"),
        };

        match size {
            OpSize::Byte => {
                if !address.bit(0) {
                    value & 0xFF
                } else {
                    (value >> 8) & 0xFF
                }
            }
            OpSize::HalfWord => value,
            OpSize::Word => unreachable!("size Word should have early returned"),
        }
    }

    fn reduce_voices_low(&self, f: impl Fn(&Voice) -> bool) -> u32 {
        (0..16).map(|i| u32::from(f(&self.voices[i])) << i).reduce(|a, b| a | b).unwrap()
    }

    fn reduce_voices_high(&self, f: impl Fn(&Voice) -> bool) -> u32 {
        (16..24).map(|i| u32::from(f(&self.voices[i])) << (i - 16)).reduce(|a, b| a | b).unwrap()
    }

    pub fn write_register(&mut self, address: u32, value: u32, size: OpSize) {
        log::trace!("SPU register write: {address:08X} {value:08X} {size:?}");

        match size {
            OpSize::Byte => {
                if address.bit(0) {
                    // 8-bit writes to odd addresses do nothing
                    return;
                }
            }
            OpSize::HalfWord => {}
            OpSize::Word => {
                // Split 32-bit writes into a pair of 16-bit writes
                // 32-bit writes are apparently somewhat unstable on actual hardware; not emulating that
                self.write_register(address, value & 0xFFFF, OpSize::HalfWord);
                self.write_register(address | 2, value >> 16, OpSize::HalfWord);
                return;
            }
        }

        // Only write lowest 16 bits
        // 8-bit writes to even addresses also seem to write the lowest 16 bits from the source
        // register, as opposed to only writing the lowest 8 bits
        let value = value & 0xFFFF;

        match address & 0xFFFF {
            0x1C00..=0x1D7F => self.write_voice_register(address, value),
            0x1D80 => self.volume.write_main_l(value),
            0x1D82 => self.volume.write_main_r(value),
            0x1D84 => self.reverb.write_output_volume_l(value),
            0x1D86 => self.reverb.write_output_volume_r(value),
            0x1D88 => self.key_on_low(value),
            0x1D8A => self.key_on_high(value),
            0x1D8C => self.key_off_low(value),
            0x1D8E => self.key_off_high(value),
            0x1D90 => self.write_pitch_modulation_low(value),
            0x1D92 => self.write_pitch_modulation_high(value),
            0x1D94 => self.write_noise_low(value),
            0x1D96 => self.write_noise_high(value),
            0x1D98 => self.reverb.write_reverb_on_low(value),
            0x1D9A => self.reverb.write_reverb_on_high(value),
            0x1D9C => {
                log::warn!("ENDX write (voices 0-15): {value:04X}");
            }
            0x1D9E => {
                log::warn!("ENDX write (voices 16-23): {value:04X}");
            }
            0x1DA2 => self.reverb.write_buffer_start_address(value),
            0x1DA4 => self.sound_ram.write_irq_address(value),
            0x1DA6 => self.data_port.write_transfer_address(value),
            0x1DA8 => self.write_data_port(value as u16),
            0x1DAA => self.control.write_spucnt(
                value,
                &mut self.sound_ram,
                &mut self.data_port,
                &mut self.reverb,
                &mut self.noise,
            ),
            0x1DAC => {
                // Sound RAM data transfer control register; writing any value other than $0004
                // would be highly unexpected
                if value & 0xFFFF != 0x0004 {
                    todo!("Unexpected sound RAM data transfer control write: {value:04X}");
                }
            }
            0x1DB0 => self.volume.write_cd_l(value),
            0x1DB2 => self.volume.write_cd_r(value),
            0x1DB4 => log::warn!("Unimplemented external audio volume L write: {value:04X}"),
            0x1DB6 => log::warn!("Unimplemented external audio volume R write: {value:04X}"),
            0x1DC0..=0x1DFF => self.reverb.write_register(address, value),
            _ => todo!("SPU write {address:08X} {value:08X}"),
        }
    }

    // $1F801C00-$1F801D7F: Individual voice registers (read)
    fn read_voice_register(&self, address: u32) -> u32 {
        let voice = get_voice_number(address);
        if voice >= NUM_VOICES {
            log::error!("Invalid voice register read: {address:08X}");
            return 0;
        }

        match address & 0xF {
            0x0 => {
                // $1F801C00 + N*$10: Voice volume L
                self.voices[voice].volume_l.read()
            }
            0x2 => {
                // $1F801C02 + N*$10: Voice volume R
                self.voices[voice].volume_r.read()
            }
            0x4 => {
                // $1F801C04 + N*$10: Voice sample rate
                self.voices[voice].sample_rate.into()
            }
            0x6 => {
                // $1F801C06 + N*$10: ADPCM start address
                self.voices[voice].read_start_address()
            }
            0x8 => {
                // $1F801C08 + N*$10: ADSR settings, low halfword
                self.voices[voice].adsr.settings.read_low()
            }
            0xA => {
                // $1F801C0A + N*$10: ADSR settings, high halfword
                self.voices[voice].adsr.settings.read_high()
            }
            0xC => {
                // $1F801C0C + N*$10: Current ADSR level
                self.voices[voice].read_adsr_level()
            }
            0xE => {
                // $1F801C0E + N*$10: ADPCM repeat address
                self.voices[voice].read_repeat_address()
            }
            _ => {
                log::warn!("SPU voice {voice} register read: {address:08X}");
                0
            }
        }
    }

    // $1F801C00-$1F801D7F: Individual voice registers (write)
    fn write_voice_register(&mut self, address: u32, value: u32) {
        let voice = get_voice_number(address);
        if voice >= NUM_VOICES {
            log::error!("Invalid voice register write: {address:08X} {value:04X}");
            return;
        }

        match address & 0xF {
            0x0 => {
                // $1F801C00 + N*$10: Voice volume L
                self.voices[voice].volume_l.write(value);
                log::trace!("Voice {voice} volume L: {:?}", self.voices[voice].volume_l);
            }
            0x2 => {
                // $1F801C02 + N*$10: Voice volume R
                self.voices[voice].volume_r.write(value);
                log::trace!("Voice {voice} volume R: {:?}", self.voices[voice].volume_r);
            }
            0x4 => {
                // $1F801C04 + N*$10: Voice sample rate
                self.voices[voice].sample_rate = value as u16;
                log::trace!("Voice {voice} sample rate: {:04X}", self.voices[voice].sample_rate);
            }
            0x6 => {
                // $1F801C06 + N*$10: ADPCM start address
                self.voices[voice].write_start_address(value);
                log::trace!("Voice {voice} start address: {:05X}", (value & 0xFFFF) << 3);
            }
            0x8 => {
                // $1F801C08 + N*$10: ADSR settings, low halfword
                self.voices[voice].adsr.settings.write_low(value);
                log::trace!("Voice {voice} ADSR settings (low): {:?}", self.voices[voice].adsr);
            }
            0xA => {
                // $1F801C0A + N*$10: ADSR settings, high halfword
                self.voices[voice].adsr.settings.write_high(value);
                log::trace!("Voice {voice} ADSR settings (high): {:?}", self.voices[voice].adsr);
            }
            0xC => {
                // $1F801C0C + N*$10: Current ADSR level
                log::warn!("Unimplemented ADSR level write (voice {voice}): {value:04X}");
            }
            0xE => {
                // $1F801C0E + N*$10: ADPCM repeat address
                self.voices[voice].write_repeat_address(value);
                log::trace!("Voice {voice} repeat address: {:05X}", (value & 0xFFFF) << 3);
            }
            _ => todo!("voice {voice} register write: {address:08X} {value:04X}"),
        }
    }

    // $1F801DAE: SPU status register (SPUSTAT)
    fn read_status_register(&self) -> u32 {
        // TODO: bit 11 (writing to first/second half of capture buffers)
        // TODO: bit 10 (data transfer busy) is hardcoded
        // TODO: timing? switching to DMA read mode should not immediately set bits 7 and 9
        let value = (u32::from(self.data_port.mode == DataPortMode::DmaRead) << 9)
            | (u32::from(self.data_port.mode == DataPortMode::DmaWrite) << 8)
            | (u32::from(self.data_port.mode.is_dma()) << 7)
            | (u32::from(self.sound_ram.irq.get()) << 6)
            | ((self.data_port.mode as u32) << 5)
            | (u32::from(self.control.external_audio_reverb_enabled) << 3)
            | (u32::from(self.control.cd_audio_reverb_enabled) << 2)
            | (u32::from(self.control.external_audio_enabled) << 1)
            | u32::from(self.control.cd_audio_enabled);

        log::trace!("SPUSTAT read: {value:08X}");

        value
    }

    // $1F801DA8: Sound RAM data transfer FIFO port
    pub fn read_data_port(&mut self) -> u16 {
        let addr = self.data_port.current_address as usize;
        let halfword = u16::from_le_bytes([self.sound_ram[addr], self.sound_ram[addr + 1]]);

        self.data_port.current_address = (self.data_port.current_address + 2) & SOUND_RAM_MASK;

        halfword
    }

    // $1F801DA8: Sound RAM data transfer FIFO port
    pub fn write_data_port(&mut self, value: u16) {
        // TODO emulate the 32-halfword FIFO?
        // TODO check current state? (requires FIFO emulation, the BIOS writes while mode is off)
        let [lsb, msb] = value.to_le_bytes();
        self.sound_ram[self.data_port.current_address as usize] = lsb;
        self.sound_ram[(self.data_port.current_address + 1) as usize] = msb;

        log::trace!("Wrote to {:05X} in audio RAM", self.data_port.current_address);

        self.data_port.current_address = (self.data_port.current_address + 2) & SOUND_RAM_MASK;
    }

    // $1F801D88: Key on (voices 0-15)
    fn key_on_low(&mut self, value: u32) {
        log::debug!("Key on low write: {value:04X}");

        for voice in 0..16 {
            if value.bit(voice) {
                log::trace!("Keying on voice {voice}");
                self.voices[voice as usize].key_on(&self.sound_ram);
            }
        }

        self.control.record_kon_low_write(value);
    }

    // $1F801D8A: Key on (voices 16-23)
    fn key_on_high(&mut self, value: u32) {
        log::debug!("Key on high write: {value:04X}");

        for voice in 16..24 {
            if value.bit(voice - 16) {
                log::trace!("Keying on voice {voice}");
                self.voices[voice as usize].key_on(&self.sound_ram);
            }
        }

        self.control.record_kon_high_write(value);
    }

    // $1F801D8C: Key off (voices 0-15)
    fn key_off_low(&mut self, value: u32) {
        log::debug!("Key off low write: {value:04X}");

        for voice in 0..16 {
            if value.bit(voice) {
                log::trace!("Keying off voice {voice}");
                self.voices[voice as usize].key_off();
            }
        }

        self.control.record_koff_low_write(value);
    }

    // $1F801D8E: Key off (voices 16-23)
    fn key_off_high(&mut self, value: u32) {
        log::debug!("Key off high write: {value:04X}");

        for voice in 16..24 {
            if value.bit(voice - 16) {
                log::trace!("Keying off voice {voice}");
                self.voices[voice as usize].key_off();
            }
        }

        self.control.record_koff_high_write(value);
    }

    // $1F801D90: Pitch modulation enabled (voices 1-15)
    // Pitch modulation cannot be enabled for voice 0
    fn write_pitch_modulation_low(&mut self, value: u32) {
        log::debug!("Pitch modulation low write: {value:04X}");

        for voice in 1..16 {
            self.voices[voice].pitch_modulation_enabled = value.bit(voice as u8);
        }
    }

    // $1F801D92: Pitch modulation enabled (voices 16-23)
    fn write_pitch_modulation_high(&mut self, value: u32) {
        log::debug!("Pitch modulation high write: {value:04X}");

        for voice in 16..24 {
            self.voices[voice].pitch_modulation_enabled = value.bit((voice - 16) as u8);
        }
    }

    // $1F801D94: Noise enabled (voices 0-15)
    fn write_noise_low(&mut self, value: u32) {
        log::debug!("Noise low write: {value:04X}");

        for voice in 0..16 {
            self.voices[voice].noise_enabled = value.bit(voice as u8);
        }
    }

    // $1F801D96: Noise enabled (voices 16-23)
    fn write_noise_high(&mut self, value: u32) {
        log::debug!("Noise high write: {value:04X}");

        for voice in 16..24 {
            self.voices[voice].noise_enabled = value.bit((voice - 16) as u8);
        }
    }
}

fn get_voice_number(address: u32) -> usize {
    ((address >> 4) & 0x1F) as usize
}

fn multiply_volume(sample: i16, volume: i16) -> i16 {
    ((i32::from(sample) * i32::from(volume)) >> 15) as i16
}

fn multiply_volume_i32(sample: i32, volume: i32) -> i32 {
    (sample * volume) >> 15
}

// [ cd_l_to_spu_l  cd_r_to_spu_l ] * [ cd_l ] = [ cd_in_l ]
// [ cd_l_to_spu_r  cd_r_to_spu_r ] * [ cd_r ]   [ cd_in_r ]
fn apply_volume_matrix(cd_sample: (i16, i16), matrix: [[u8; 2]; 2]) -> (i16, i16) {
    // TODO maybe not correct saturation behavior?
    let cd_l = i32::from(cd_sample.0);
    let cd_r = i32::from(cd_sample.1);

    let matrix = matrix.map(|row| row.map(i32::from));

    let cd_in_l = ((matrix[0][0] * cd_l) >> 7) + ((matrix[0][1] * cd_r) >> 7);
    let cd_in_r = ((matrix[1][0] * cd_l) >> 7) + ((matrix[1][1] * cd_r) >> 7);

    let cd_in_l = cd_in_l.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
    let cd_in_r = cd_in_r.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
    (cd_in_l, cd_in_r)
}
