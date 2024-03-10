//! PS1 system memory (main RAM / scratchpad / BIOS ROM)

use crate::api::{Ps1Error, Ps1Result};

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

macro_rules! impl_read_u8 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {
        $memory[($address & $addr_mask) as usize]
    };
}

macro_rules! impl_read_u16 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {{
        let address = ($address & $addr_mask) as usize;
        u16::from_le_bytes([$memory[address], $memory[address + 1]])
    }};
}

macro_rules! impl_read_u32 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {{
        let address = ($address & $addr_mask) as usize;
        u32::from_le_bytes([
            $memory[address],
            $memory[address + 1],
            $memory[address + 2],
            $memory[address + 3],
        ])
    }};
}

macro_rules! impl_write_u8 {
    ($memory:expr, $addr_mask: expr, $address:expr, $value:expr) => {
        $memory[($address & $addr_mask) as usize] = $value;
    };
}

macro_rules! impl_write_u16 {
    ($memory:expr, $addr_mask: expr, $address:expr, $value:expr) => {{
        let [lsb, msb] = $value.to_le_bytes();
        let address = ($address & $addr_mask) as usize;
        $memory[address] = lsb;
        $memory[address + 1] = msb;
    }};
}

macro_rules! impl_write_u32 {
    ($memory:expr, $addr_mask: expr, $address:expr, $value:expr) => {{
        let bytes = $value.to_le_bytes();
        let address = ($address & $addr_mask) as usize;
        for i in 0..4 {
            $memory[address + i] = bytes[i];
        }
    }};
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

    pub fn read_bios_u8(&self, address: u32) -> u8 {
        impl_read_u8!(self.bios_rom, BIOS_ROM_MASK, address)
    }

    pub fn read_bios_u16(&self, address: u32) -> u16 {
        impl_read_u16!(self.bios_rom, BIOS_ROM_MASK, address)
    }

    pub fn read_bios_u32(&self, address: u32) -> u32 {
        impl_read_u32!(self.bios_rom, BIOS_ROM_MASK, address)
    }

    pub fn read_main_ram_u8(&self, address: u32) -> u8 {
        impl_read_u8!(self.main_ram, MAIN_RAM_MASK, address)
    }

    pub fn read_main_ram_u16(&self, address: u32) -> u16 {
        impl_read_u16!(self.main_ram, MAIN_RAM_MASK, address)
    }

    pub fn read_main_ram_u32(&self, address: u32) -> u32 {
        impl_read_u32!(self.main_ram, MAIN_RAM_MASK, address)
    }

    pub fn write_main_ram_u8(&mut self, address: u32, value: u8) {
        impl_write_u8!(self.main_ram, MAIN_RAM_MASK, address, value);
    }

    pub fn write_main_ram_u16(&mut self, address: u32, value: u16) {
        impl_write_u16!(self.main_ram, MAIN_RAM_MASK, address, value);
    }

    pub fn write_main_ram_u32(&mut self, address: u32, value: u32) {
        impl_write_u32!(self.main_ram, MAIN_RAM_MASK, address, value);
    }

    pub fn read_scratchpad_u8(&self, address: u32) -> u8 {
        impl_read_u8!(self.scratchpad, SCRATCHPAD_MASK, address)
    }

    pub fn read_scratchpad_u16(&self, address: u32) -> u16 {
        impl_read_u16!(self.scratchpad, SCRATCHPAD_MASK, address)
    }

    pub fn read_scratchpad_u32(&self, address: u32) -> u32 {
        impl_read_u32!(self.scratchpad, SCRATCHPAD_MASK, address)
    }

    pub fn write_scratchpad_u8(&mut self, address: u32, value: u8) {
        impl_write_u8!(self.scratchpad, SCRATCHPAD_MASK, address, value);
    }

    pub fn write_scratchpad_u16(&mut self, address: u32, value: u16) {
        impl_write_u16!(self.scratchpad, SCRATCHPAD_MASK, address, value);
    }

    pub fn write_scratchpad_u32(&mut self, address: u32, value: u32) {
        impl_write_u32!(self.scratchpad, SCRATCHPAD_MASK, address, value);
    }

    pub fn copy_to_main_ram(&mut self, data: &[u8], ram_addr: u32) {
        self.main_ram[ram_addr as usize..ram_addr as usize + data.len()].copy_from_slice(data);
    }
}
