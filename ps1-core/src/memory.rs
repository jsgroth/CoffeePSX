use crate::api::{Ps1Error, Ps1Result};
use crate::cpu::OpSize;

const BIOS_ROM_LEN: usize = 512 * 1024;
const MAIN_RAM_LEN: usize = 2 * 1024 * 1024;
const SCRATCHPAD_LEN: usize = 1024;

const BIOS_ROM_MASK: u32 = (BIOS_ROM_LEN - 1) as u32;
const MAIN_RAM_MASK: u32 = (MAIN_RAM_LEN - 1) as u32;
const SCRATCHPAD_MASK: u32 = (SCRATCHPAD_LEN - 1) as u32;

type BiosRom = [u8; BIOS_ROM_LEN];
type MainRam = [u8; MAIN_RAM_LEN];
type Scratchpad = [u8; SCRATCHPAD_LEN];

// TODO I-cache (or is this stored in CP0?)
#[derive(Debug, Clone)]
pub struct Memory {
    bios_rom: Box<BiosRom>,
    main_ram: Box<MainRam>,
    scratchpad: Box<Scratchpad>,
}

impl Memory {
    pub fn new(bios_rom: Vec<u8>) -> Ps1Result<Self> {
        if bios_rom.len() != BIOS_ROM_LEN {
            return Err(Ps1Error::IncorrectBiosSize {
                bios_len: bios_rom.len(),
            });
        }

        Ok(Self {
            bios_rom: bios_rom.into_boxed_slice().try_into().unwrap(),
            main_ram: vec![0; MAIN_RAM_LEN].into_boxed_slice().try_into().unwrap(),
            scratchpad: vec![0; SCRATCHPAD_LEN]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
        })
    }

    pub fn read_bios_rom(&self, address: u32, size: OpSize) -> u32 {
        size.read_memory(self.bios_rom.as_slice(), address & BIOS_ROM_MASK)
    }

    pub fn read_main_ram(&self, address: u32, size: OpSize) -> u32 {
        log::trace!("Main RAM read: {address:08X} {size:?}");
        size.read_memory(self.main_ram.as_slice(), address & MAIN_RAM_MASK)
    }

    pub fn write_main_ram(&mut self, address: u32, value: u32, size: OpSize) {
        log::trace!("Main RAM write: {address:08X} {value:08X} {size:?}");
        size.write_memory(self.main_ram.as_mut_slice(), address & MAIN_RAM_MASK, value);
    }

    pub fn read_scratchpad_ram(&self, address: u32, size: OpSize) -> u32 {
        size.read_memory(self.scratchpad.as_slice(), address & SCRATCHPAD_MASK)
    }

    pub fn write_scratchpad_ram(&mut self, address: u32, value: u32, size: OpSize) {
        size.write_memory(
            self.scratchpad.as_mut_slice(),
            address & SCRATCHPAD_MASK,
            value,
        );
    }

    pub fn copy_to_main_ram(&mut self, data: &[u8], ram_addr: u32) {
        self.main_ram[ram_addr as usize..ram_addr as usize + data.len()].copy_from_slice(data);
    }
}
