//! PS1 SPU (Sound Processing Unit)

mod adpcm;
mod envelope;
mod reverb;
mod voice;

use crate::cd::CdController;
use crate::cpu::OpSize;
use crate::num::U32Ext;
use crate::spu::envelope::VolumeControl;
use crate::spu::reverb::ReverbUnit;
use crate::spu::voice::Voice;
use bincode::{Decode, Encode};
use std::array;

const AUDIO_RAM_LEN: usize = 512 * 1024;
const AUDIO_RAM_MASK: u32 = (AUDIO_RAM_LEN - 1) as u32;

const NUM_VOICES: usize = 24;

type AudioRam = [u8; AUDIO_RAM_LEN];

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

        log::trace!("Sound RAM data transfer address: {:05X}", self.start_address);
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
    irq_enabled: bool,
    noise_shift: u8,
    noise_step: u8,
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
            irq_enabled: false,
            noise_shift: 0,
            noise_step: 0,
            last_key_on_write: 0,
            last_key_off_write: 0,
        }
    }

    // $1F801DAA: SPU control register (SPUCNT)
    fn read_spucnt(&self, data_port: &DataPort, reverb: &ReverbUnit) -> u32 {
        (u32::from(self.spu_enabled) << 15)
            | (u32::from(self.amplifier_enabled) << 14)
            | (u32::from(self.noise_shift) << 10)
            | (u32::from(self.noise_step) << 8)
            | (u32::from(reverb.writes_enabled) << 7)
            | (u32::from(self.irq_enabled) << 6)
            | ((data_port.mode as u32) << 4)
            | (u32::from(self.external_audio_reverb_enabled) << 3)
            | (u32::from(self.cd_audio_reverb_enabled) << 2)
            | (u32::from(self.external_audio_enabled) << 1)
            | u32::from(self.cd_audio_enabled)
    }

    // $1F801DAA: SPU control register (SPUCNT)
    fn write_spucnt(&mut self, value: u32, data_port: &mut DataPort, reverb: &mut ReverbUnit) {
        self.spu_enabled = value.bit(15);
        self.amplifier_enabled = value.bit(14);
        self.noise_shift = ((value >> 10) & 0xF) as u8;
        self.noise_step = ((value >> 8) & 3) as u8;
        reverb.writes_enabled = value.bit(7);
        self.irq_enabled = value.bit(6);
        data_port.mode = DataPortMode::from_bits(value >> 4);
        self.external_audio_reverb_enabled = value.bit(3);
        self.cd_audio_reverb_enabled = value.bit(2);
        self.external_audio_enabled = value.bit(1);
        self.cd_audio_enabled = value.bit(0);

        log::trace!("SPUCNT write");
        log::trace!("  SPU enabled: {}", self.spu_enabled);
        log::trace!("  Amplifier enabled: {}", self.amplifier_enabled);
        log::trace!("  Noise shift: {}", self.noise_shift);
        log::trace!("  Noise step: {}", self.noise_step + 4);
        log::trace!("  Reverb writes enabled: {}", reverb.writes_enabled);
        log::trace!("  IRQ enabled: {}", self.irq_enabled);
        log::trace!("  Data port mode: {:?}", data_port.mode);
        log::trace!("  External audio reverb enabled: {}", self.external_audio_reverb_enabled);
        log::trace!("  CD audio reverb enabled: {}", self.cd_audio_reverb_enabled);
        log::trace!("  External audio enabled: {}", self.external_audio_enabled);
        log::trace!("  CD audio enabled: {}", self.cd_audio_enabled);
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
    audio_ram: Box<AudioRam>,
    voices: [Voice; NUM_VOICES],
    control: ControlRegisters,
    volume: VolumeControl,
    data_port: DataPort,
    reverb: ReverbUnit,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            audio_ram: vec![0; AUDIO_RAM_LEN].into_boxed_slice().try_into().unwrap(),
            voices: array::from_fn(|_| Voice::new()),
            control: ControlRegisters::new(),
            volume: VolumeControl::new(),
            data_port: DataPort::new(),
            reverb: ReverbUnit::default(),
        }
    }

    pub fn clock(&mut self, cd_controller: &CdController) -> (f64, f64) {
        self.volume.main_l.clock();
        self.volume.main_r.clock();

        for voice in &mut self.voices {
            voice.clock(&self.audio_ram);
        }

        self.reverb.clock(&self.voices, &mut self.audio_ram);

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

        // Mix in CD audio samples
        let (cd_l, cd_r) = apply_volume_matrix(
            cd_controller.current_audio_sample(),
            cd_controller.spu_volume_matrix(),
        );
        let cd_l = multiply_volume(cd_l, self.volume.cd_l);
        let cd_r = multiply_volume(cd_r, self.volume.cd_r);
        let sample_l = (i32::from(sample_l) + i32::from(cd_l)).clamp_to_i16();
        let sample_r = (i32::from(sample_r) + i32::from(cd_r)).clamp_to_i16();

        // Convert from i16 to f64
        let sample_l = f64::from(sample_l) / -f64::from(i16::MIN);
        let sample_r = f64::from(sample_r) / -f64::from(i16::MIN);

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
            // KON/KOFF are normally write-only, but reads return the last written value
            0x1D88 => self.control.last_key_on_write & 0xFFFF,
            0x1D8A => self.control.last_key_on_write >> 16,
            0x1D8C => self.control.last_key_off_write & 0xFFFF,
            0x1D8E => self.control.last_key_off_write >> 16,
            0x1DA6 => self.data_port.read_start_address(),
            0x1DAA => self.control.read_spucnt(&self.data_port, &self.reverb),
            // TODO return an actual value for sound RAM data transfer control?
            0x1DAC => 0x0004,
            0x1DAE => self.read_status_register(),
            0x1DB8 => (self.volume.main_l.volume as u16).into(),
            0x1DBA => (self.volume.main_r.volume as u16).into(),
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
                self.write_register(address, value & 0xFFFF, OpSize::HalfWord);
                self.write_register(address | 2, value >> 16, OpSize::HalfWord);
                return;
            }
        }

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
            0x1D90 => log::warn!("Unimplemented FM/LFO mode write (low halfword): {value:04X}"),
            0x1D92 => log::warn!("Unimplemented FM/LFO mode write (high halfword): {value:04X}"),
            0x1D94 => log::warn!("Unimplemented noise mode write (low halfword): {value:04X}"),
            0x1D96 => log::warn!("Unimplemented noise mode write (high halfword): {value:04X}"),
            0x1D98 => self.reverb.write_reverb_on_low(value),
            0x1D9A => self.reverb.write_reverb_on_high(value),
            0x1D9C => {
                log::warn!("ENDX write (voices 0-15): {value:04X}");
            }
            0x1D9E => {
                log::warn!("ENDX write (voices 16-23): {value:04X}");
            }
            0x1DA2 => self.reverb.write_buffer_start_address(value),
            0x1DA6 => self.data_port.write_transfer_address(value),
            0x1DA8 => self.write_data_port(value as u16),
            0x1DAA => self.control.write_spucnt(value, &mut self.data_port, &mut self.reverb),
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
            0xC => {
                // $1F801C0C + N*$10: Current ADSR level
                self.voices[voice].read_adsr_level()
            }
            _ => todo!("SPU voice {voice} register read: {address:08X}"),
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
                self.voices[voice].write_volume_l(value);
                log::trace!("Voice {voice} volume L: {:?}", self.voices[voice].volume_l);
            }
            0x2 => {
                // $1F801C02 + N*$10: Voice volume R
                self.voices[voice].write_volume_r(value);
                log::trace!("Voice {voice} volume R: {:?}", self.voices[voice].volume_r);
            }
            0x4 => {
                // $1F801C04 + N*$10: Voice sample rate
                self.voices[voice].write_sample_rate(value);
                log::trace!("Voice {voice} sample rate: {:04X}", self.voices[voice].sample_rate);
            }
            0x6 => {
                // $1F801C06 + N*$10: ADPCM start address
                self.voices[voice].write_start_address(value);
                log::trace!(
                    "Voice {voice} start address: {:05X}",
                    self.voices[voice].start_address
                );
            }
            0x8 => {
                // $1F801C08 + N*$10: ADSR settings, low halfword
                self.voices[voice].write_adsr_low(value);
                log::trace!("Voice {voice} ADSR settings (low): {:?}", self.voices[voice].adsr);
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
            0xA => {
                // $1F801C0A + N*$10: ADSR settings, high halfword
                self.voices[voice].write_adsr_high(value);
                log::trace!("Voice {voice} ADSR settings (high): {:?}", self.voices[voice].adsr);
            }
            _ => todo!("voice {voice} register write: {address:08X} {value:04X}"),
        }
    }

    // $1F801DAE: SPU status register (SPUSTAT)
    fn read_status_register(&self) -> u32 {
        // TODO: bit 11 (writing to first/second half of capture buffers)
        // TODO: bit 10 (data transfer busy) is hardcoded
        // TODO: bit 6 (IRQ)
        // TODO: timing? switching to DMA read mode should not immediately set bits 7 and 9
        let value = (u32::from(self.data_port.mode == DataPortMode::DmaRead) << 9)
            | (u32::from(self.data_port.mode == DataPortMode::DmaWrite) << 8)
            | (u32::from(self.data_port.mode.is_dma()) << 7)
            | ((self.data_port.mode as u32) << 5)
            | (u32::from(self.control.external_audio_reverb_enabled) << 3)
            | (u32::from(self.control.cd_audio_reverb_enabled) << 2)
            | (u32::from(self.control.external_audio_enabled) << 1)
            | u32::from(self.control.cd_audio_enabled);

        log::trace!("SPUSTAT read: {value:08X}");

        value
    }

    // $1F801DA8: Sound RAM data transfer FIFO port
    pub fn write_data_port(&mut self, value: u16) {
        // TODO emulate the 32-halfword FIFO?
        // TODO check current state? (requires FIFO emulation, the BIOS writes while mode is off)
        let [lsb, msb] = value.to_le_bytes();
        self.audio_ram[self.data_port.current_address as usize] = lsb;
        self.audio_ram[(self.data_port.current_address + 1) as usize] = msb;

        log::trace!("Wrote to {:05X} in audio RAM", self.data_port.current_address);

        self.data_port.current_address = (self.data_port.current_address + 2) & AUDIO_RAM_MASK;
    }

    // $1F801D88: Key on (voices 0-15)
    fn key_on_low(&mut self, value: u32) {
        log::trace!("Key on low write: {value:04X}");

        for voice in 0..16 {
            if value.bit(voice) {
                log::trace!("Keying on voice {voice}");
                self.voices[voice as usize].key_on(&self.audio_ram);
            }
        }

        self.control.record_kon_low_write(value);
    }

    // $1F801D8A: Key on (voices 16-23)
    fn key_on_high(&mut self, value: u32) {
        log::trace!("Key on high write: {value:04X}");

        for voice in 16..24 {
            if value.bit(voice - 16) {
                log::trace!("Keying on voice {voice}");
                self.voices[voice as usize].key_on(&self.audio_ram);
            }
        }

        self.control.record_kon_high_write(value);
    }

    // $1F801D8C: Key off (voices 0-15)
    fn key_off_low(&mut self, value: u32) {
        log::trace!("Key off low write: {value:04X}");

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
        log::trace!("Key off high write: {value:04X}");

        for voice in 16..24 {
            if value.bit(voice - 16) {
                log::trace!("Keying off voice {voice}");
                self.voices[voice as usize].key_off();
            }
        }

        self.control.record_koff_high_write(value);
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
