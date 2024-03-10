use crate::bus::Bus;
use crate::cd::CdController;
use crate::control::ControlRegisters;
use crate::cpu::R3000;
use crate::dma::DmaController;
use crate::gpu::{Gpu, TickEffect};
use crate::input::Ps1Inputs;
use crate::memory::Memory;
use crate::spu::Spu;
use crate::timers::Timers;
use thiserror::Error;

pub trait Renderer {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while rendering the frame.
    fn render_frame(&mut self, vram: &[u8]) -> Result<(), Self::Err>;
}

pub trait AudioOutput {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while queueing the samples.
    fn queue_samples(&mut self, samples: &[(f64, f64)]) -> Result<(), Self::Err>;
}

#[derive(Debug, Error)]
pub enum Ps1Error {
    #[error("Incorrect BIOS ROM size; expected 512KB, was {bios_len}")]
    IncorrectBiosSize { bios_len: usize },
    #[error("EXE format is invalid")]
    InvalidExeFormat,
}

pub type Ps1Result<T> = Result<T, Ps1Error>;

#[derive(Debug, Error)]
pub enum TickError<RErr, AErr> {
    #[error("Error rendering frame: {0}")]
    Render(RErr),
    #[error("Error queueing audio samples: {0}")]
    Audio(AErr),
}

#[derive(Debug)]
pub struct Ps1Emulator {
    cpu: R3000,
    gpu: Gpu,
    spu: Spu,
    audio_buffer: Vec<(f64, f64)>,
    cd_controller: CdController,
    memory: Memory,
    dma_controller: DmaController,
    control_registers: ControlRegisters,
    timers: Timers,
    tty_enabled: bool,
    tty_buffer: String,
}

#[derive(Debug)]
pub struct Ps1EmulatorBuilder {
    bios_rom: Vec<u8>,
    tty_enabled: bool,
}

impl Ps1EmulatorBuilder {
    #[must_use]
    pub fn new(bios_rom: Vec<u8>) -> Self {
        Self {
            bios_rom,
            tty_enabled: false,
        }
    }

    #[must_use]
    pub fn tty_enabled(self, tty_enabled: bool) -> Self {
        Self {
            tty_enabled,
            ..self
        }
    }

    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn build(self) -> Ps1Result<Ps1Emulator> {
        Ps1Emulator::new(self.bios_rom, self.tty_enabled)
    }
}

impl Ps1Emulator {
    #[must_use]
    pub fn builder(bios_rom: Vec<u8>) -> Ps1EmulatorBuilder {
        Ps1EmulatorBuilder::new(bios_rom)
    }

    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn new(bios_rom: Vec<u8>, tty_enabled: bool) -> Ps1Result<Self> {
        let memory = Memory::new(bios_rom)?;

        Ok(Self {
            cpu: R3000::new(),
            gpu: Gpu::new(),
            spu: Spu::new(),
            audio_buffer: Vec::with_capacity(1600),
            cd_controller: CdController::new(),
            memory,
            dma_controller: DmaController::new(),
            control_registers: ControlRegisters::new(),
            timers: Timers::new(),
            tty_enabled,
            tty_buffer: String::new(),
        })
    }

    #[inline]
    #[must_use]
    pub fn cpu_pc(&self) -> u32 {
        self.cpu.pc()
    }

    /// # Errors
    ///
    /// Will return an error if the EXE does not appear to be a PS1 executable based on the header.
    #[allow(clippy::missing_panics_doc)]
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

    /// # Errors
    ///
    /// Will propagate any error encountered while rendering a frame.
    #[inline]
    pub fn tick<R: Renderer, A: AudioOutput>(
        &mut self,
        _inputs: Ps1Inputs,
        renderer: &mut R,
        audio_output: &mut A,
    ) -> Result<(), TickError<R::Err, A::Err>> {
        self.cpu.execute_instruction(&mut Bus {
            gpu: &mut self.gpu,
            spu: &mut self.spu,
            cd_controller: &mut self.cd_controller,
            memory: &mut self.memory,
            dma_controller: &mut self.dma_controller,
            control_registers: &mut self.control_registers,
            timers: &mut self.timers,
        });

        if self.tty_enabled {
            self.check_for_putchar_call();
        }

        // Very, very rough timing: Assume that the CPU takes on average 2 cycles/instruction.
        // On actual hardware, timing varies depending on what memory was accessed (if any),
        // whether the opcode read hit in I-cache, and whether the instruction wrote to memory
        // while the write queue was full.
        let cpu_cycles = 2;

        self.spu.tick(cpu_cycles, &mut self.audio_buffer);
        self.cd_controller
            .tick(cpu_cycles, &mut self.control_registers);

        if self
            .gpu
            .tick(cpu_cycles, &mut self.control_registers, &mut self.timers)
            == TickEffect::RenderFrame
        {
            renderer
                .render_frame(self.gpu.vram())
                .map_err(TickError::Render)?;

            audio_output
                .queue_samples(&self.audio_buffer)
                .map_err(TickError::Audio)?;
            self.audio_buffer.clear();
        }

        Ok(())
    }

    fn check_for_putchar_call(&mut self) {
        // BIOS function calls work by jumping to $A0 (A functions), $B0 (B functions), or
        // $C0 (C functions) with the function number specified in R9.
        //
        // A($3C) and B($3D) are both the putchar() function, which prints the ASCII character
        // in R4 to the TTY.
        let pc = self.cpu.pc() & 0x1FFFFFFF;
        if pc == 0xA0 || pc == 0xB0 {
            let r9 = self.cpu.get_gpr(9);
            if (pc == 0xA0 && r9 == 0x3C) || (pc == 0xB0 && r9 == 0x3D) {
                let r4 = self.cpu.get_gpr(4);
                let c = r4 as u8 as char;
                if c == '\n' {
                    println!("TTY: {}", self.tty_buffer);
                    self.tty_buffer.clear();
                } else {
                    self.tty_buffer.push(c);
                }
            }
        }
    }
}
