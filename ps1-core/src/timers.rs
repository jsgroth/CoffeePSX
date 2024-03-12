//! PS1 hardware timers

use crate::interrupts::{InterruptRegisters, InterruptType};

#[derive(Debug, Clone)]
pub struct Timer {
    pub counter: u16,
}

impl Timer {
    pub fn write_mode(&mut self, _value: u32) {
        // TODO actually configure timer
        self.counter = 0;
    }

    pub fn increment(&mut self) {
        self.counter = self.counter.wrapping_add(1);
    }
}

const NTSC_GPU_CLOCK_SPEED: u64 = 53_693_175;
const CPU_CLOCK_SPEED: u64 = 44_100 * 768;

const NTSC_DOTS_PER_LINE: u16 = 3413;
const NTSC_LINES_PER_FRAME: u16 = 263;

#[derive(Debug, Clone)]
struct GpuTimer {
    gpu_cycles_product: u64,
    line: u16,
    line_gpu_cycles: u16,
    dot_gpu_cycles: u16,
    dot_clock_divider: u16,
    x1: u16,
    x2: u16,
    y1: u16,
    y2: u16,
    interlaced: bool,
    odd_frame: bool,
}

impl GpuTimer {
    fn new() -> Self {
        Self {
            gpu_cycles_product: 0,
            line: 0,
            line_gpu_cycles: 0,
            dot_gpu_cycles: 0,
            dot_clock_divider: 10,
            x1: 0x200,
            x2: 0x200 + 256 * 10,
            y1: 0x10,
            y2: 0x10 + 240,
            interlaced: false,
            odd_frame: false,
        }
    }

    fn tick(&mut self, cpu_cycles: u32, interrupt_registers: &mut InterruptRegisters) {
        self.gpu_cycles_product += u64::from(cpu_cycles) * NTSC_GPU_CLOCK_SPEED;
        while self.gpu_cycles_product >= CPU_CLOCK_SPEED {
            self.gpu_cycles_product -= CPU_CLOCK_SPEED;

            self.line_gpu_cycles += 1;
            if self.line_gpu_cycles == NTSC_DOTS_PER_LINE
                || (self.interlaced
                    && self.line == NTSC_LINES_PER_FRAME - 1
                    && self.line_gpu_cycles == NTSC_DOTS_PER_LINE / 2)
            {
                self.line_gpu_cycles = 0;
                self.dot_gpu_cycles = 0;

                self.line += 1;
                if self.line == NTSC_LINES_PER_FRAME {
                    self.line = 0;
                    self.odd_frame = !self.odd_frame;
                }

                if self.line == self.y2 {
                    interrupt_registers.set_interrupt_flag(InterruptType::VBlank);
                }
            } else {
                self.dot_gpu_cycles += 1;
                if self.dot_gpu_cycles >= self.dot_clock_divider {
                    self.dot_gpu_cycles -= self.dot_clock_divider;
                    // TODO tick dot clock
                }
            }

            // TODO in HBlank if !(X1..X2).contains(line_gpu_cycles)
        }
    }

    pub fn in_vblank(&self) -> bool {
        self.y1 > self.y2 || !(self.y1..self.y2).contains(&self.line)
    }

    fn in_hblank(&self) -> bool {
        self.x1 > self.x2 || !(self.x1..self.x2).contains(&self.line_gpu_cycles)
    }
}

#[derive(Debug, Clone)]
pub struct Timers {
    gpu: GpuTimer,
    // Horizontal retrace timer
    pub timer_1: Timer,
}

impl Timers {
    pub fn new() -> Self {
        Self {
            gpu: GpuTimer::new(),
            timer_1: Timer { counter: 0 },
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32, interrupt_registers: &mut InterruptRegisters) {
        self.gpu.tick(cpu_cycles, interrupt_registers);
    }

    pub fn in_vblank(&self) -> bool {
        self.gpu.in_vblank()
    }

    pub fn odd_frame(&self) -> bool {
        self.gpu.odd_frame
    }

    pub fn scanline(&self) -> u16 {
        self.gpu.line
    }

    pub fn update_display_mode(&mut self, dot_clock_divider: u16, interlaced: bool) {
        self.gpu.dot_clock_divider = dot_clock_divider;
        self.gpu.interlaced = interlaced;
    }

    pub fn update_h_display_area(&mut self, x1: u16, x2: u16) {
        self.gpu.x1 = x1;
        self.gpu.x2 = x2;
    }

    pub fn update_v_display_area(&mut self, y1: u16, y2: u16) {
        self.gpu.y1 = y1;
        self.gpu.y2 = y2;
    }

    pub fn write_register(&mut self, address: u32, value: u32) {
        let timer_idx = (address >> 4) & 3;
        if timer_idx != 1 {
            log::warn!("Unhandled timer {timer_idx} write: {address:08X} {value:08X}");
            return;
        }

        match address & 0xF {
            0x0 => {
                self.timer_1.counter = value as u16;
                log::trace!("Timer 1 counter write: {:04X}", self.timer_1.counter);
            }
            0x4 => {
                self.timer_1.write_mode(value);
                log::trace!("Timer 1 mode write: {value:08X}");
            }
            0x8 => {
                log::warn!("Unhandled timer 1 target write: {value:08X}");
            }
            _ => todo!("timer register write {address:08X} {value:08X}"),
        }
    }
}
