#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroFill {
    Yes,
    No,
}

#[derive(Debug, Clone)]
pub struct Fifo<const MAX_LEN: usize> {
    values: [u8; MAX_LEN],
    idx: usize,
    len: usize,
}

impl<const MAX_LEN: usize> Fifo<MAX_LEN> {
    pub fn new() -> Self {
        Self { values: [0; MAX_LEN], idx: 0, len: 0 }
    }

    pub fn reset(&mut self, zero_fill: ZeroFill) {
        self.idx = 0;
        self.len = 0;

        if zero_fill == ZeroFill::Yes {
            self.values.fill(0);
        }
    }

    pub fn push(&mut self, value: u8) {
        if self.len == self.values.len() {
            log::error!("Push to CD-ROM FIFO while full: {value:02X}");
            return;
        }

        self.values[self.len] = value;
        self.len += 1;
    }

    pub fn pop(&mut self) -> u8 {
        let value = self.values[self.idx];

        self.idx += 1;
        if self.idx == self.values.len() {
            self.idx = 0;
        }

        value
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn empty(&self) -> bool {
        self.idx >= self.len
    }

    pub fn full(&self) -> bool {
        self.len == MAX_LEN
    }

    pub fn fully_consumed(&self) -> bool {
        self.idx >= self.len
    }

    pub fn copy_from(&mut self, array: &[u8]) {
        self.values[..array.len()].copy_from_slice(array);
        self.idx = 0;
        self.len = array.len();
    }
}

pub type ParameterFifo = Fifo<16>;
pub type ResponseFifo = Fifo<16>;
pub type DataFifo = Fifo<2352>;
