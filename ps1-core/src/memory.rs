//! PS1 system memory (main RAM / scratchpad / BIOS ROM) and memory control registers

use crate::api::{Ps1Error, Ps1Result};
use crate::boxedarray::BoxedArray;
use crate::num::U32Ext;
use bincode::{Decode, Encode};

const BIOS_ROM_LEN: usize = 512 * 1024;
const MAIN_RAM_LEN: usize = 2 * 1024 * 1024;
const SCRATCHPAD_LEN: usize = 1024;

const BIOS_ROM_MASK: u32 = (BIOS_ROM_LEN - 1) as u32;
pub const MAIN_RAM_MASK: u32 = (MAIN_RAM_LEN - 1) as u32;
const SCRATCHPAD_MASK: u32 = (SCRATCHPAD_LEN - 1) as u32;

type BiosRom = BoxedArray<u8, BIOS_ROM_LEN>;
type MainRam = BoxedArray<u8, MAIN_RAM_LEN>;
type Scratchpad = BoxedArray<u8, SCRATCHPAD_LEN>;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Memory {
    bios_rom: BiosRom,
    main_ram: MainRam,
    scratchpad: Scratchpad,
}

macro_rules! impl_read_u8 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {
        $memory[($address & $addr_mask) as usize]
    };
}

macro_rules! impl_read_u16 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {{
        let address = ($address & $addr_mask & !1) as usize;
        u16::from_le_bytes([$memory[address], $memory[address + 1]])
    }};
}

macro_rules! impl_read_u32 {
    ($memory:expr, $addr_mask:expr, $address:expr) => {{
        let address = ($address & $addr_mask & !3) as usize;
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
        let address = ($address & $addr_mask & !1) as usize;
        $memory[address] = lsb;
        $memory[address + 1] = msb;
    }};
}

macro_rules! impl_write_u32 {
    ($memory:expr, $addr_mask: expr, $address:expr, $value:expr) => {{
        let bytes = $value.to_le_bytes();
        let address = ($address & $addr_mask & !3) as usize;
        for i in 0..4 {
            $memory[address + i] = bytes[i];
        }
    }};
}

impl Memory {
    pub fn new(bios_rom: Vec<u8>) -> Ps1Result<Self> {
        if bios_rom.len() != BIOS_ROM_LEN {
            return Err(Ps1Error::IncorrectBiosSize { bios_len: bios_rom.len() });
        }

        let bios_rom: Box<[u8; BIOS_ROM_LEN]> = bios_rom.into_boxed_slice().try_into().unwrap();

        let mut main_ram = MainRam::new();
        main_ram.fill_with(rand::random);

        let mut scratchpad = Scratchpad::new();
        scratchpad.fill_with(rand::random);

        Ok(Self { bios_rom: BiosRom::from(bios_rom), main_ram, scratchpad })
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

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct CommonDelay {
    pub recovery_cycles: u8,
    pub hold_cycles: u8,
    pub floating_cycles: u8,
    pub pre_strobe_cycles: u8,
}

impl Default for CommonDelay {
    fn default() -> Self {
        Self { recovery_cycles: 16, hold_cycles: 16, floating_cycles: 16, pre_strobe_cycles: 16 }
    }
}

impl From<u32> for CommonDelay {
    fn from(value: u32) -> Self {
        Self {
            recovery_cycles: (value & 0xF) as u8 + 1,
            hold_cycles: ((value >> 4) & 0xF) as u8 + 1,
            floating_cycles: ((value >> 8) & 0xF) as u8 + 1,
            pre_strobe_cycles: ((value >> 12) & 0xF) as u8 + 1,
        }
    }
}

impl From<CommonDelay> for u32 {
    fn from(value: CommonDelay) -> Self {
        u32::from(value.recovery_cycles - 1)
            | (u32::from(value.hold_cycles - 1) << 4)
            | (u32::from(value.floating_cycles - 1) << 8)
            | (u32::from(value.pre_strobe_cycles - 1) << 12)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum DataBusWidthBits {
    Eight = 0,
    Sixteen = 1,
}

impl DataBusWidthBits {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Sixteen } else { Self::Eight }
    }
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct DeviceDelay {
    pub write_delay: u8,
    pub read_delay: u8,
    pub recovery_delay: bool,
    pub hold_delay: bool,
    pub floating_delay: bool,
    pub pre_strobe_delay: bool,
    pub data_bus_width: DataBusWidthBits,
    pub auto_increment: bool,
    pub unknown_rw_bits: u8,
    pub memory_window_size_exponent: u8,
    pub dma_cycles_override: u8,
    pub use_dma_cycle_override: bool,
    pub wide_dma: bool,
    pub wait_on_device: bool,
}

impl Default for DeviceDelay {
    fn default() -> Self {
        Self {
            read_delay: 16,
            write_delay: 16,
            recovery_delay: false,
            hold_delay: false,
            floating_delay: false,
            pre_strobe_delay: false,
            data_bus_width: DataBusWidthBits::Eight,
            auto_increment: false,
            unknown_rw_bits: 0,
            memory_window_size_exponent: 0,
            dma_cycles_override: 16,
            use_dma_cycle_override: false,
            wide_dma: false,
            wait_on_device: false,
        }
    }
}

impl From<u32> for DeviceDelay {
    fn from(value: u32) -> Self {
        Self {
            write_delay: (value & 0xF) as u8 + 1,
            read_delay: ((value >> 4) & 0xF) as u8 + 1,
            recovery_delay: value.bit(8),
            hold_delay: value.bit(9),
            floating_delay: value.bit(10),
            pre_strobe_delay: value.bit(11),
            data_bus_width: DataBusWidthBits::from_bit(value.bit(12)),
            auto_increment: value.bit(13),
            unknown_rw_bits: ((value >> 14) & 0x3) as u8,
            memory_window_size_exponent: ((value >> 16) & 0x1F) as u8,
            dma_cycles_override: ((value >> 24) & 0xF) as u8 + 1,
            use_dma_cycle_override: value.bit(29),
            wide_dma: value.bit(30),
            wait_on_device: value.bit(31),
        }
    }
}

impl From<DeviceDelay> for u32 {
    fn from(value: DeviceDelay) -> Self {
        (u32::from(value.write_delay - 1))
            | (u32::from(value.read_delay - 1) << 4)
            | (u32::from(value.recovery_delay) << 8)
            | (u32::from(value.hold_delay) << 9)
            | (u32::from(value.floating_delay) << 10)
            | (u32::from(value.pre_strobe_delay) << 11)
            | ((value.data_bus_width as u32) << 12)
            | (u32::from(value.auto_increment) << 13)
            | (u32::from(value.unknown_rw_bits) << 14)
            | (u32::from(value.memory_window_size_exponent) << 16)
            | (u32::from(value.dma_cycles_override - 1) << 24)
            | (u32::from(value.use_dma_cycle_override) << 29)
            | (u32::from(value.wide_dma) << 30)
            | (u32::from(value.wait_on_device) << 31)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum MainRamWindow {
    // 2MB + 6MB locked
    Single = 4,
    // 8MB
    #[default]
    FourMirrors = 5,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MemoryControl {
    pub common: CommonDelay,
    pub expansion_1: DeviceDelay,
    pub expansion_2: DeviceDelay,
    pub expansion_3: DeviceDelay,
    pub bios: DeviceDelay,
    pub spu: DeviceDelay,
    pub cdrom: DeviceDelay,
    pub ram_size: MainRamWindow,
}

impl MemoryControl {
    pub fn new() -> Self {
        Self {
            common: CommonDelay::default(),
            expansion_1: DeviceDelay::default(),
            expansion_2: DeviceDelay::default(),
            expansion_3: DeviceDelay::default(),
            bios: DeviceDelay::default(),
            spu: DeviceDelay::default(),
            cdrom: DeviceDelay::default(),
            ram_size: MainRamWindow::default(),
        }
    }

    pub fn read_register(&self, address: u32) -> u32 {
        match address & 0xFFFF {
            0x1000 => {
                log::warn!("Unimplemented Expansion 1 base address read, returning $1F000000");
                0x1F000000
            }
            0x1004 => {
                log::warn!("Unimplemented Expansion 2 base address read, returning $1F802000");
                0x1F802000
            }
            0x1008 => self.expansion_1.into(),
            0x100C => self.expansion_3.into(),
            0x1010 => self.bios.into(),
            0x1014 => self.spu.into(),
            0x1018 => self.cdrom.into(),
            0x101C => self.expansion_2.into(),
            0x1020 => self.common.into(),
            _ => panic!("Invalid memory control read: {address:08X}"),
        }
    }

    pub fn write_register(&mut self, address: u32, value: u32) {
        match address & 0xFFFF {
            0x1000 => {
                log::warn!("Unimplemented Expansion 1 base address write: {value:08X}");
            }
            0x1004 => {
                log::warn!("Unimplemented Expansion 2 base address write: {value:08X}");
            }
            0x1008 => {
                self.expansion_1 = value.into();
                log::debug!("Expansion 1 delay write ({value:08X}): {:?}", self.expansion_1);
            }
            0x100C => {
                self.expansion_3 = value.into();
                log::debug!("Expansion 3 delay write ({value:08X}): {:?}", self.expansion_3);
            }
            0x1010 => {
                self.bios = value.into();
                log::debug!("BIOS ROM delay write ({value:08X}): {:?}", self.bios);
            }
            0x1014 => {
                self.spu = value.into();
                log::debug!("SPU delay write ({value:08X}): {:?}", self.spu);
            }
            0x1018 => {
                self.cdrom = value.into();
                log::debug!("CD-ROM delay write ({value:08X}): {:?}", self.cdrom);
            }
            0x101C => {
                self.expansion_2 = value.into();
                log::debug!("Expansion 2 delay write ({value:08X}): {:?}", self.expansion_2);
            }
            0x1020 => {
                self.common = value.into();
                log::debug!("Common delay write ({value:08X}): {:?}", self.common);
            }
            _ => panic!("Invalid memory control write: {address:08X} {value:08X}"),
        }
    }

    pub fn read_ram_size(&self) -> u32 {
        (self.ram_size as u32) << 9
    }

    pub fn write_ram_size(&mut self, value: u32) {
        log::debug!("RAM size write: {value:08X}");

        self.ram_size = match (value >> 9) & 7 {
            4 => MainRamWindow::Single,
            5 => MainRamWindow::FourMirrors,
            window => {
                log::warn!("Unexpected main RAM window size value: {window}");
                MainRamWindow::default()
            }
        };
    }
}
