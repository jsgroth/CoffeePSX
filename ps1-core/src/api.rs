//! PS1 public interface and main loop

use crate::bus::Bus;
use crate::cd::CdController;
use crate::cpu::R3000;
use crate::dma::DmaController;
use crate::gpu::Gpu;
use crate::input::Ps1Inputs;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::memory::Memory;
use crate::scheduler::{Scheduler, SchedulerEvent, SchedulerEventType};
use crate::sio::SerialPort;
use crate::spu::Spu;
use crate::timers::Timers;
use bincode::{Decode, Encode};
use cdrom::reader::CdRom;
use cdrom::CdRomError;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct RenderParams {
    pub frame_x: u32,
    pub frame_y: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub display_x_offset: i32,
    pub display_y_offset: i32,
    pub display_width: u32,
    pub display_height: u32,
    pub display_enabled: bool,
}

pub trait Renderer {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while rendering the frame.
    fn render_frame(&mut self, vram: &[u8], params: RenderParams) -> Result<(), Self::Err>;
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
    #[error("CD-ROM error: {0}")]
    CdRom(#[from] CdRomError),
}

#[derive(Debug, Encode, Decode)]
pub struct Ps1Emulator {
    cpu: R3000,
    gpu: Gpu,
    spu: Spu,
    audio_buffer: Vec<(f64, f64)>,
    cd_controller: CdController,
    memory: Memory,
    dma_controller: DmaController,
    interrupt_registers: InterruptRegisters,
    sio0: SerialPort,
    timers: Timers,
    scheduler: Scheduler,
    last_render_cycles: u64,
    tty_enabled: bool,
    tty_buffer: String,
}

#[derive(Debug)]
pub struct Ps1EmulatorBuilder {
    bios_rom: Vec<u8>,
    disc: Option<CdRom>,
    tty_enabled: bool,
}

impl Ps1EmulatorBuilder {
    #[must_use]
    pub fn new(bios_rom: Vec<u8>) -> Self {
        Self { bios_rom, disc: None, tty_enabled: false }
    }

    #[must_use]
    pub fn with_disc(mut self, disc: CdRom) -> Self {
        self.disc = Some(disc);
        self
    }

    #[must_use]
    pub fn tty_enabled(mut self, tty_enabled: bool) -> Self {
        self.tty_enabled = tty_enabled;
        self
    }

    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn build(self) -> Ps1Result<Ps1Emulator> {
        Ps1Emulator::new(self.bios_rom, self.disc, self.tty_enabled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickEffect {
    None,
    FrameRendered,
}

// The SPU clock rate is exactly 1/768 the CPU clock rate
// This _should_ be 44100 Hz, but it may not be exactly depending on the exact oscillator speed
const SPU_CLOCK_DIVIDER: u64 = 768;

impl Ps1Emulator {
    #[must_use]
    pub fn builder(bios_rom: Vec<u8>) -> Ps1EmulatorBuilder {
        Ps1EmulatorBuilder::new(bios_rom)
    }

    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn new(bios_rom: Vec<u8>, disc: Option<CdRom>, tty_enabled: bool) -> Ps1Result<Self> {
        let memory = Memory::new(bios_rom)?;

        let mut emulator = Self {
            cpu: R3000::new(),
            gpu: Gpu::new(),
            spu: Spu::new(),
            audio_buffer: Vec::with_capacity(1600),
            cd_controller: CdController::new(disc),
            memory,
            dma_controller: DmaController::new(),
            interrupt_registers: InterruptRegisters::new(),
            sio0: SerialPort::new(),
            timers: Timers::new(),
            scheduler: Scheduler::new(),
            last_render_cycles: 0,
            tty_enabled,
            tty_buffer: String::new(),
        };
        emulator.schedule_initial_events();

        Ok(emulator)
    }

    fn schedule_initial_events(&mut self) {
        self.timers.schedule_next_vblank(&mut self.scheduler);
        self.scheduler.update_or_push_event(SchedulerEvent::spu_and_cd_clock(SPU_CLOCK_DIVIDER));
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
        self.memory.copy_to_main_ram(exe_data, ram_dest_addr & 0x1FFFFFFF);

        Ok(())
    }

    /// # Errors
    ///
    /// Will propagate any error encountered while rendering a frame.
    #[inline]
    pub fn tick<R: Renderer, A: AudioOutput>(
        &mut self,
        inputs: Ps1Inputs,
        renderer: &mut R,
        audio_output: &mut A,
    ) -> Result<TickEffect, TickError<R::Err, A::Err>> {
        self.cpu.execute_instruction(&mut Bus {
            gpu: &mut self.gpu,
            spu: &mut self.spu,
            cd_controller: &mut self.cd_controller,
            memory: &mut self.memory,
            dma_controller: &mut self.dma_controller,
            interrupt_registers: &mut self.interrupt_registers,
            sio0: &mut self.sio0,
            timers: &mut self.timers,
            scheduler: &mut self.scheduler,
        });

        if self.tty_enabled {
            self.check_for_putchar_call();
        }

        // Very, very rough timing: Assume that the CPU takes on average 2 cycles/instruction.
        // On actual hardware, timing varies depending on what memory was accessed (if any),
        // whether the opcode read hit in I-cache, and whether the instruction wrote to memory
        // while the write queue was full.
        let cpu_cycles = 2;

        self.scheduler.increment_cpu_cycles(cpu_cycles.into());

        // TODO use scheduler instead of advancing SIO0 every CPU tick
        self.sio0.tick(cpu_cycles, inputs, &mut self.interrupt_registers);

        let tick_effect = self.process_scheduler_events(renderer, audio_output)?;

        if self.scheduler.cpu_cycle_counter() - self.last_render_cycles >= 33_868_800 / 30 {
            // Force a frame render
            self.render_frame(renderer, audio_output)?;
            return Ok(TickEffect::FrameRendered);
        }

        Ok(tick_effect)
    }

    fn render_frame<R: Renderer, A: AudioOutput>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
    ) -> Result<(), TickError<R::Err, A::Err>> {
        self.last_render_cycles = self.scheduler.cpu_cycle_counter();

        renderer
            .render_frame(self.gpu.vram(), self.gpu.render_params())
            .map_err(TickError::Render)?;

        audio_output.queue_samples(&self.audio_buffer).map_err(TickError::Audio)?;
        self.audio_buffer.clear();

        Ok(())
    }

    fn process_scheduler_events<R: Renderer, A: AudioOutput>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
    ) -> Result<TickEffect, TickError<R::Err, A::Err>> {
        let mut tick_effect = TickEffect::None;

        while self.scheduler.is_event_ready() {
            let event = self.scheduler.pop_event();
            match event.event_type {
                SchedulerEventType::VBlank => {
                    self.interrupt_registers.set_interrupt_flag(InterruptType::VBlank);
                    self.timers.schedule_next_vblank(&mut self.scheduler);

                    self.render_frame(renderer, audio_output)?;

                    tick_effect = TickEffect::FrameRendered;
                }
                SchedulerEventType::SpuAndCdClock => {
                    self.cd_controller.clock(&mut self.interrupt_registers)?;
                    self.audio_buffer.push(self.spu.clock(&self.cd_controller));

                    self.scheduler.update_or_push_event(SchedulerEvent::spu_and_cd_clock(
                        event.cpu_cycles + SPU_CLOCK_DIVIDER,
                    ));
                }
                SchedulerEventType::Timer0Irq => {
                    self.interrupt_registers.set_interrupt_flag(InterruptType::Timer0);
                    self.timers.schedule_next_timer_0_irq(&mut self.scheduler);
                }
                SchedulerEventType::Timer1Irq => {
                    self.interrupt_registers.set_interrupt_flag(InterruptType::Timer1);
                    self.timers.scheduler_next_timer_1_irq(&mut self.scheduler);
                }
                SchedulerEventType::Timer2Irq => {
                    self.interrupt_registers.set_interrupt_flag(InterruptType::Timer2);
                    self.timers.schedule_next_timer_2_irq(&mut self.scheduler);
                }
            }
        }

        Ok(tick_effect)
    }

    fn check_for_putchar_call(&mut self) {
        // BIOS function calls work by jumping to $A0 (A functions), $B0 (B functions), or
        // $C0 (C functions) with the function number specified in R9.
        //
        // A($3C) and B($3D) are both the putchar() function, which prints the ASCII character
        // in R4 to the TTY.
        let pc = self.cpu.pc() & 0x1FFFFFFF;
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

    #[must_use]
    pub fn take_disc(&mut self) -> Option<CdRom> {
        self.cd_controller.take_disc()
    }

    pub fn set_disc(&mut self, disc: Option<CdRom>) {
        self.cd_controller.set_disc(disc);
    }
}
