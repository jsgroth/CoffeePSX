#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    Byte,
    HalfWord,
    Word,
}

impl OpSize {
    pub fn read_memory(self, memory: &[u8], address: u32) -> u32 {
        let address = address as usize;
        match self {
            Self::Byte => memory[address].into(),
            Self::HalfWord => {
                u16::from_le_bytes(memory[address..address + 2].try_into().unwrap()).into()
            }
            Self::Word => u32::from_le_bytes(memory[address..address + 4].try_into().unwrap()),
        }
    }

    pub fn write_memory(self, memory: &mut [u8], address: u32, value: u32) {
        let address = address as usize;
        match self {
            Self::Byte => {
                memory[address] = value as u8;
            }
            Self::HalfWord => {
                let bytes = (value as u16).to_le_bytes();
                memory[address..address + 2].copy_from_slice(&bytes);
            }
            Self::Word => {
                let bytes = value.to_le_bytes();
                memory[address..address + 4].copy_from_slice(&bytes);
            }
        }
    }
}

pub trait BusInterface {
    fn read(&mut self, address: u32, size: OpSize) -> u32;

    fn write(&mut self, address: u32, value: u32, size: OpSize);
}
