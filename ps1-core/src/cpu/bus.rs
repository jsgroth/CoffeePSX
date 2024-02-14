#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    Byte,
    HalfWord,
    Word,
}

pub trait BusInterface {
    fn read(&mut self, address: u32, size: OpSize) -> u32;

    fn write(&mut self, address: u32, value: u32, size: OpSize);
}
