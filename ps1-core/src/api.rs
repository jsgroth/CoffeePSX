//! PS1 public interface and main loop

use crate::bus::Bus;
use crate::cd::{CdController, CdControllerState};
use crate::cpu::R3000;
use crate::dma::DmaController;
use crate::gpu::Gpu;
use crate::gpu::GpuState;
use crate::input::Ps1Inputs;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::mdec::MacroblockDecoder;
use crate::memory::{Memory, MemoryControl};
use crate::scheduler::{Scheduler, SchedulerEvent, SchedulerEventType};
use crate::sio::SerialPort;
use crate::spu::Spu;
use crate::timers::Timers;
use bincode::{Decode, Encode};
use cdrom::reader::CdRom;
use cdrom::CdRomError;
use proc_macros::SaveState;
use std::fmt::{Display, Formatter};
use std::rc::Rc;
use thiserror::Error;

pub use crate::gpu::DisplayConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum ColorDepthBits {
    #[default]
    Fifteen = 0,
    TwentyFour = 1,
}

impl Display for ColorDepthBits {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fifteen => write!(f, "15-bit"),
            Self::TwentyFour => write!(f, "24-bit"),
        }
    }
}

impl ColorDepthBits {
    pub(crate) fn from_bit(bit: bool) -> Self {
        if bit { Self::TwentyFour } else { Self::Fifteen }
    }
}

pub trait Renderer {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while rendering the frame.
    fn render_frame(
        &mut self,
        frame: &wgpu::Texture,
        pixel_aspect_ratio: f64,
    ) -> Result<(), Self::Err>;
}

pub trait AudioOutput {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while queueing the samples.
    fn queue_samples(&mut self, samples: &[(f64, f64)]) -> Result<(), Self::Err>;
}

pub trait SaveWriter {
    type Err;

    /// # Errors
    ///
    /// Should propagate any error encountered while persisting the memory card.
    fn save_memory_card_1(&mut self, card_data: &[u8]) -> Result<(), Self::Err>;
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
pub enum TickError<RErr, AErr, SErr> {
    #[error("Error rendering frame: {0}")]
    Render(RErr),
    #[error("Error queueing audio samples: {0}")]
    Audio(AErr),
    #[error("Error saving memory card: {0}")]
    SaveWrite(SErr),
    #[error("CD-ROM error: {0}")]
    CdRom(#[from] CdRomError),
}

pub struct UnserializedFields {
    disc: Option<CdRom>,
    wgpu_device: Rc<wgpu::Device>,
    wgpu_queue: Rc<wgpu::Queue>,
    display_config: DisplayConfig,
}

#[derive(Debug, SaveState)]
pub struct Ps1Emulator {
    cpu: R3000,
    #[save_state(to = GpuState)]
    gpu: Gpu,
    spu: Spu,
    audio_buffer: Vec<(f64, f64)>,
    #[save_state(to = CdControllerState)]
    cd_controller: CdController,
    mdec: MacroblockDecoder,
    memory: Memory,
    memory_control: MemoryControl,
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
    wgpu_device: Rc<wgpu::Device>,
    wgpu_queue: Rc<wgpu::Queue>,
    display_config: DisplayConfig,
    disc: Option<CdRom>,
    memory_card_1: Option<Vec<u8>>,
    tty_enabled: bool,
}

impl Ps1EmulatorBuilder {
    #[must_use]
    pub fn new(
        bios_rom: Vec<u8>,
        wgpu_device: Rc<wgpu::Device>,
        wgpu_queue: Rc<wgpu::Queue>,
    ) -> Self {
        Self {
            bios_rom,
            wgpu_device,
            wgpu_queue,
            display_config: DisplayConfig::default(),
            disc: None,
            memory_card_1: None,
            tty_enabled: false,
        }
    }

    #[must_use]
    pub fn with_disc(mut self, disc: CdRom) -> Self {
        self.disc = Some(disc);
        self
    }

    #[must_use]
    pub fn with_memory_card_1(mut self, memory_card_1: Vec<u8>) -> Self {
        self.memory_card_1 = Some(memory_card_1);
        self
    }

    #[must_use]
    pub fn tty_enabled(mut self, tty_enabled: bool) -> Self {
        self.tty_enabled = tty_enabled;
        self
    }

    #[must_use]
    pub fn with_display_config(mut self, display_config: DisplayConfig) -> Self {
        self.display_config = display_config;
        self
    }

    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn build(self) -> Ps1Result<Ps1Emulator> {
        Ps1Emulator::new(
            self.bios_rom,
            self.wgpu_device,
            self.wgpu_queue,
            self.display_config,
            self.disc,
            self.memory_card_1,
            self.tty_enabled,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickEffect {
    None,
    FrameRendered,
}

// The SPU/CD-ROM clock rate is exactly 1/768 the CPU clock rate
// This _should_ be 44100 Hz, but it may not be exactly depending on the exact oscillator speed
const SPU_CLOCK_DIVIDER: u64 = 768;

impl Ps1Emulator {
    /// # Errors
    ///
    /// Will return an error if the BIOS ROM is invalid.
    pub fn new(
        bios_rom: Vec<u8>,
        wgpu_device: Rc<wgpu::Device>,
        wgpu_queue: Rc<wgpu::Queue>,
        display_config: DisplayConfig,
        disc: Option<CdRom>,
        memory_card_1: Option<Vec<u8>>,
        tty_enabled: bool,
    ) -> Ps1Result<Self> {
        let memory = Memory::new(bios_rom)?;

        let mut emulator = Self {
            cpu: R3000::new(),
            gpu: Gpu::new(wgpu_device, wgpu_queue, display_config),
            spu: Spu::new(),
            audio_buffer: Vec::with_capacity(1600),
            cd_controller: CdController::new(disc),
            mdec: MacroblockDecoder::new(),
            memory,
            memory_control: MemoryControl::new(),
            dma_controller: DmaController::new(),
            interrupt_registers: InterruptRegisters::new(),
            sio0: SerialPort::new(memory_card_1),
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
    #[allow(clippy::type_complexity)]
    pub fn tick<R: Renderer, A: AudioOutput, S: SaveWriter>(
        &mut self,
        inputs: Ps1Inputs,
        renderer: &mut R,
        audio_output: &mut A,
        save_writer: &mut S,
    ) -> Result<TickEffect, TickError<R::Err, A::Err, S::Err>> {
        let cpu_cycles = self.cpu.execute_instruction(&mut Bus {
            gpu: &mut self.gpu,
            spu: &mut self.spu,
            cd_controller: &mut self.cd_controller,
            mdec: &mut self.mdec,
            memory: &mut self.memory,
            memory_control: &mut self.memory_control,
            dma_controller: &mut self.dma_controller,
            interrupt_registers: &mut self.interrupt_registers,
            sio0: &mut self.sio0,
            timers: &mut self.timers,
            scheduler: &mut self.scheduler,
        });

        if self.tty_enabled {
            self.check_for_putchar_call();
        }

        self.scheduler.increment_cpu_cycles(cpu_cycles.into());

        // TODO use scheduler instead of advancing SIO0 every CPU tick
        self.sio0.tick(cpu_cycles, inputs, &mut self.interrupt_registers);

        let tick_effect = if self.scheduler.is_event_ready() {
            self.process_scheduler_events(renderer, audio_output, save_writer)?
        } else {
            TickEffect::None
        };

        if self.scheduler.cpu_cycle_counter() - self.last_render_cycles >= 33_868_800 / 30 {
            // Force a frame render
            // TODO handle this with the scheduler if the GPU stops generating VBlank IRQs due to
            // invalid Y1/Y2
            self.render_frame(renderer, audio_output, save_writer)?;
            return Ok(TickEffect::FrameRendered);
        }

        Ok(tick_effect)
    }

    #[allow(clippy::type_complexity)]
    fn render_frame<R: Renderer, A: AudioOutput, S: SaveWriter>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        save_writer: &mut S,
    ) -> Result<(), TickError<R::Err, A::Err, S::Err>> {
        self.last_render_cycles = self.scheduler.cpu_cycle_counter();

        let pixel_aspect_ratio = self.gpu.pixel_aspect_ratio();
        let frame = self.gpu.generate_frame_texture();
        renderer.render_frame(frame, pixel_aspect_ratio).map_err(TickError::Render)?;

        audio_output.queue_samples(&self.audio_buffer).map_err(TickError::Audio)?;
        self.audio_buffer.clear();

        let memory_card_1 = self.sio0.memory_card_1();
        if memory_card_1.get_and_clear_dirty() {
            save_writer.save_memory_card_1(memory_card_1.data()).map_err(TickError::SaveWrite)?;
        }

        Ok(())
    }

    #[inline]
    #[allow(clippy::type_complexity)]
    fn process_scheduler_events<R: Renderer, A: AudioOutput, S: SaveWriter>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        save_writer: &mut S,
    ) -> Result<TickEffect, TickError<R::Err, A::Err, S::Err>> {
        let mut tick_effect = TickEffect::None;

        while let Some(event) = self.scheduler.pop_ready_event() {
            match event.event_type {
                SchedulerEventType::VBlank => {
                    // VBlank event: Generate VBlank IRQ and render the current display frame buffer
                    // to video output.
                    // Triggers once per frame (when scanline == Y2) unless the GPU's vertical
                    // display range is invalid
                    self.interrupt_registers.set_interrupt_flag(InterruptType::VBlank);
                    self.timers.schedule_next_vblank(&mut self.scheduler);

                    self.render_frame(renderer, audio_output, save_writer)?;

                    tick_effect = TickEffect::FrameRendered;
                }
                SchedulerEventType::SpuAndCdClock => {
                    // SPU/CD-ROM clock event: Clock the CD-ROM controller and the SPU, then push
                    // the current stereo audio sample to audio output.
                    // Triggers every 768 CPU clocks which is 44100 Hz
                    self.cd_controller.clock(&mut self.interrupt_registers)?;
                    self.audio_buffer
                        .push(self.spu.clock(&self.cd_controller, &mut self.interrupt_registers));

                    self.scheduler.update_or_push_event(SchedulerEvent::spu_and_cd_clock(
                        event.cpu_cycles + SPU_CLOCK_DIVIDER,
                    ));
                }
                SchedulerEventType::Timer0Irq => {
                    // Timer 0 IRQ event: Set the Timer 0 IRQ flag (rarely used but can track the GPU dot clock).
                    // Trigger rate depends on Timer 0 configuration
                    self.interrupt_registers.set_interrupt_flag(InterruptType::Timer0);
                    self.timers.schedule_next_timer_0_irq(&mut self.scheduler);
                }
                SchedulerEventType::Timer1Irq => {
                    // Timer 1 IRQ event: Set the Timer 1 IRQ flag (usually counts GPU HBlank signals).
                    // Trigger rate depends on Timer 1 configuration
                    self.interrupt_registers.set_interrupt_flag(InterruptType::Timer1);
                    self.timers.scheduler_next_timer_1_irq(&mut self.scheduler);
                }
                SchedulerEventType::Timer2Irq => {
                    // Timer 2 IRQ event: Set the Timer 2 IRQ flag (usually ticks at system clock rate / 8).
                    // Trigger rate depends on Timer 2 configuration
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

    pub fn update_display_config(&mut self, display_config: DisplayConfig) {
        self.gpu.update_display_config(display_config);
    }

    #[must_use]
    pub fn take_unserialized_fields(&mut self) -> UnserializedFields {
        let (wgpu_device, wgpu_queue, display_config) = self.gpu.get_wgpu_resources();

        UnserializedFields {
            disc: self.cd_controller.take_disc(),
            wgpu_device,
            wgpu_queue,
            display_config,
        }
    }

    pub fn from_state(state: Ps1EmulatorState, unserialized: UnserializedFields) -> Self {
        Self {
            cpu: state.cpu,
            gpu: Gpu::from_state(
                state.gpu,
                unserialized.wgpu_device,
                unserialized.wgpu_queue,
                unserialized.display_config,
            ),
            spu: state.spu,
            audio_buffer: state.audio_buffer,
            cd_controller: CdController::from_state(state.cd_controller, unserialized.disc),
            mdec: state.mdec,
            memory: state.memory,
            memory_control: state.memory_control,
            dma_controller: state.dma_controller,
            interrupt_registers: state.interrupt_registers,
            sio0: state.sio0,
            timers: state.timers,
            scheduler: state.scheduler,
            last_render_cycles: state.last_render_cycles,
            tty_enabled: state.tty_enabled,
            tty_buffer: state.tty_buffer,
        }
    }
}
