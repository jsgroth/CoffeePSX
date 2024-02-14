use ps1_core::cpu::bus::{BusInterface, OpSize};
use ps1_core::cpu::R3000;
use std::error::Error;
use std::{env, fs};

const RAM_LEN: usize = 2 * 1024 * 1024;

struct Bus {
    rom: Vec<u8>,
    ram: Vec<u8>,
}

impl Bus {
    fn new(rom: Vec<u8>, ram: Vec<u8>) -> Self {
        Self { rom, ram }
    }
}

impl BusInterface for Bus {
    fn read(&mut self, address: u32, size: OpSize) -> u32 {
        match address {
            0x00000000..=0x007FFFFF | 0x80000000..=0x807FFFFF | 0xB0000000..=0xB07FFFFF => {
                let ram_addr = (address & 0x1FFFFF) as usize;
                read_memory(&self.ram, ram_addr, size)
            }
            0x1FC00000..=0x1FFFFFFF | 0x9FC00000..=0x9FFFFFFF | 0xBFC00000..=0xBFFFFFFF => {
                let rom_addr = (address & 0x7FFFF) as usize;
                read_memory(&self.rom, rom_addr, size)
            }
            _ => todo!("read {address:08X} {size:?}"),
        }
    }

    fn write(&mut self, address: u32, value: u32, size: OpSize) {
        todo!("write {address:08X} {value:08X} {size:?}")
    }
}

fn read_memory(memory: &[u8], address: usize, size: OpSize) -> u32 {
    match size {
        OpSize::Byte => memory[address].into(),
        OpSize::HalfWord => {
            u16::from_le_bytes(memory[address..address + 2].try_into().unwrap()).into()
        }
        OpSize::Word => u32::from_le_bytes(memory[address..address + 4].try_into().unwrap()),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args();
    args.next();

    let filename = args.next().expect("Missing filename arg");
    let is_exe = args.next().as_ref().map(String::as_str) == Some("-e");

    let rom = fs::read(&filename)?;
    let mut ram = vec![0; RAM_LEN];

    let mut r3000 = R3000::new();
    if is_exe {
        let initial_pc = u32::from_le_bytes(rom[24..28].try_into().unwrap());
        r3000.set_pc(initial_pc);

        ram[..rom.len()].copy_from_slice(&rom);
    }

    let mut bus = Bus::new(rom, ram);

    for _ in 0..1_000_000_000 {
        r3000.execute_instruction(&mut bus);
    }

    Ok(())
}
