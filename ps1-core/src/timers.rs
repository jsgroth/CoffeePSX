//! PS1 hardware timers

use crate::num::U32Ext;
use crate::scheduler::{Scheduler, SchedulerEvent, SchedulerEventType};
use std::cmp;

const NTSC_GPU_CLOCK_SPEED: u64 = 53_693_175;
const CPU_CLOCK_SPEED: u64 = 44_100 * 768;

const NTSC_DOTS_PER_LINE: u64 = 3413;
const NTSC_LINES_PER_FRAME: u64 = 263;

const PROGRESSIVE_CYCLES_PER_FRAME: u64 = NTSC_LINES_PER_FRAME * NTSC_DOTS_PER_LINE;
const INTERLACED_CYCLES_PER_FRAME: u64 =
    (NTSC_LINES_PER_FRAME - 1) * NTSC_DOTS_PER_LINE + NTSC_DOTS_PER_LINE / 2;

#[derive(Debug, Clone)]
struct GpuTimer {
    last_update_cpu_cycles: u64,
    leftover_product_cycles: u64,
    frame_gpu_cycles: u64,
    vblank_start_gpu_cycle: u64,
    vblank_end_gpu_cycle: u64,
    interlaced: bool,
    odd_frame: bool,
    dot_clock_divider: u64,
    dot_clock: u64,
    hblank_clock: u64,
}

impl GpuTimer {
    fn new() -> Self {
        Self {
            last_update_cpu_cycles: 0,
            leftover_product_cycles: 0,
            frame_gpu_cycles: 0,
            vblank_start_gpu_cycle: 256 * NTSC_DOTS_PER_LINE,
            vblank_end_gpu_cycle: 16 * NTSC_DOTS_PER_LINE,
            interlaced: false,
            odd_frame: false,
            dot_clock_divider: 10,
            dot_clock: 0,
            hblank_clock: 0,
        }
    }

    fn cycles_per_frame(&self) -> u64 {
        if self.interlaced {
            INTERLACED_CYCLES_PER_FRAME
        } else {
            PROGRESSIVE_CYCLES_PER_FRAME
        }
    }

    fn catch_up(&mut self, cpu_cycle_counter: u64) {
        if cpu_cycle_counter == self.last_update_cpu_cycles {
            return;
        }

        let elapsed_cpu_cycles = cpu_cycle_counter - self.last_update_cpu_cycles;
        self.last_update_cpu_cycles = cpu_cycle_counter;

        self.leftover_product_cycles += elapsed_cpu_cycles * NTSC_GPU_CLOCK_SPEED;
        let mut elapsed_gpu_cycles = self.leftover_product_cycles / CPU_CLOCK_SPEED;
        self.leftover_product_cycles %= CPU_CLOCK_SPEED;

        // Special case the first line because later lines will always start from dot 0
        let mut line = self.frame_gpu_cycles / NTSC_DOTS_PER_LINE;
        let first_line_gpu_cycles = self.frame_gpu_cycles % NTSC_DOTS_PER_LINE;
        let dots_in_first_line: u64 = if self.interlaced && line == NTSC_LINES_PER_FRAME - 1 {
            NTSC_DOTS_PER_LINE / 2
        } else {
            NTSC_DOTS_PER_LINE
        };
        if elapsed_gpu_cycles < dots_in_first_line - first_line_gpu_cycles {
            self.frame_gpu_cycles += elapsed_gpu_cycles;
            self.dot_clock += (first_line_gpu_cycles + elapsed_gpu_cycles) / self.dot_clock_divider
                - first_line_gpu_cycles / self.dot_clock_divider;
            return;
        }

        elapsed_gpu_cycles -= dots_in_first_line - first_line_gpu_cycles;
        line += 1;
        if line == NTSC_LINES_PER_FRAME {
            line = 0;
            self.frame_gpu_cycles = 0;
            self.odd_frame = !self.odd_frame;
        }
        self.frame_gpu_cycles += dots_in_first_line - first_line_gpu_cycles;
        self.dot_clock += dots_in_first_line / self.dot_clock_divider
            - first_line_gpu_cycles / self.dot_clock_divider;
        self.hblank_clock += 1;

        loop {
            let mut dots_in_line: u64 = NTSC_DOTS_PER_LINE;
            if self.interlaced && line == NTSC_LINES_PER_FRAME - 1 {
                dots_in_line /= 2;
            }

            if elapsed_gpu_cycles < dots_in_line {
                self.frame_gpu_cycles += elapsed_gpu_cycles;
                self.dot_clock += elapsed_gpu_cycles / self.dot_clock_divider;
                return;
            }

            elapsed_gpu_cycles -= dots_in_line;
            line += 1;
            if line == NTSC_LINES_PER_FRAME {
                line = 0;
                self.frame_gpu_cycles = 0;
                self.odd_frame = !self.odd_frame;
            }
            self.frame_gpu_cycles += dots_in_line;
            self.dot_clock += dots_in_line / self.dot_clock_divider;
            self.hblank_clock += 1;
        }
    }

    pub fn schedule_next_vblank(&mut self, scheduler: &mut Scheduler) {
        let cpu_cycle_counter = scheduler.cpu_cycle_counter();
        self.catch_up(cpu_cycle_counter);

        let cycles_per_frame = self.cycles_per_frame();

        if self.vblank_start_gpu_cycle >= cycles_per_frame {
            scheduler.remove_event(SchedulerEventType::VBlank);
            return;
        }

        let gpu_cycles_till_vblank = if self.frame_gpu_cycles >= self.vblank_start_gpu_cycle {
            self.vblank_start_gpu_cycle + (cycles_per_frame - self.frame_gpu_cycles)
        } else {
            self.vblank_start_gpu_cycle - self.frame_gpu_cycles
        };

        let cpu_cycles_till_vblank = (gpu_cycles_till_vblank * CPU_CLOCK_SPEED
            - self.leftover_product_cycles)
            / NTSC_GPU_CLOCK_SPEED
            + 1;
        scheduler.update_or_push_event(SchedulerEvent::vblank(
            cpu_cycle_counter + cpu_cycles_till_vblank,
        ));
    }

    pub fn update_v_display_range(&mut self, y1: u16, y2: u16, scheduler: &mut Scheduler) {
        self.catch_up(scheduler.cpu_cycle_counter());

        if y1 > y2 || y2 >= NTSC_LINES_PER_FRAME as u16 {
            log::error!("Invalid Y display range: [{y1}, {y2}]");
            self.vblank_start_gpu_cycle = u64::MAX;
            self.vblank_end_gpu_cycle = 0;
            return;
        }

        self.vblank_start_gpu_cycle = u64::from(y2) * NTSC_LINES_PER_FRAME;
        self.vblank_end_gpu_cycle = u64::from(y1) * NTSC_LINES_PER_FRAME;

        self.schedule_next_vblank(scheduler);
    }

    pub fn update_display_mode(
        &mut self,
        dot_clock_divider: u64,
        interlaced: bool,
        scheduler: &mut Scheduler,
    ) {
        self.catch_up(scheduler.cpu_cycle_counter());

        self.dot_clock_divider = dot_clock_divider;
        self.interlaced = interlaced;

        if self.interlaced && self.frame_gpu_cycles >= INTERLACED_CYCLES_PER_FRAME {
            self.frame_gpu_cycles = 0;
        }

        self.schedule_next_vblank(scheduler);
    }

    fn scanline(&self) -> u16 {
        (self.frame_gpu_cycles / NTSC_DOTS_PER_LINE) as u16
    }

    fn in_vblank(&self) -> bool {
        !(self.vblank_end_gpu_cycle..self.vblank_start_gpu_cycle).contains(&self.frame_gpu_cycles)
    }
}

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
enum ClockSource {
    Dot,
    HBlank,
    #[default]
    System,
    SystemDiv8,
}

#[derive(Debug, Clone)]
struct SystemTimer {
    idx: u8,
    interrupt_type: SchedulerEventType,
    counter: u16,
    wait_cycle: bool,
    clock_source: ClockSource,
    last_update_clock: u64,
    target_value: u16,
    reset_mode: ResetMode,
    target_irq_enabled: bool,
    overflow_irq_enabled: bool,
    irq_mode: IrqMode,
    irq: bool,
    irq_since_mode_write: bool,
}

impl SystemTimer {
    fn new(idx: u8, interrupt_type: SchedulerEventType) -> Self {
        Self {
            idx,
            interrupt_type,
            counter: 0,
            wait_cycle: false,
            clock_source: ClockSource::default(),
            last_update_clock: 0,
            target_value: 0,
            reset_mode: ResetMode::default(),
            target_irq_enabled: false,
            overflow_irq_enabled: false,
            irq_mode: IrqMode::default(),
            irq: false,
            irq_since_mode_write: false,
        }
    }

    fn catch_up(&mut self, scheduler: &Scheduler, gpu_timer: &GpuTimer) {
        let clock = match self.clock_source {
            ClockSource::Dot => gpu_timer.dot_clock,
            ClockSource::HBlank => gpu_timer.hblank_clock,
            ClockSource::System => scheduler.cpu_cycle_counter(),
            ClockSource::SystemDiv8 => scheduler.cpu_cycle_counter() / 8,
        };

        if clock == self.last_update_clock {
            return;
        }

        let mut elapsed = clock - self.last_update_clock;
        self.last_update_clock = clock;

        if self.wait_cycle {
            self.wait_cycle = false;
            elapsed -= 1;
        }

        if self
            .clocks_until_irq()
            .is_some_and(|clocks| clocks <= elapsed)
        {
            self.irq = true;
            self.irq_since_mode_write = true;
        }

        match self.reset_mode {
            ResetMode::Overflow => {
                self.counter = self.counter.wrapping_add(elapsed as u16);
            }
            ResetMode::Target => {
                // TODO optimize this
                for _ in 0..elapsed {
                    if self.counter == self.target_value {
                        self.counter = 0;
                        self.wait_cycle = true;
                    } else if self.wait_cycle {
                        self.wait_cycle = false;
                    } else {
                        self.counter = self.counter.wrapping_add(1);
                    }
                }
            }
        }
    }

    fn clocks_until_irq(&self) -> Option<u64> {
        if self.irq_mode == IrqMode::Once && self.irq_since_mode_write {
            return None;
        }

        let max_value = match self.reset_mode {
            ResetMode::Target => self.target_value,
            ResetMode::Overflow => 0xFFFF,
        };

        let clocks_until_target_irq = self.target_irq_enabled.then(|| {
            if self.counter >= self.target_value {
                u64::from(max_value - self.counter) + u64::from(self.target_value) + 2
            } else {
                u64::from(self.target_value - self.counter)
            }
        });
        let clocks_until_overflow_irq = (self.overflow_irq_enabled && max_value == 0xFFFF)
            .then_some(if self.counter == 0xFFFF {
                0x10000_u64
            } else {
                (0xFFFF - self.counter).into()
            });

        match (clocks_until_target_irq, clocks_until_overflow_irq) {
            (Some(a), Some(b)) => Some(cmp::min(a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    fn write_counter(&mut self, value: u32) {
        self.counter = value as u16;
        self.wait_cycle = true;

        log::debug!("Timer {} counter write: {value:04X}", self.idx);
    }

    fn write_mode(&mut self, value: u32, scheduler: &Scheduler, gpu_timer: &GpuTimer) {
        if value.bit(0) {
            log::error!(
                "Sync mode enabled for timer {} with sync mode {}",
                self.idx,
                (value >> 1) & 3
            );
        }

        if value.bit(7) {
            log::error!(
                "IRQ toggle mode enabled for timer {}, not implemented",
                self.idx
            );
        }

        self.reset_mode = ResetMode::from_bit(value.bit(3));
        self.target_irq_enabled = value.bit(4);
        self.overflow_irq_enabled = value.bit(5);
        self.irq_mode = IrqMode::from_bit(value.bit(6));

        let raw_clock_source = (value >> 8) & 3;
        self.clock_source = match (self.idx, raw_clock_source) {
            (0 | 1, 0 | 2) | (2, 0 | 1) => ClockSource::System,
            (0, 1 | 3) => ClockSource::Dot,
            (1, 1 | 3) => ClockSource::HBlank,
            (2, 2 | 3) => ClockSource::SystemDiv8,
            _ => panic!("Invalid timer idx: {}", self.idx),
        };

        self.last_update_clock = match self.clock_source {
            ClockSource::Dot => gpu_timer.dot_clock,
            ClockSource::HBlank => gpu_timer.hblank_clock,
            ClockSource::System => scheduler.cpu_cycle_counter(),
            ClockSource::SystemDiv8 => scheduler.cpu_cycle_counter() / 8,
        };

        self.counter = 0;
        self.wait_cycle = true;
        self.irq = false;
        self.irq_since_mode_write = false;

        log::debug!("Timer {} mode write: {value:04X}", self.idx);
        log::debug!("  Reset mode: {:?}", self.reset_mode);
        log::debug!("  IRQ at counter=target: {}", self.target_irq_enabled);
        log::debug!("  IRQ at counter=$FFFF: {}", self.overflow_irq_enabled);
        log::debug!("  IRQ mode: {:?}", self.irq_mode);
        log::debug!("  Clock source: {:?}", self.clock_source);
    }

    fn write_target_value(&mut self, value: u32) {
        self.target_value = value as u16;

        log::debug!("Timer {} target value write: {value:04X}", self.idx);
    }

    fn maybe_schedule_irq(&self, scheduler: &mut Scheduler) {
        let Some(clocks_until_irq) = self.clocks_until_irq() else {
            scheduler.remove_event(self.interrupt_type);
            return;
        };

        let cpu_clocks_until_irq = match self.clock_source {
            ClockSource::System => clocks_until_irq,
            ClockSource::SystemDiv8 => 8 * clocks_until_irq,
            _ => todo!("clock source {:?}", self.clock_source),
        };

        scheduler.update_or_push_event(SchedulerEvent::timer_2_irq(
            scheduler.cpu_cycle_counter() + cpu_clocks_until_irq,
        ));
    }
}

#[derive(Debug, Clone)]
pub struct Timers {
    gpu: GpuTimer,
    timers: [SystemTimer; 3],
}

impl Timers {
    pub fn new() -> Self {
        Self {
            gpu: GpuTimer::new(),
            timers: [
                SystemTimer::new(0, SchedulerEventType::Timer0Irq),
                SystemTimer::new(1, SchedulerEventType::Timer1Irq),
                SystemTimer::new(2, SchedulerEventType::Timer2Irq),
            ],
        }
    }

    pub fn schedule_next_vblank(&mut self, scheduler: &mut Scheduler) {
        self.gpu.schedule_next_vblank(scheduler);
    }

    pub fn schedule_next_timer_0_irq(&mut self, scheduler: &mut Scheduler) {
        self.schedule_next_timer_irq(0, scheduler);
    }

    pub fn scheduler_next_timer_1_irq(&mut self, scheduler: &mut Scheduler) {
        self.schedule_next_timer_irq(1, scheduler);
    }

    pub fn schedule_next_timer_2_irq(&mut self, scheduler: &mut Scheduler) {
        self.schedule_next_timer_irq(2, scheduler);
    }

    fn schedule_next_timer_irq(&mut self, timer_idx: usize, scheduler: &mut Scheduler) {
        self.timers[timer_idx].catch_up(scheduler, &self.gpu);
        self.timers[timer_idx].maybe_schedule_irq(scheduler);
    }

    pub fn in_vblank(&mut self, scheduler: &mut Scheduler) -> bool {
        self.gpu.catch_up(scheduler.cpu_cycle_counter());
        self.gpu.in_vblank()
    }

    pub fn odd_frame(&mut self, scheduler: &mut Scheduler) -> bool {
        self.gpu.catch_up(scheduler.cpu_cycle_counter());
        self.gpu.odd_frame
    }

    pub fn scanline(&mut self, scheduler: &mut Scheduler) -> u16 {
        self.gpu.catch_up(scheduler.cpu_cycle_counter());
        self.gpu.scanline()
    }

    pub fn update_display_mode(
        &mut self,
        dot_clock_divider: u16,
        interlaced: bool,
        scheduler: &mut Scheduler,
    ) {
        self.gpu
            .update_display_mode(dot_clock_divider.into(), interlaced, scheduler);

        if self.timers[0].clock_source == ClockSource::Dot {
            self.timers[0].maybe_schedule_irq(scheduler);
        }

        if self.timers[1].clock_source == ClockSource::HBlank {
            self.timers[1].maybe_schedule_irq(scheduler);
        }
    }

    pub fn update_v_display_area(&mut self, y1: u16, y2: u16, scheduler: &mut Scheduler) {
        self.gpu.update_v_display_range(y1, y2, scheduler);
    }

    pub fn read_register(&mut self, address: u32, scheduler: &Scheduler) -> u32 {
        let timer_idx = ((address >> 4) & 3) as usize;
        if timer_idx == 3 {
            return 0;
        }

        match address & 0xF {
            0x0 => {
                self.timers[timer_idx].catch_up(scheduler, &self.gpu);
                self.timers[timer_idx].counter.into()
            }
            _ => todo!("timer register read {address:08X}"),
        }
    }

    pub fn write_register(&mut self, address: u32, value: u32, scheduler: &mut Scheduler) {
        let timer_idx = ((address >> 4) & 3) as usize;
        if timer_idx == 3 {
            return;
        }

        self.timers[timer_idx].catch_up(scheduler, &self.gpu);

        match address & 0xF {
            0x0 => {
                self.timers[timer_idx].write_counter(value);
            }
            0x4 => {
                self.timers[timer_idx].write_mode(value, scheduler, &self.gpu);
            }
            0x8 => {
                self.timers[timer_idx].write_target_value(value);
            }
            _ => todo!("timer register write {address:08X} {value:08X}"),
        }

        self.timers[timer_idx].maybe_schedule_irq(scheduler);
    }
}
