use r3000_emu::bus::{BusInterface, OpSize};
use r3000_emu::R3000;
use std::error::Error;
use std::{env, fs};

const RAM_LEN: usize = 2 * 1024 * 1024;

struct Bus {
    rom: Vec<u8>,
    ram: Vec<u8>,
}

impl Bus {
    fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            ram: vec![0; RAM_LEN],
        }
    }
}

impl BusInterface for Bus {
    fn read(&mut self, address: u32, size: OpSize) -> u32 {
        match address {
            0x1FC00000..=0x1FFFFFFF | 0x9FC00000..=0x9FFFFFFF | 0xBFC00000..=0xBFFFFFFF => {
                let rom_addr = (address & 0x7FFFF) as usize;
                match size {
                    OpSize::Byte => self.rom[rom_addr].into(),
                    OpSize::HalfWord => {
                        u16::from_le_bytes([self.rom[rom_addr], self.rom[rom_addr + 1]]).into()
                    }
                    OpSize::Word => u32::from_le_bytes([
                        self.rom[rom_addr],
                        self.rom[rom_addr + 1],
                        self.rom[rom_addr + 2],
                        self.rom[rom_addr + 3],
                    ]),
                }
            }
            _ => todo!("read {address:08X} {size:?}"),
        }
    }

    fn write(&mut self, address: u32, value: u32, size: OpSize) {
        todo!("write {address:08X} {value:08X} {size:?}")
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args();
    args.next();

    let filename = args.next().expect("Missing filename arg");
    let rom = fs::read(&filename)?;

    let mut bus = Bus::new(rom);

    let mut r3000 = R3000::new();
    for _ in 0..1_000_000_000 {
        r3000.execute_instruction(&mut bus);
    }

    Ok(())
}
