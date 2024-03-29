//! SPU ADPCM decoding
//!
//! Each 16-byte ADPCM block contains 28 compressed PCM samples.
//!
//! The block begins with a 2-byte header specifying the ADPCM shift value, the ADPCM filter value,
//! and loop flags. The remaining 14 bytes contain 4-bit ADPCM sample values.

use crate::spu::I32Ext;
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, Default, Encode, Decode)]
pub struct AdpcmHeader {
    pub shift: u8,
    pub filter: u8,
    pub loop_start: bool,
    pub loop_end: bool,
    pub loop_repeat: bool,
}

impl AdpcmHeader {
    fn from_spu_header(first_byte: u8, second_byte: u8) -> Self {
        let shift = first_byte & 0xF;
        let loop_end = second_byte & 1 != 0;
        let loop_repeat = second_byte & (1 << 1) != 0;
        let loop_start = second_byte & (1 << 2) != 0;

        let mut filter = (first_byte >> 4) & 0x7;
        if filter > 4 {
            // Only 0-4 are valid filter values
            log::error!("Invalid SPU ADPCM filter value, using 4 instead: {filter}");
            filter = 4;
        }

        Self { shift, filter, loop_start, loop_end, loop_repeat }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct SpuAdpcmBuffer {
    pub header: AdpcmHeader,
    samples: [i16; 32],
    idx: usize,
}

impl SpuAdpcmBuffer {
    pub fn new() -> Self {
        Self { header: AdpcmHeader::default(), samples: [0; 32], idx: 4 }
    }

    pub fn four_most_recent_samples(&self) -> [i16; 4] {
        self.samples[self.idx - 3..=self.idx].try_into().unwrap()
    }

    pub fn advance(&mut self) {
        self.idx += 1;
        assert!(self.idx <= self.samples.len(), "ADPCM decoding bug: advanced past end of block");
    }

    pub fn at_end_of_block(&self) -> bool {
        self.idx >= self.samples.len()
    }

    pub fn reset(&mut self) {
        self.samples.fill(0);
        self.idx = 4;
    }
}

pub const FILTER_0_TABLE: [i32; 5] = [0, 60, 115, 98, 122];
pub const FILTER_1_TABLE: [i32; 5] = [0, 0, -52, -55, -60];

pub fn compute_effective_shift(shift: u8) -> u8 {
    // Effective shift is (12 - shift)
    // Shift values of 13-15 function the same as 9, so effective shift is (12 - 9) = 3
    if shift > 12 { 3 } else { 12 - shift }
}

pub fn decode_spu_block(block: &[u8], buffer: &mut SpuAdpcmBuffer) {
    buffer.header = AdpcmHeader::from_spu_header(block[0], block[1]);

    let effective_shift = compute_effective_shift(buffer.header.shift);
    let filter_0 = FILTER_0_TABLE[buffer.header.filter as usize];
    let filter_1 = FILTER_1_TABLE[buffer.header.filter as usize];

    // Copy the last 4 samples from the previous block to the start of the buffer
    for i in 0..4 {
        buffer.samples[i] = buffer.samples[28 + i];
    }

    for sample_idx in 0..28 {
        let byte_idx = 2 + (sample_idx >> 1);
        let shift = 4 * (sample_idx & 1);
        let nibble = sign_extend_nibble(block[byte_idx] >> shift);

        let shifted = nibble << effective_shift;

        let buffer_idx = 4 + sample_idx;
        let old: i32 = buffer.samples[buffer_idx - 1].into();
        let older: i32 = buffer.samples[buffer_idx - 2].into();

        let filtered = shifted + (filter_0 * old + filter_1 * older + 32) / 64;
        buffer.samples[buffer_idx] = filtered.clamp_to_i16();
    }

    buffer.idx = 4;
}

fn sign_extend_nibble(nibble: u8) -> i32 {
    (((nibble as i8) << 4) >> 4).into()
}
