use bincode::{Decode, Encode};
use std::array;

const PARAMETER_CAPACITY: usize = 16;

#[derive(Debug, Clone, Encode, Decode)]
pub struct ParameterFifo {
    values: [u8; PARAMETER_CAPACITY],
    idx: usize,
    len: usize,
    consumed: bool,
}

impl ParameterFifo {
    pub fn new() -> Self {
        Self { values: array::from_fn(|_| 0), idx: 0, len: 0, consumed: true }
    }

    pub fn reset(&mut self) {
        self.values.fill(0);
        self.idx = 0;
        self.len = 0;
        self.consumed = true;
    }

    pub fn push(&mut self, value: u8) {
        if self.len == PARAMETER_CAPACITY {
            log::error!(
                "Push to response FIFO while at capacity of {PARAMETER_CAPACITY}, ignoring"
            );
            return;
        }

        self.values[self.len] = value;
        self.len += 1;
        self.consumed = false;
    }

    pub fn pop(&mut self) -> u8 {
        let value = self.values[self.idx];
        self.idx += 1;

        if self.idx == self.len {
            self.consumed = true;
        }

        // Response FIFO loops if the host reads 16 times before a new response is generated
        self.idx %= PARAMETER_CAPACITY;

        value
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn empty(&self) -> bool {
        self.len == 0
    }

    pub fn full(&self) -> bool {
        self.len == PARAMETER_CAPACITY
    }

    pub fn fully_consumed(&self) -> bool {
        self.consumed
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DataFifo {
    values: Box<[u8; cdrom::BYTES_PER_SECTOR as usize]>,
    idx: usize,
    len: usize,
}

impl DataFifo {
    pub fn new() -> Self {
        Self { values: Box::new(array::from_fn(|_| 0)), idx: 0, len: 0 }
    }

    pub fn copy_from_slice(&mut self, slice: &[u8]) {
        self.values[..slice.len()].copy_from_slice(slice);
        self.idx = 0;
        self.len = slice.len();
    }

    pub fn pop(&mut self) -> u8 {
        // Data FIFO repeatedly returns the last value if all elements are popped
        if self.len == 0 {
            return 0;
        } else if self.idx == self.len {
            return self.values[self.len - 1];
        }

        let value = self.values[self.idx];
        self.idx += 1;
        value
    }

    pub fn fully_consumed(&self) -> bool {
        self.idx == self.len
    }
}
