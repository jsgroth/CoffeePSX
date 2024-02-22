use crate::bus::Bus;
use crate::control::ControlRegisters;
use crate::cpu::R3000;
use crate::dma::DmaController;
use crate::memory::Memory;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Ps1Error {
    #[error("Incorrect BIOS ROM size; expected 512KB, was {bios_len}")]
    IncorrectBiosSize { bios_len: usize },
    #[error("EXE format is invalid")]
    InvalidExeFormat,
}

pub type Ps1Result<T> = Result<T, Ps1Error>;

#[derive(Debug)]
pub struct Ps1Emulator {
    cpu: R3000,
    memory: Memory,
    dma_controller: DmaController,
    control_registers: ControlRegisters,
}

impl Ps1Emulator {
    pub fn new(bios_rom: Vec<u8>) -> Ps1Result<Self> {
        let memory = Memory::new(bios_rom)?;

        Ok(Self {
            cpu: R3000::new(),
            memory,
            dma_controller: DmaController::new(),
            control_registers: ControlRegisters::new(),
        })
    }

    pub fn cpu_pc(&self) -> u32 {
        self.cpu.pc()
    }

    pub fn sideload_exe(&mut self, exe: &[u8]) -> Ps1Result<()> {
        if exe.len() < 0x800 || &exe[..0x008] != "PS-X EXE".as_bytes() {
            return Err(Ps1Error::InvalidExeFormat);
        }

        let pc = u32::from_le_bytes(exe[0x010..0x014].try_into().unwrap());
        let initial_gp = u32::from_le_bytes(exe[0x014..0x018].try_into().unwrap());
        let ram_dest_addr = u32::from_le_bytes(exe[0x018..0x01C].try_into().unwrap());
        let exe_size = u32::from_le_bytes(exe[0x01C..0x020].try_into().unwrap());
        let initial_sp = u32::from_le_bytes(exe[0x030..0x034].try_into().unwrap());
        let initial_sp_offset = u32::from_le_bytes(exe[0x034..0x038].try_into().unwrap());

        self.cpu.set_pc(pc);
        self.cpu.set_gpr(28, initial_gp);

        if initial_sp != 0 {
            self.cpu.set_gpr(29, initial_sp);
            self.cpu.set_gpr(30, initial_sp);
        }

        if initial_sp_offset != 0 {
            for r in [29, 30] {
                let r_value = self.cpu.get_gpr(r);
                self.cpu.set_gpr(r, r_value.wrapping_add(initial_sp_offset));
            }
        }

        let exe_data = &exe[0x800..0x800 + exe_size as usize];
        self.memory
            .copy_to_main_ram(exe_data, ram_dest_addr & 0x1FFFFFFF);

        Ok(())
    }

    pub fn tick(&mut self) {
        self.cpu.execute_instruction(&mut Bus {
            memory: &mut self.memory,
            dma_controller: &mut self.dma_controller,
            control_registers: &mut self.control_registers,
        });
    }
}
