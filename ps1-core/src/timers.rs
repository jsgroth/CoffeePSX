//! PS1 hardware timers

use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::num::{U32Ext, U8Ext};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ResetMode {
    #[default]
    Overflow = 0,
    Target = 1,
}

impl ResetMode {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Target
        } else {
            Self::Overflow
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum IrqMode {
    #[default]
    Once = 0,
    Repeat = 1,
}

impl IrqMode {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Repeat
        } else {
            Self::Once
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum IrqPulseMode {
    #[default]
    ShortPulse = 0,
    Toggle = 1,
}

impl IrqPulseMode {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Toggle
        } else {
            Self::ShortPulse
        }
    }
}

#[derive(Debug, Clone)]
struct Timer {
    idx: u8,
    interrupt_type: InterruptType,
    counter: u16,
    wait_cycle: bool,
    target_value: u16,
    sync_enabled: bool,
    sync_mode: u8,
    reset_mode: ResetMode,
    target_irq_enabled: bool,
    overflow_irq_enabled: bool,
    irq_mode: IrqMode,
    irq_pulse_mode: IrqPulseMode,
    clock_source: u8,
    irq: bool,
    irq_since_mode_write: bool,
    reached_target: bool,
    overflowed: bool,
}

impl Timer {
    fn new(idx: u8, interrupt_type: InterruptType) -> Self {
        Self {
            idx,
            interrupt_type,
            counter: 0,
            wait_cycle: false,
            target_value: 0,
            sync_enabled: false,
            sync_mode: 0,
            reset_mode: ResetMode::default(),
            overflow_irq_enabled: false,
            target_irq_enabled: false,
            irq_mode: IrqMode::default(),
            irq_pulse_mode: IrqPulseMode::default(),
            clock_source: 0,
            irq: false,
            irq_since_mode_write: false,
            reached_target: false,
            overflowed: false,
        }
    }

    pub fn write_mode(&mut self, value: u32) {
        self.sync_enabled = value.bit(0);
        self.sync_mode = ((value >> 1) & 3) as u8;
        self.reset_mode = ResetMode::from_bit(value.bit(3));
        self.target_irq_enabled = value.bit(4);
        self.overflow_irq_enabled = value.bit(5);
        self.irq_mode = IrqMode::from_bit(value.bit(6));
        self.irq_pulse_mode = IrqPulseMode::from_bit(value.bit(7));
        self.clock_source = ((value >> 8) & 3) as u8;

        if value.bit(10) || self.irq_pulse_mode == IrqPulseMode::ShortPulse {
            self.irq = false;
        }

        self.counter = 0;
        self.wait_cycle = true;
        self.irq_since_mode_write = false;

        log::debug!("Timer {} mode write: {value:04X}", self.idx);
        log::debug!("  Sync enabled: {}", self.sync_enabled);
        log::debug!("  Sync mode: {}", self.sync_mode);
        log::debug!("  Reset mode: {:?}", self.reset_mode);
        log::debug!("  IRQ at counter=target: {}", self.target_irq_enabled);
        log::debug!("  IRQ at counter=$FFFF: {}", self.overflow_irq_enabled);
        log::debug!("  IRQ mode: {:?}", self.irq_mode);
        log::debug!("  IRQ pulse mode: {:?}", self.irq_pulse_mode);
        log::debug!("  Clock source: {}", self.clock_source);
    }

    pub fn increment(&mut self, interrupt_registers: &mut InterruptRegisters) {
        if self.wait_cycle {
            self.wait_cycle = false;
            return;
        }

        if self.reset_mode == ResetMode::Target && self.counter == self.target_value {
            self.counter = 0;
            self.wait_cycle = true;
            return;
        }

        self.counter = self.counter.wrapping_add(1);

        let reached_target = self.counter == self.target_value;
        let reached_max = self.counter == 0xFFFF;

        self.reached_target |= reached_target;
        self.overflowed |= reached_max;

        let irq_triggered = (self.target_irq_enabled && reached_target)
            || (self.overflow_irq_enabled && reached_max);
        if irq_triggered && (self.irq_mode == IrqMode::Repeat || !self.irq_since_mode_write) {
            self.irq_since_mode_write = true;

            match self.irq_pulse_mode {
                IrqPulseMode::ShortPulse => {
                    interrupt_registers.set_interrupt_flag(self.interrupt_type);
                }
                IrqPulseMode::Toggle => {
                    self.irq = !self.irq;
                    if self.irq {
                        interrupt_registers.set_interrupt_flag(self.interrupt_type);
                    }
                }
            }
        }
    }

    fn write_counter(&mut self, counter: u16) {
        self.counter = counter;
        self.wait_cycle = true;

        log::debug!("Timer {} counter write: {counter:04X}", self.idx);
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

    fn tick(
        &mut self,
        cpu_cycles: u32,
        timers: &mut [Timer; 3],
        interrupt_registers: &mut InterruptRegisters,
    ) {
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
                    // TODO sync modes
                    if timers[0].clock_source & 1 != 0 {
                        timers[0].increment(interrupt_registers);
                    }
                }
            }

            if self.line_gpu_cycles == self.x2 {
                // TODO sync modes
                if timers[1].clock_source & 1 != 0 {
                    timers[1].increment(interrupt_registers);
                }
            }
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
    timers: [Timer; 3],
    sysclk_div_8: u32,
}

impl Timers {
    pub fn new() -> Self {
        Self {
            gpu: GpuTimer::new(),
            timers: [
                Timer::new(0, InterruptType::Timer0),
                Timer::new(1, InterruptType::Timer1),
                Timer::new(2, InterruptType::Timer2),
            ],
            sysclk_div_8: 0,
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32, interrupt_registers: &mut InterruptRegisters) {
        self.gpu
            .tick(cpu_cycles, &mut self.timers, interrupt_registers);

        for timer_idx in [0, 1] {
            let timer = &mut self.timers[timer_idx];
            if timer.clock_source & 1 == 0 {
                for _ in 0..cpu_cycles {
                    timer.increment(interrupt_registers);
                }
            }
        }

        self.sysclk_div_8 += cpu_cycles;
        if !(self.timers[2].sync_enabled
            && (self.timers[2].sync_mode == 0 || self.timers[2].sync_mode == 3))
        {
            if self.timers[2].clock_source.bit(1) {
                // CPU clock / 8
                while self.sysclk_div_8 >= 8 {
                    self.sysclk_div_8 -= 8;
                    self.timers[2].increment(interrupt_registers);
                }
            } else {
                // CPU clock
                for _ in 0..cpu_cycles {
                    self.timers[2].increment(interrupt_registers);
                }
                self.sysclk_div_8 &= 7;
            }
        }
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

    pub fn read_register(&self, address: u32) -> u32 {
        let timer_idx = ((address >> 4) & 3) as usize;
        if timer_idx == 3 {
            return 0;
        }

        match address & 0xF {
            0x0 => self.timers[timer_idx].counter.into(),
            _ => todo!("timer register read {address:08X}"),
        }
    }

    pub fn write_register(&mut self, address: u32, value: u32) {
        let timer_idx = ((address >> 4) & 3) as usize;
        if timer_idx == 3 {
            return;
        }

        match address & 0xF {
            0x0 => {
                self.timers[timer_idx].write_counter(value as u16);
            }
            0x4 => {
                self.timers[timer_idx].write_mode(value);
            }
            0x8 => {
                self.timers[timer_idx].target_value = value as u16;
                log::debug!("Timer {timer_idx} target value: {value:04X}");
            }
            _ => todo!("timer register write {address:08X} {value:08X}"),
        }
    }
}
