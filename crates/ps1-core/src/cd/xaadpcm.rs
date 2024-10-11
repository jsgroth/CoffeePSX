//! CD-XA ADPCM code
//!
//! The CD-XA ADPCM compression format is the same as the SPU ADPCM format, but the data layout is
//! a bit different.
//!
//! The coding info byte in the sector subheader specifies metadata that applies to the entire sector:
//! - Mono or Stereo
//! - 37800 Hz or 18900 Hz
//! - 4-bit or 8-bit samples
//! - Emphasis flag (apparently not used by any released games)
//!
//! Each sector contains 18 data blocks that are each 128 bytes. The number of samples in each data
//! block depends on coding info and is one of the following:
//! - 224 4-bit Mono samples
//! - 112 4-bit Stereo samples or 8-bit Mono samples
//! - 56 8-bit Stereo samples
//!
//! In total, that means each ADPCM sector contains one of the following:
//! - 4032 4-bit Mono samples (equivalent to 8 CD-DA sectors at 37800 Hz or 16 at 18900 Hz)
//! - 2016 4-bit Stereo samples (equivalent to 4 CD-DA sectors at 37800 Hz or 8 at 18900 Hz)
//! - 1008 8-bit Stereo samples (equivalent to 2 CD-DA sectors at 37800 Hz or 4 at 18900 Hz)
//!
//! Each 128-byte data block is split into either 8 audio blocks (4-bit samples) or 4 audio blocks
//! (8-bit samples), with each audio block containing 28 samples. For Stereo the audio blocks alternate
//! between the left channel and the right channel, and audio blocks are played two at a time. For
//! Mono the audio blocks are played one at a time in sequence.
//!
//! Data blocks begin with a 16-byte header that specifies the ADPCM shift and filter values for each
//! audio block. The remaining 112 bytes contain interleaved ADPCM sample values: the first sample
//! from each block, then the second sample from each block, then the third, etc. The final 4 bytes
//! contain the 28th sample from each block.

mod tables;

use crate::num::U8Ext;
use crate::spu::adpcm;
use bincode::{Decode, Encode};

const FILTER_0_TABLE: [i32; 5] = adpcm::FILTER_0_TABLE;
const FILTER_1_TABLE: [i32; 5] = adpcm::FILTER_1_TABLE;

// Each sector contains up to 18 data blocks * 8 audio blocks * 28 samples
const ADPCM_BUFFER_CAPACITY: usize = 18 * 8 * 28;

// 44100 Hz = 14/6 * 18900 Hz
const OUTPUT_BUFFER_CAPACITY: usize = ADPCM_BUFFER_CAPACITY * 14 / 6;

#[derive(Debug, Clone, Encode, Decode)]
struct ResampleRingBuffer {
    buffer: [i16; 32],
    idx: usize,
}

impl ResampleRingBuffer {
    fn new() -> Self {
        Self { buffer: [0; 32], idx: 0 }
    }

    fn clear(&mut self) {
        self.buffer.fill(0);
        self.idx = 0;
    }

    fn push(&mut self, value: i16) {
        self.buffer[self.idx] = value;
        self.idx = (self.idx + 1) & 0x1F;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum ChannelMode {
    Stereo,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum SampleRate {
    // 39800 Hz
    Normal,
    // 18900 Hz
    Half,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct XaAdpcmState {
    pub file: u8,
    pub channel: u8,
    pub muted: bool,
    adpcm_buffer_l: Vec<i16>,
    adpcm_buffer_r: Vec<i16>,
    output_buffer_l: Vec<i16>,
    output_buffer_r: Vec<i16>,
    output_idx: usize,
    block_buffer_l: [i16; 32],
    block_buffer_r: [i16; 32],
    resample_ring_buffer_l: ResampleRingBuffer,
    resample_ring_buffer_r: ResampleRingBuffer,
    channel_mode: ChannelMode,
    sample_rate: SampleRate,
}

impl XaAdpcmState {
    pub fn new() -> Self {
        Self {
            file: 0,
            channel: 0,
            muted: true,
            adpcm_buffer_l: Vec::with_capacity(ADPCM_BUFFER_CAPACITY),
            adpcm_buffer_r: Vec::with_capacity(ADPCM_BUFFER_CAPACITY),
            output_buffer_l: Vec::with_capacity(OUTPUT_BUFFER_CAPACITY),
            output_buffer_r: Vec::with_capacity(OUTPUT_BUFFER_CAPACITY),
            output_idx: 0,
            block_buffer_l: [0; 32],
            block_buffer_r: [0; 32],
            resample_ring_buffer_l: ResampleRingBuffer::new(),
            resample_ring_buffer_r: ResampleRingBuffer::new(),
            channel_mode: ChannelMode::Stereo,
            sample_rate: SampleRate::Normal,
        }
    }

    pub fn clear_buffers(&mut self) {
        self.adpcm_buffer_l.clear();
        self.adpcm_buffer_r.clear();
        self.output_buffer_l.clear();
        self.output_buffer_r.clear();
        self.output_idx = 0;
        self.block_buffer_l.fill(0);
        self.block_buffer_r.fill(0);
        self.resample_ring_buffer_l.clear();
        self.resample_ring_buffer_r.clear();
    }

    pub fn decode_sector(&mut self, sector: &[u8]) {
        let coding_info = sector[19];
        if coding_info != 0x00 && coding_info != 0x01 && coding_info != 0x04 && coding_info != 0x05
        {
            todo!("CD-XA ADPCM sector with coding info {coding_info:02X}");
        }

        self.channel_mode =
            if coding_info.bit(0) { ChannelMode::Stereo } else { ChannelMode::Mono };

        self.sample_rate = if coding_info.bit(2) { SampleRate::Half } else { SampleRate::Normal };

        self.adpcm_buffer_l.clear();
        self.adpcm_buffer_r.clear();
        self.output_buffer_l.clear();
        self.output_buffer_r.clear();
        self.output_idx = 0;

        // At beginning, skip 12 sync bytes + 4 header bytes + 8 subheader bytes
        // At end, skip 20 padding bytes + 4 EDC bytes
        for data_block in sector[24..2352 - 24].chunks_exact(128) {
            for audio_block_idx in 0..4 {
                match self.channel_mode {
                    ChannelMode::Stereo => {
                        // Stereo: Block N is the next L block and block N+1 is the next R block
                        decode_audio_block(
                            data_block,
                            2 * audio_block_idx,
                            &mut self.block_buffer_l,
                            &mut self.adpcm_buffer_l,
                        );
                        decode_audio_block(
                            data_block,
                            2 * audio_block_idx + 1,
                            &mut self.block_buffer_r,
                            &mut self.adpcm_buffer_r,
                        );
                    }
                    ChannelMode::Mono => {
                        // Mono: Decode the next 2 blocks in sequence using the same buffers
                        decode_audio_block(
                            data_block,
                            2 * audio_block_idx,
                            &mut self.block_buffer_l,
                            &mut self.adpcm_buffer_l,
                        );
                        decode_audio_block(
                            data_block,
                            2 * audio_block_idx + 1,
                            &mut self.block_buffer_l,
                            &mut self.adpcm_buffer_l,
                        );
                    }
                }
            }
        }

        resample_to_44100hz(
            self.sample_rate,
            &self.adpcm_buffer_l,
            &mut self.output_buffer_l,
            &mut self.resample_ring_buffer_l,
        );

        match self.channel_mode {
            ChannelMode::Stereo => {
                resample_to_44100hz(
                    self.sample_rate,
                    &self.adpcm_buffer_r,
                    &mut self.output_buffer_r,
                    &mut self.resample_ring_buffer_r,
                );
            }
            ChannelMode::Mono => {
                // If Mono, use the L ADPCM buffer instead of the R ADPCM buffer because Mono ADPCM
                // decoding only populates the L buffer
                resample_to_44100hz(
                    self.sample_rate,
                    &self.adpcm_buffer_l,
                    &mut self.output_buffer_r,
                    &mut self.resample_ring_buffer_r,
                );
            }
        }
    }

    pub fn maybe_output_sample(&mut self) -> Option<(i16, i16)> {
        if self.output_idx >= self.output_buffer_l.len() {
            return None;
        }

        let sample_l = self.output_buffer_l[self.output_idx];
        let sample_r = self.output_buffer_r[self.output_idx];
        self.output_idx += 1;

        (!self.muted).then_some((sample_l, sample_r))
    }
}

fn decode_audio_block(
    data_block: &[u8],
    audio_block_idx: usize,
    block_buffer: &mut [i16; 32],
    adpcm_buffer: &mut Vec<i16>,
) {
    // Move last 4 samples from the previous block to the beginning of the buffer
    for i in 0..4 {
        block_buffer[i] = block_buffer[28 + i];
    }

    let header_byte = data_block[0x04 + audio_block_idx];
    let shift = header_byte & 0xF;
    let filter = (header_byte >> 4) & 0x3;

    let effective_shift = adpcm::compute_effective_shift(shift);
    let filter_0 = FILTER_0_TABLE[filter as usize];
    let filter_1 = FILTER_1_TABLE[filter as usize];

    for i in 0..28 {
        let sample_byte = data_block[16 + 4 * i + audio_block_idx / 2];
        let sample = i4_sample(sample_byte >> (4 * (audio_block_idx & 1)));

        let shifted = sample << effective_shift;

        let older: i32 = block_buffer[2 + i].into();
        let old: i32 = block_buffer[3 + i].into();
        let filtered = shifted + (filter_0 * old + filter_1 * older + 32) / 64;

        let clamped = filtered.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
        block_buffer[4 + i] = clamped;
        adpcm_buffer.push(clamped);
    }
}

fn i4_sample(sample: u8) -> i32 {
    (((sample as i8) << 4) >> 4).into()
}

fn resample_to_44100hz(
    sample_rate: SampleRate,
    input: &[i16],
    output: &mut Vec<i16>,
    ring_buffer: &mut ResampleRingBuffer,
) {
    let pushes_per_sample = match sample_rate {
        SampleRate::Normal => 1,
        SampleRate::Half => 2,
    };

    let mut counter = 0;
    for &input_sample in input {
        for _ in 0..pushes_per_sample {
            ring_buffer.push(input_sample);

            counter += 1;
            if counter == 6 {
                counter = 0;

                for table in 0..7 {
                    let mut sum = 0;
                    for i in 1..30 {
                        let ring_buffer_value: i32 =
                            ring_buffer.buffer[ring_buffer.idx.wrapping_sub(i) & 0x1F].into();
                        let table_value: i32 = tables::INTERPOLATION[7 * (i - 1) + table].into();
                        sum += (ring_buffer_value * table_value) >> 15;
                    }

                    output.push(sum.clamp(i16::MIN.into(), i16::MAX.into()) as i16);
                }
            }
        }
    }
}
