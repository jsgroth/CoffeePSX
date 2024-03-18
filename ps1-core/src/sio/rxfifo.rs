use bincode::{Decode, Encode};

const FIFO_LEN: u8 = 8;
const FIFO_MASK: u8 = FIFO_LEN - 1;

#[derive(Debug, Clone, Encode, Decode)]
pub struct RxFifo {
    values: [u8; FIFO_LEN as usize],
    write_idx: u8,
    read_idx: u8,
    len: u8,
}

impl RxFifo {
    pub fn new() -> Self {
        Self { values: [0; FIFO_LEN as usize], write_idx: 0, read_idx: 0, len: 0 }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn push(&mut self, value: u8) {
        if self.len == FIFO_LEN {
            // Overwrite the last value
            self.values[(self.write_idx.wrapping_sub(1) & FIFO_MASK) as usize] = value;
            return;
        }

        // Fill every remaining slot with the written value
        for i in self.len..FIFO_LEN {
            self.values[((self.write_idx + (i - self.len)) & FIFO_MASK) as usize] = value;
        }

        self.write_idx = (self.write_idx + 1) & FIFO_MASK;
        self.len += 1;

        log::debug!("Pushed {value:02X} into RX FIFO");
    }

    pub fn pop(&mut self) -> u8 {
        let value = self.values[self.read_idx as usize];

        // Replace popped value with 0
        self.values[self.read_idx as usize] = 0;

        self.read_idx = (self.read_idx + 1) & FIFO_MASK;
        if self.len != 0 {
            self.len -= 1;
        } else {
            self.write_idx = self.read_idx;
        }

        value
    }

    pub fn empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_functionality() {
        let mut fifo = RxFifo::new();
        for i in 0..8 {
            fifo.push(i * 4);
        }
        for i in 0..8 {
            assert_eq!(fifo.pop(), i * 4);
        }

        for _ in 0..3 {
            assert_eq!(fifo.pop(), 0);
        }

        for i in 0..8 {
            fifo.push(i * 4);
        }
        for i in 0..8 {
            assert_eq!(fifo.pop(), i * 4);
        }

        for _ in 0..16 {
            assert_eq!(fifo.pop(), 0);
        }
    }

    #[test]
    fn buffer_overrun() {
        let mut fifo = RxFifo::new();
        for i in 0..63 {
            fifo.push(i * 4)
        }
        for i in 0..7 {
            assert_eq!(fifo.pop(), i * 4);
        }
        assert_eq!(fifo.pop(), 62 * 4);

        for _ in 0..3 {
            assert_eq!(fifo.pop(), 0);
        }

        for i in 0..63 {
            fifo.push(i * 4)
        }
        for i in 0..7 {
            assert_eq!(fifo.pop(), i * 4);
        }
        assert_eq!(fifo.pop(), 62 * 4);

        for _ in 0..16 {
            assert_eq!(fifo.pop(), 0);
        }
    }

    #[test]
    fn buffer_underrun() {
        let mut fifo = RxFifo::new();

        fifo.push(1);
        fifo.push(3);

        assert_eq!(fifo.pop(), 1);
        for _ in 0..7 {
            assert_eq!(fifo.pop(), 3);
        }

        for _ in 0..3 {
            assert_eq!(fifo.pop(), 0);
        }

        fifo.push(5);
        fifo.push(7);

        assert_eq!(fifo.pop(), 5);
        for _ in 0..7 {
            assert_eq!(fifo.pop(), 7);
        }

        for _ in 0..16 {
            assert_eq!(fifo.pop(), 0);
        }
    }
}
