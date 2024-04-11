//! PS1 DMA registers and transfers
//!
//! DMA channels:
//! - DMA0: MDEC In (RAM-to-MDEC)
//! - DMA1: MDEC Out (MDEC-to-RAM)
//! - DMA2: GPU
//! - DMA3: CD-ROM
//! - DMA4: SPU
//! - DMA5: PIO (Parallel I/O port, apparently not used by any games?)
//! - DMA6: OTC (Ordering Table Clear, used to prepare the graphics ordering table in RAM)

use crate::cd::CdController;
use crate::gpu::Gpu;
use crate::interrupts::{InterruptRegisters, InterruptType};
use crate::mdec::MacroblockDecoder;
use crate::memory::Memory;
use crate::num::{U32Ext, U8Ext};
use crate::scheduler::{Scheduler, SchedulerEvent};
use crate::spu::Spu;
use bincode::{Decode, Encode};
use std::{array, cmp, mem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum DmaDirection {
    #[default]
    ToRam = 0,
    FromRam = 1,
}

impl DmaDirection {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::FromRam } else { Self::ToRam }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum Step {
    #[default]
    Forwards = 0,
    Backwards = 1,
}

impl Step {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::Backwards } else { Self::Forwards }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
enum TransferMode {
    // Transfer all at once; used for CD-ROM and OTC
    #[default]
    Burst = 0,
    // Transfer in blocks according to DMA request signal; used for GPU (data), SPU, and MDEC
    Block = 1,
    // Linked list mode; used for GPU (command lists)
    LinkedList = 2,
}

impl TransferMode {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Burst,
            1 => Self::Block,
            2 => Self::LinkedList,
            3 => {
                log::error!("Unexpected DMA sync mode of 3");
                Self::Burst
            }
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Encode, Decode)]
struct ChannelConfig {
    start_address: u32,
    // total length for SyncMode=0, block size for SyncMode=1
    block_size: u32,
    // only applicable for SyncMode=1
    num_blocks: u32,
    direction: DmaDirection,
    step: Step,
    transfer_mode: TransferMode,
    chopping_enabled: bool,
    // Both chopping sizes are in 2^N units, e.g. N=3 -> 8
    chopping_dma_window_size: u32,
    chopping_cpu_window_size: u32,
    transfer_active: bool,
}

impl ChannelConfig {
    fn otc() -> Self {
        Self { step: Step::Backwards, ..Self::default() }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
struct DmaControlRegister {
    channel_priority: [u8; 7],
    channel_enabled: [bool; 7],
    channels_in_priority_order: [usize; 7],
}

impl DmaControlRegister {
    fn new() -> Self {
        // Control register value at power-on should be $07654321:
        //   Channel 0 has priority 1, channel 1 has priority 2, etc.
        //   All channels are disabled
        Self {
            channel_priority: array::from_fn(|i| (i + 1) as u8),
            channel_enabled: array::from_fn(|_| false),
            channels_in_priority_order: array::from_fn(|i| i),
        }
    }

    fn read(&self) -> u32 {
        let priority_bits = self
            .channel_priority
            .into_iter()
            .enumerate()
            .map(|(channel, priority)| u32::from(priority) << (4 * channel))
            .reduce(|a, b| a | b)
            .unwrap();

        let enabled_bits = self
            .channel_enabled
            .into_iter()
            .enumerate()
            .map(|(channel, enabled)| u32::from(enabled) << (3 + 4 * channel))
            .reduce(|a, b| a | b)
            .unwrap();

        priority_bits | enabled_bits
    }

    fn write(&mut self, value: u32) {
        self.channel_priority = array::from_fn(|i| ((value >> (4 * i)) & 7) as u8);

        self.channel_enabled = array::from_fn(|i| value.bit(3 + 4 * i as u8));

        self.channels_in_priority_order.sort_by(|&a, &b| {
            self.channel_enabled[a]
                .cmp(&self.channel_enabled[b])
                .reverse()
                .then(self.channel_priority[a].cmp(&self.channel_priority[b]))
                .then(a.cmp(&b))
        });
    }
}

#[derive(Debug, Clone, Default, Encode, Decode)]
struct DmaInterruptRegister {
    channel_irq_enabled: u8,
    irq_enabled: bool,
    channel_irq_pending: u8,
    force_irq: bool,
}

impl DmaInterruptRegister {
    fn pending(&self) -> bool {
        self.force_irq
            || (self.irq_enabled && (self.channel_irq_enabled & self.channel_irq_pending != 0))
    }

    fn read(&self) -> u32 {
        let irq_pending = self.pending();

        (u32::from(self.force_irq) << 15)
            | (u32::from(self.channel_irq_enabled) << 16)
            | (u32::from(self.irq_enabled) << 23)
            | (u32::from(self.channel_irq_pending) << 24)
            | (u32::from(irq_pending) << 31)
    }

    fn write(&mut self, value: u32) {
        self.force_irq = value.bit(15);

        let channel_irq_enabled = ((value >> 16) as u8) & 0x7F;
        self.channel_irq_enabled = channel_irq_enabled;
        self.irq_enabled = value.bit(23);

        let irq_pending_mask = ((value >> 24) as u8) & 0x7F;
        self.channel_irq_pending &= !irq_pending_mask & channel_irq_enabled;
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DmaController {
    control: DmaControlRegister,
    interrupt: DmaInterruptRegister,
    channel_configs: [ChannelConfig; 7],
    cpu_wait_cycles: u32,
}

impl DmaController {
    pub fn new() -> Self {
        let mut channel_configs = array::from_fn(|_| ChannelConfig::default());
        channel_configs[6] = ChannelConfig::otc();

        Self {
            control: DmaControlRegister::new(),
            interrupt: DmaInterruptRegister::default(),
            channel_configs,
            cpu_wait_cycles: 0,
        }
    }

    pub fn read_control(&self) -> u32 {
        self.control.read()
    }

    pub fn read_interrupt(&self) -> u32 {
        self.interrupt.read()
    }

    pub fn write_interrupt(&mut self, value: u32, interrupt_registers: &mut InterruptRegisters) {
        let prev_irq_pending = self.interrupt.pending();
        self.interrupt.write(value);

        if !prev_irq_pending && self.interrupt.pending() {
            interrupt_registers.set_interrupt_flag(InterruptType::Dma);
        }

        log::debug!("DMA interrupt register write: {value:08X} {:X?}", self.interrupt);
    }

    pub fn write_control(&mut self, value: u32) {
        self.control.write(value);

        log::trace!("DMA control register write: {value:08X}");
        log::trace!("  DMA enabled: {:?}", self.control.channel_enabled);
        log::trace!("  DMA priority: {:?}", self.control.channel_priority);
    }

    pub fn write_channel_address(&mut self, address: u32, value: u32) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        self.channel_configs[channel as usize].start_address = value & 0x1FFFFF;

        log::trace!(
            "DMA{channel} address: {:06X}",
            self.channel_configs[channel as usize].start_address
        );
    }

    pub fn write_channel_length(&mut self, address: u32, value: u32) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &mut self.channel_configs[channel as usize];
        channel_config.block_size = value & 0xFFFF;
        channel_config.num_blocks = value >> 16;

        if channel_config.block_size == 0 {
            channel_config.block_size = 0x10000;
        }

        log::trace!(
            "DMA{channel} length: block_size={:04X}, block_amount={:04X}",
            channel_config.block_size,
            channel_config.num_blocks
        );
    }

    pub fn read_channel_control(&self, address: u32) -> u32 {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &self.channel_configs[channel as usize];

        // TODO bit 24 hardcoded to 0 (stopped/completed)
        (channel_config.direction as u32)
            | ((channel_config.step as u32) << 1)
            | (u32::from(channel_config.chopping_enabled) << 8)
            | ((channel_config.transfer_mode as u32) << 9)
            | (channel_config.chopping_dma_window_size << 16)
            | (channel_config.chopping_cpu_window_size << 20)
    }

    pub fn write_channel_control(&mut self, address: u32, value: u32, scheduler: &mut Scheduler) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &mut self.channel_configs[channel as usize];

        if channel != 6 {
            // Only channels 0-5 are allowed to change most channel settings
            // Channel 6 (OTC) can only start DMA through the control register, nothing else

            channel_config.direction = DmaDirection::from_bit(value.bit(0));
            channel_config.step = Step::from_bit(value.bit(1));
            channel_config.chopping_enabled = value.bit(8);
            channel_config.transfer_mode = TransferMode::from_bits(value >> 9);
            channel_config.chopping_dma_window_size = (value >> 16) & 7;
            channel_config.chopping_cpu_window_size = (value >> 20) & 7;

            log::trace!("DMA{channel} channel control write: {value:08X}");
            log::trace!("  Direction: {:?}", channel_config.direction);
            log::trace!("  Step: {:?}", channel_config.step);
            log::trace!("  Sync mode: {:?}", channel_config.transfer_mode);
            log::trace!("  Chopping enabled: {}", channel_config.chopping_enabled);
            log::trace!(
                "  Chopping DMA window size: {}",
                1 << channel_config.chopping_dma_window_size
            );
            log::trace!(
                "  Chopping CPU window size: {}",
                1 << channel_config.chopping_cpu_window_size
            );
        }

        let start_transfer = value.bit(24);
        channel_config.transfer_active = start_transfer;

        if start_transfer {
            scheduler
                .update_or_push_event(SchedulerEvent::process_dma(scheduler.cpu_cycle_counter()));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        memory: &mut Memory,
        gpu: &mut Gpu,
        spu: &mut Spu,
        mdec: &mut MacroblockDecoder,
        cd_controller: &mut CdController,
        scheduler: &mut Scheduler,
        interrupt_registers: &mut InterruptRegisters,
    ) {
        for channel in self.control.channels_in_priority_order {
            if !self.control.channel_enabled[channel] {
                break;
            }

            if !self.channel_configs[channel].transfer_active {
                continue;
            }

            match channel {
                0 => {
                    // DMA0: MDEC In
                    // Always uses block DMA
                    // Takes roughly 17 cycles per 16 words
                    // TODO per-block timing
                    let words =
                        self.channel_configs[0].num_blocks * self.channel_configs[0].block_size;
                    self.cpu_wait_cycles = words * 17 / 16;

                    log::debug!(
                        "Running MDEC In DMA; {} blocks of size {}",
                        self.channel_configs[0].num_blocks,
                        self.channel_configs[0].block_size
                    );
                    run_mdec_in_dma(&mut self.channel_configs[0], mdec, memory);

                    self.channel_configs[0].transfer_active = false;
                    self.maybe_flag_dma_interrupt(0, interrupt_registers);

                    break;
                }
                1 => {
                    // DMA1: MDEC Out
                    // Always uses block DMA
                    // Takes roughly 17 cycles per 16 words, not including decompression timing
                    // TODO per-block timing and decompression timing
                    let words =
                        self.channel_configs[1].num_blocks * self.channel_configs[1].block_size;
                    self.cpu_wait_cycles = words * 17 / 16;

                    log::debug!(
                        "Running MDEC Out DMA; {} blocks of size {}",
                        self.channel_configs[1].num_blocks,
                        self.channel_configs[1].block_size
                    );
                    run_mdec_out_dma(&mut self.channel_configs[1], mdec, memory);

                    self.channel_configs[1].transfer_active = false;
                    self.maybe_flag_dma_interrupt(1, interrupt_registers);

                    break;
                }
                2 => {
                    // DMA2: GPU DMA
                    // Can use block DMA or linked list DMA
                    // Takes roughly 17 cycles per 16 words, not including GPU draw timing
                    // TODO per-block/node timing and GPU draw timing
                    log::debug!(
                        "Running GPU DMA in mode {:?}",
                        self.channel_configs[2].transfer_mode
                    );
                    let words = run_gpu_dma(&mut self.channel_configs[2], gpu, memory);
                    self.cpu_wait_cycles = words * 17 / 16;

                    self.channel_configs[2].transfer_active = false;
                    self.maybe_flag_dma_interrupt(2, interrupt_registers);

                    break;
                }
                3 => {
                    // DMA3: CD-ROM DMA
                    // Always uses burst DMA, very rarely with chopping enabled
                    // Takes either 24 cycles per word or 40 cycles per word depending on memory settings
                    // TODO chopping
                    // TODO look at memory control instead of assuming 24 cycles/word
                    self.cpu_wait_cycles = 24 * self.channel_configs[3].block_size;

                    log::debug!(
                        "Running CD-ROM DMA of size {}",
                        self.channel_configs[3].block_size
                    );
                    run_cdrom_dma(&self.channel_configs[3], memory, cd_controller);

                    self.channel_configs[3].transfer_active = false;
                    self.maybe_flag_dma_interrupt(3, interrupt_registers);

                    break;
                }
                4 => {
                    // DMA4: SPU DMA
                    // Always uses block DMA
                    // Takes roughly 4 cycles per word
                    // TODO per-block timing
                    self.cpu_wait_cycles =
                        4 * self.channel_configs[4].num_blocks * self.channel_configs[4].block_size;

                    log::debug!(
                        "Running SPU DMA; {} blocks of size {}",
                        self.channel_configs[4].num_blocks,
                        self.channel_configs[4].block_size
                    );
                    run_spu_dma(&mut self.channel_configs[4], memory, spu);

                    self.channel_configs[4].transfer_active = false;
                    self.maybe_flag_dma_interrupt(4, interrupt_registers);

                    break;
                }
                6 => {
                    // DMA6: OTC DMA
                    // Always uses burst DMA
                    // Takes roughly 17 cycles per 16 words
                    self.cpu_wait_cycles = self.channel_configs[6].block_size * 17 / 16;

                    log::debug!("Running OTC DMA of size {}", self.channel_configs[6].block_size);
                    run_otc_dma(&self.channel_configs[6], memory);

                    self.channel_configs[6].transfer_active = false;
                    self.maybe_flag_dma_interrupt(6, interrupt_registers);

                    break;
                }
                _ => panic!("Invalid DMA channel {channel}"),
            }
        }

        if self.channel_configs.iter().any(|config| config.transfer_active) {
            let wait_cycles: u64 = cmp::max(1, self.cpu_wait_cycles).into();
            scheduler.update_or_push_event(SchedulerEvent::process_dma(
                scheduler.cpu_cycle_counter() + wait_cycles,
            ));
        }
    }

    fn maybe_flag_dma_interrupt(
        &mut self,
        channel: usize,
        interrupt_registers: &mut InterruptRegisters,
    ) {
        if !self.interrupt.channel_irq_enabled.bit(channel as u8) {
            // DMA interrupt pending flags are only set if interrupts are enabled for that channel
            return;
        }

        let prev_pending = self.interrupt.pending();
        self.interrupt.channel_irq_pending |= 1 << channel;

        if !prev_pending && self.interrupt.pending() {
            // IRQ2 is set when DMA interrupt pending goes from 0 to 1
            interrupt_registers.set_interrupt_flag(InterruptType::Dma);
        }
    }

    pub fn cpu_wait_cycles(&self) -> u32 {
        self.cpu_wait_cycles
    }

    pub fn take_cpu_wait_cycles(&mut self) -> u32 {
        mem::take(&mut self.cpu_wait_cycles)
    }
}

fn run_mdec_in_dma(config: &mut ChannelConfig, mdec: &mut MacroblockDecoder, memory: &Memory) {
    let mut address = config.start_address & !3;
    for _ in 0..config.block_size * config.num_blocks {
        let word = memory.read_main_ram_u32(address);
        mdec.write_command(word);

        address = match config.step {
            Step::Forwards => address.wrapping_add(4) & 0x1FFFFF,
            Step::Backwards => address.wrapping_sub(4) & 0x1FFFFF,
        };
    }

    config.start_address = address;
    config.num_blocks = 0;
}

// TODO reorder 8x8 blocks if MDEC is in 15bpp or 24bpp mode instead of assuming the MDEC code will do it
fn run_mdec_out_dma(config: &mut ChannelConfig, mdec: &mut MacroblockDecoder, memory: &mut Memory) {
    let mut address = config.start_address & !3;
    for _ in 0..config.block_size * config.num_blocks {
        let word = mdec.read_data();
        memory.write_main_ram_u32(address, word);

        address = match config.step {
            Step::Forwards => address.wrapping_add(4) & 0x1FFFFF,
            Step::Backwards => address.wrapping_sub(4) & 0x1FFFFF,
        };
    }

    config.start_address = address;
    config.num_blocks = 0;
}

fn run_gpu_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &mut Memory) -> u32 {
    match config.transfer_mode {
        TransferMode::Block => run_gpu_block_dma(config, gpu, memory),
        TransferMode::LinkedList => run_gpu_linked_list_dma(config, gpu, memory),
        TransferMode::Burst => todo!("sync mode 0 unexpected for GPU DMA"),
    }
}

fn run_gpu_block_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &mut Memory) -> u32 {
    let total_words = config.num_blocks * config.block_size;

    match config.direction {
        DmaDirection::FromRam => {
            let mut address = config.start_address & !3;
            for _ in 0..config.block_size * config.num_blocks {
                let word = memory.read_main_ram_u32(address);
                gpu.write_gp0_command(word);

                address = match config.step {
                    Step::Forwards => address.wrapping_add(4) & 0x1FFFFF,
                    Step::Backwards => address.wrapping_sub(4) & 0x1FFFFF,
                };
            }

            config.start_address = address;
            config.num_blocks = 0;
        }
        DmaDirection::ToRam => {
            let mut address = config.start_address & !3;
            for _ in 0..config.block_size * config.num_blocks {
                let word = gpu.read_port();
                memory.write_main_ram_u32(address, word);

                address = match config.step {
                    Step::Forwards => address.wrapping_add(4) & 0x1FFFFF,
                    Step::Backwards => address.wrapping_sub(4) & 0x1FFFFF,
                };
            }

            config.start_address = address;
            config.num_blocks = 0;
        }
    }

    total_words
}

fn run_gpu_linked_list_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &Memory) -> u32 {
    let mut total_word_count = 0;

    match config.direction {
        DmaDirection::FromRam => {
            let mut address = config.start_address & !3;
            loop {
                // TODO timing, don't do all at once
                let node = memory.read_main_ram_u32(address);

                let word_count = node >> 24;
                for i in 0..word_count {
                    let word_addr = address.wrapping_add(4 * (i + 1));
                    let word = memory.read_main_ram_u32(word_addr);
                    gpu.write_gp0_command(word);
                }

                total_word_count += word_count;

                if node.bit(23) {
                    // End marker encountered
                    config.start_address = node & 0xFFFFFF;
                    break;
                }

                address = node & 0x1FFFFF;
            }
        }
        DmaDirection::ToRam => todo!("GPU linked list DMA from VRAM to CPU RAM"),
    }

    total_word_count
}

// CD-ROM DMA
// Copies data from the CD controller's data FIFO to main RAM
fn run_cdrom_dma(config: &ChannelConfig, memory: &mut Memory, cd_controller: &mut CdController) {
    let mut address = config.start_address & !3;
    for _ in 0..config.block_size {
        let mut bytes = [0; 4];
        for byte in &mut bytes {
            *byte = cd_controller.read_data_fifo();
        }

        memory.write_main_ram_u32(address, u32::from_le_bytes(bytes));
        address = address.wrapping_add(4);
    }
}

// SPU DMA
// Copies data between main RAM and SPU audio RAM
fn run_spu_dma(config: &mut ChannelConfig, memory: &mut Memory, spu: &mut Spu) {
    match config.direction {
        DmaDirection::FromRam => {
            let mut address = config.start_address & !3;
            for _ in 0..config.block_size * config.num_blocks {
                let word = memory.read_main_ram_u32(address);
                spu.write_data_port(word as u16);
                spu.write_data_port((word >> 16) as u16);

                address = match config.step {
                    Step::Forwards => address.wrapping_add(4),
                    Step::Backwards => address.wrapping_sub(4),
                };
            }

            config.start_address = address;
            config.num_blocks = 0;
        }
        DmaDirection::ToRam => {
            let mut address = config.start_address & !3;
            for _ in 0..config.block_size * config.num_blocks {
                let low_halfword = spu.read_data_port();
                let high_halfword = spu.read_data_port();
                let word = u32::from(low_halfword) | (u32::from(high_halfword) << 16);
                memory.write_main_ram_u32(address, word);

                address = match config.step {
                    Step::Forwards => address.wrapping_add(4),
                    Step::Backwards => address.wrapping_sub(4),
                };
            }

            config.start_address = address;
            config.num_blocks = 0;
        }
    }
}

// OTC (ordering table clear) DMA
// Prepares a section of main RAM for a GPU linked list DMA by creating a linked list where every
// entry points to the entry at the previous word address
fn run_otc_dma(config: &ChannelConfig, memory: &mut Memory) {
    let mut address = config.start_address & !3;
    for i in 0..config.block_size {
        let next_addr = if i == config.block_size - 1 {
            // TODO is this right?
            0xFFFFFF
        } else {
            address.wrapping_sub(4) & 0x1FFFFF
        };

        memory.write_main_ram_u32(address, next_addr);

        address = next_addr;
    }
}
