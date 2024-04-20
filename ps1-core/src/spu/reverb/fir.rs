use crate::spu::{multiply_volume_i32, I32Ext};
use bincode::{Decode, Encode};
use std::collections::VecDeque;

// From <https://psx-spx.consoledev.net/soundprocessingunitspu/#reverb-buffer-resampling>
const FILTER: &[i32; 39] = &[
    -0x0001, 0x0000, 0x0002, 0x0000, -0x000A, 0x0000, 0x0023, 0x0000, -0x0067, 0x0000, 0x010A,
    0x0000, -0x0268, 0x0000, 0x0534, 0x0000, -0x0B90, 0x0000, 0x2806, 0x4000, 0x2806, 0x0000,
    -0x0B90, 0x0000, 0x0534, 0x0000, -0x0268, 0x0000, 0x010A, 0x0000, -0x0067, 0x0000, 0x0023,
    0x0000, -0x000A, 0x0000, 0x0002, 0x0000, -0x0001,
];

#[derive(Debug, Clone, Encode, Decode)]
pub struct FirSampleDeque(VecDeque<i32>);

impl FirSampleDeque {
    pub fn new() -> Self {
        Self(VecDeque::with_capacity(FILTER.len()))
    }

    pub fn push(&mut self, sample: i32) {
        if self.0.len() == FILTER.len() {
            self.0.pop_front();
        }
        self.0.push_back(sample);
    }
}

impl Default for FirSampleDeque {
    fn default() -> Self {
        Self::new()
    }
}

pub fn filter(samples: &FirSampleDeque) -> i16 {
    let mut sum = 0_i32;
    for (i, sample) in samples.0.iter().copied().enumerate().take(FILTER.len()) {
        sum += multiply_volume_i32(sample, FILTER[i]);
    }
    sum.clamp_to_i16()
}
