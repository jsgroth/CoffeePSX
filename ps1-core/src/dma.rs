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
use crate::memory;
use crate::memory::Memory;
use crate::num::{U32Ext, U8Ext};
use crate::pgxp::PgxpConfig;
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

    fn apply(self, address: u32) -> u32 {
        match self {
            Self::Forwards => address.wrapping_add(4) & memory::MAIN_RAM_MASK,
            Self::Backwards => address.wrapping_sub(4) & memory::MAIN_RAM_MASK,
        }
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
    next_active_cycles: u64,
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

pub struct DmaContext<'a> {
    pub memory: &'a mut Memory,
    pub gpu: &'a mut Gpu,
    pub spu: &'a mut Spu,
    pub mdec: &'a mut MacroblockDecoder,
    pub cd_controller: &'a mut CdController,
    pub scheduler: &'a mut Scheduler,
    pub interrupt_registers: &'a mut InterruptRegisters,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DmaController {
    control: DmaControlRegister,
    interrupt: DmaInterruptRegister,
    channel_configs: [ChannelConfig; 7],
    pgxp_config: PgxpConfig,
    cpu_wait_cycles: u32,
    global_next_active_cycles: u64,
}

const MDEC_IN: usize = 0;
const MDEC_OUT: usize = 1;
const GPU: usize = 2;
const CD_ROM: usize = 3;
const SPU: usize = 4;
const OTC: usize = 6;

impl DmaController {
    pub fn new(pgxp_config: PgxpConfig) -> Self {
        let mut channel_configs = array::from_fn(|_| ChannelConfig::default());
        channel_configs[OTC] = ChannelConfig::otc();

        Self {
            control: DmaControlRegister::new(),
            interrupt: DmaInterruptRegister::default(),
            channel_configs,
            pgxp_config,
            cpu_wait_cycles: 0,
            global_next_active_cycles: 0,
        }
    }

    pub fn update_pgxp_config(&mut self, pgxp_config: PgxpConfig) {
        self.pgxp_config = pgxp_config;
    }

    // $1F8010F0: DPCR (DMA control register)
    pub fn read_control(&self) -> u32 {
        self.control.read()
    }

    // $1F8010F0: DPCR (DMA control register)
    pub fn write_control(&mut self, value: u32, scheduler: &mut Scheduler) {
        self.control.write(value);

        // Attempt to schedule in case any active channels were enabled
        self.maybe_schedule_process_dma(scheduler);

        log::debug!("DMA control register write: {value:08X}");
        log::debug!("  DMA enabled: {:?}", self.control.channel_enabled);
        log::debug!("  DMA priority: {:?}", self.control.channel_priority);
    }

    // $1F8010F4: DICR (DMA interrupt register)
    pub fn read_interrupt(&self) -> u32 {
        self.interrupt.read()
    }

    // $1F8010F4: DICR (DMA interrupt register)
    pub fn write_interrupt(&mut self, value: u32, interrupt_registers: &mut InterruptRegisters) {
        let prev_irq_pending = self.interrupt.pending();
        self.interrupt.write(value);

        if !prev_irq_pending && self.interrupt.pending() {
            interrupt_registers.set_interrupt_flag(InterruptType::Dma);
        }

        log::debug!("DMA interrupt register write: {value:08X} {:X?}", self.interrupt);
    }

    // $1F801080 + N*$10: Dn_MADR (DMA base address)
    pub fn read_channel_address(&self, address: u32) -> u32 {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        self.channel_configs[channel as usize].start_address
    }

    // $1F801080 + N*$10: Dn_MADR (DMA base address)
    pub fn write_channel_address(&mut self, address: u32, value: u32) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        self.channel_configs[channel as usize].start_address = value & 0x1FFFFF;

        log::trace!(
            "DMA{channel} address: {:06X}",
            self.channel_configs[channel as usize].start_address
        );
    }

    // $1F801084 + N*$10: Dn_BCR (DMA block control)
    pub fn read_channel_length(&self, address: u32) -> u32 {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &self.channel_configs[channel as usize];
        (channel_config.block_size & 0xFFFF) | (channel_config.num_blocks << 16)
    }

    // $1F801084 + N*$10: Dn_BCR (DMA block control)
    pub fn write_channel_length(&mut self, address: u32, value: u32) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &mut self.channel_configs[channel as usize];
        channel_config.block_size = value & 0xFFFF;
        channel_config.num_blocks = value >> 16;

        if channel_config.block_size == 0 {
            channel_config.block_size = 0x10000;
        }

        if channel_config.num_blocks == 0 {
            channel_config.num_blocks = 0x10000;
        }

        log::trace!(
            "DMA{channel} length: block_size={:04X}, block_amount={:04X}",
            channel_config.block_size,
            channel_config.num_blocks
        );
    }

    // $1F801088 + N*$10: Dn_CHCR (DMA channel control)
    pub fn read_channel_control(&self, address: u32) -> u32 {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &self.channel_configs[channel as usize];

        (channel_config.direction as u32)
            | ((channel_config.step as u32) << 1)
            | (u32::from(channel_config.chopping_enabled) << 8)
            | ((channel_config.transfer_mode as u32) << 9)
            | (channel_config.chopping_dma_window_size << 16)
            | (channel_config.chopping_cpu_window_size << 20)
            | (u32::from(channel_config.transfer_active) << 24)
    }

    // $1F801088 + N*$10: Dn_CHCR (DMA channel control)
    pub fn write_channel_control(&mut self, address: u32, value: u32, scheduler: &mut Scheduler) {
        let channel = ((address >> 4) & 7) as usize;
        assert!(channel < 7, "DMA channel should always be 0-6");

        let channel_config = &mut self.channel_configs[channel];

        if channel != OTC {
            // Only channels 0-5 are allowed to change most channel settings
            // Channel 6 (OTC) can only start DMA through the control register, nothing else

            channel_config.direction = DmaDirection::from_bit(value.bit(0));
            channel_config.step = Step::from_bit(value.bit(1));
            channel_config.chopping_enabled = value.bit(8);
            channel_config.transfer_mode = TransferMode::from_bits(value >> 9);
            channel_config.chopping_dma_window_size = (value >> 16) & 7;
            channel_config.chopping_cpu_window_size = (value >> 20) & 7;

            log::debug!("DMA{channel} channel control write: {value:08X}");
            log::debug!("  Direction: {:?}", channel_config.direction);
            log::debug!("  Step: {:?}", channel_config.step);
            log::debug!("  Sync mode: {:?}", channel_config.transfer_mode);
            log::debug!("  Chopping enabled: {}", channel_config.chopping_enabled);
            log::debug!(
                "  Chopping DMA window size: {}",
                1 << channel_config.chopping_dma_window_size
            );
            log::debug!(
                "  Chopping CPU window size: {}",
                1 << channel_config.chopping_cpu_window_size
            );
            log::debug!("  Transfer active: {}", value.bit(24));
        }

        let start_transfer = value.bit(24);
        channel_config.transfer_active = start_transfer;

        if start_transfer {
            scheduler.min_or_push_event(SchedulerEvent::process_dma(scheduler.cpu_cycle_counter()));
        }
    }

    pub fn process(
        &mut self,
        DmaContext { memory, gpu, spu, mdec, cd_controller, scheduler, interrupt_registers }: DmaContext<'_>,
    ) {
        if scheduler.cpu_cycle_counter() < self.global_next_active_cycles {
            scheduler
                .update_or_push_event(SchedulerEvent::process_dma(self.global_next_active_cycles));
            return;
        }

        self.cpu_wait_cycles = 0;

        for channel in self.control.channels_in_priority_order {
            if !self.control.channel_enabled[channel] {
                break;
            }

            if !self.channel_configs[channel].transfer_active
                || self.channel_configs[channel].next_active_cycles > scheduler.cpu_cycle_counter()
            {
                continue;
            }

            match channel {
                MDEC_IN => {
                    // DMA0: MDEC In
                    // Always uses block DMA
                    // Takes roughly 17 cycles per 16 words
                    // TODO per-block timing
                    let config = &mut self.channel_configs[MDEC_IN];

                    log::debug!(
                        "Running MDEC In DMA; {} blocks of size {}",
                        config.num_blocks,
                        config.block_size
                    );

                    self.cpu_wait_cycles =
                        progress_mdec_in_dma(config, mdec, memory, scheduler.cpu_cycle_counter());

                    if !config.transfer_active {
                        log::debug!("MDEC In DMA finished");
                        self.maybe_flag_dma_interrupt(MDEC_IN, interrupt_registers);
                    }

                    break;
                }
                MDEC_OUT => {
                    // DMA1: MDEC Out
                    // Always uses block DMA
                    // Takes roughly 17 cycles per 16 words, not including decompression timing
                    // TODO per-block timing and decompression timing
                    let config = &mut self.channel_configs[MDEC_OUT];

                    if !mdec.data_out_request() && config.num_blocks != 0 {
                        config.next_active_cycles = scheduler.cpu_cycle_counter() + 16;
                        continue;
                    }

                    log::debug!(
                        "Running MDEC Out DMA; {} blocks of size {}, addr {:X}",
                        config.num_blocks,
                        config.block_size,
                        config.start_address
                    );

                    let cpu_wait_cycles =
                        progress_mdec_out_dma(config, mdec, memory, scheduler.cpu_cycle_counter());
                    self.cpu_wait_cycles = cpu_wait_cycles;

                    if !config.transfer_active {
                        log::debug!("MDEC Out DMA finished");
                        self.maybe_flag_dma_interrupt(MDEC_OUT, interrupt_registers);
                    }

                    break;
                }
                GPU => {
                    // DMA2: GPU DMA
                    // Can use block DMA or linked list DMA
                    // Takes roughly 17 cycles per 16 words, not including GPU draw timing
                    // TODO per-block/node timing and GPU draw timing
                    let config = &mut self.channel_configs[GPU];

                    log::debug!(
                        "Running GPU DMA in mode {:?}, {} blocks of size {}, addr {:X}",
                        config.transfer_mode,
                        config.num_blocks,
                        config.block_size,
                        config.start_address
                    );
                    let cpu_wait_cycles = progress_gpu_dma(
                        config,
                        gpu,
                        memory,
                        self.pgxp_config.enabled,
                        scheduler.cpu_cycle_counter(),
                    );
                    self.cpu_wait_cycles = cpu_wait_cycles;

                    if !config.transfer_active {
                        log::debug!("GPU DMA finished");
                        self.maybe_flag_dma_interrupt(GPU, interrupt_registers);
                    }

                    break;
                }
                CD_ROM => {
                    // DMA3: CD-ROM DMA
                    // Always uses burst DMA, very rarely with chopping enabled
                    // Takes either 24 cycles per word or 40 cycles per word depending on memory settings
                    // TODO chopping
                    // TODO look at memory control instead of assuming 24 cycles/word
                    let config = &mut self.channel_configs[CD_ROM];

                    self.cpu_wait_cycles = 24 * config.block_size;

                    log::debug!("Running CD-ROM DMA of size {}", config.block_size);
                    run_cdrom_dma(config, memory, cd_controller);

                    config.transfer_active = false;
                    self.maybe_flag_dma_interrupt(CD_ROM, interrupt_registers);

                    break;
                }
                SPU => {
                    // DMA4: SPU DMA
                    // Always uses block DMA
                    // Takes roughly 4 cycles per word
                    // TODO per-block timing
                    let config = &mut self.channel_configs[SPU];

                    log::debug!(
                        "Running SPU DMA; {} blocks of size {}",
                        config.num_blocks,
                        config.block_size
                    );
                    self.cpu_wait_cycles =
                        progress_spu_dma(config, memory, spu, scheduler.cpu_cycle_counter());

                    if !config.transfer_active {
                        log::debug!("SPU DMA complete");
                        self.maybe_flag_dma_interrupt(SPU, interrupt_registers);
                    }

                    break;
                }
                OTC => {
                    // DMA6: OTC DMA
                    // Always uses burst DMA
                    // Takes roughly 17 cycles per 16 words
                    let config = &mut self.channel_configs[OTC];

                    self.cpu_wait_cycles = config.block_size * 17 / 16;

                    log::debug!("Running OTC DMA of size {}", config.block_size);
                    run_otc_dma(config, memory);

                    config.transfer_active = false;
                    self.maybe_flag_dma_interrupt(OTC, interrupt_registers);

                    break;
                }
                _ => panic!("Invalid DMA channel {channel}"),
            }
        }

        self.global_next_active_cycles =
            scheduler.cpu_cycle_counter() + u64::from(self.cpu_wait_cycles);

        self.maybe_schedule_process_dma(scheduler);
    }

    fn maybe_schedule_process_dma(&self, scheduler: &mut Scheduler) {
        if let Some(next_active_cycles) = self
            .channel_configs
            .iter()
            .enumerate()
            .filter_map(|(channel, config)| {
                (config.transfer_active && self.control.channel_enabled[channel])
                    .then_some(config.next_active_cycles)
            })
            .min()
        {
            let schedule_cycles = cmp::max(self.global_next_active_cycles, next_active_cycles);
            scheduler.min_or_push_event(SchedulerEvent::process_dma(schedule_cycles));
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

        log::debug!("Setting DMA interrupt flag for DMA{channel}");

        let prev_pending = self.interrupt.pending();
        self.interrupt.channel_irq_pending |= 1 << channel;

        if !prev_pending && self.interrupt.pending() {
            // IRQ2 is set when DMA interrupt pending goes from 0 to 1
            interrupt_registers.set_interrupt_flag(InterruptType::Dma);

            log::debug!("Set IRQ2 bit in I_STAT");
        }
    }

    pub fn cpu_wait_cycles(&self) -> u32 {
        self.cpu_wait_cycles
    }

    pub fn take_cpu_wait_cycles(&mut self) -> u32 {
        mem::take(&mut self.cpu_wait_cycles)
    }
}

fn transfer_block_from_ram(
    config: &mut ChannelConfig,
    memory: &Memory,
    mut write_fn: impl FnMut(u32),
) {
    let mut address = config.start_address & !3;
    for _ in 0..config.block_size {
        let word = memory.read_main_ram_u32(address);
        write_fn(word);

        address = config.step.apply(address);
    }

    config.start_address = address;
    config.num_blocks -= 1;
}

fn transfer_block_to_ram(
    config: &mut ChannelConfig,
    memory: &mut Memory,
    mut read_fn: impl FnMut() -> u32,
) {
    let mut address = config.start_address & !3;
    for _ in 0..config.block_size {
        let word = read_fn();
        memory.write_main_ram_u32(address, word);

        address = config.step.apply(address);
    }

    config.start_address = address;
    config.num_blocks -= 1;
}

fn progress_mdec_in_dma(
    config: &mut ChannelConfig,
    mdec: &mut MacroblockDecoder,
    memory: &Memory,
    cpu_cycle_counter: u64,
) -> u32 {
    if config.num_blocks == 0 {
        config.transfer_active = false;
        return 0;
    }

    transfer_block_from_ram(config, memory, |word| mdec.write_command(word));

    // TODO actual MDEC timing
    let cpu_wait_cycles = config.block_size * 17 / 16;
    config.next_active_cycles = cpu_cycle_counter + u64::from(cpu_wait_cycles);

    cpu_wait_cycles
}

// TODO reorder 8x8 blocks if MDEC is in 15bpp or 24bpp mode instead of assuming the MDEC code will do it
fn progress_mdec_out_dma(
    config: &mut ChannelConfig,
    mdec: &mut MacroblockDecoder,
    memory: &mut Memory,
    cpu_cycle_counter: u64,
) -> u32 {
    if config.num_blocks == 0 {
        config.transfer_active = false;
        return 0;
    }

    transfer_block_to_ram(config, memory, || mdec.read_data());

    // TODO actual MDEC decompression time
    let cpu_wait_cycles = config.block_size * 17 / 16;
    config.next_active_cycles = cpu_cycle_counter + u64::from(cpu_wait_cycles) + 256;

    cpu_wait_cycles
}

fn progress_gpu_dma(
    config: &mut ChannelConfig,
    gpu: &mut Gpu,
    memory: &mut Memory,
    pgxp_enabled: bool,
    cpu_cycle_counter: u64,
) -> u32 {
    match config.transfer_mode {
        TransferMode::Block => progress_gpu_block_dma(config, gpu, memory, cpu_cycle_counter),
        TransferMode::LinkedList => {
            progress_gpu_linked_list_dma(config, gpu, memory, pgxp_enabled, cpu_cycle_counter)
        }
        TransferMode::Burst => panic!("GPU DMA executed in Burst mode: {config:?}"),
    }
}

fn progress_gpu_block_dma(
    config: &mut ChannelConfig,
    gpu: &mut Gpu,
    memory: &mut Memory,
    cpu_cycle_counter: u64,
) -> u32 {
    if config.num_blocks == 0 {
        config.transfer_active = false;
        return 0;
    }

    match config.direction {
        DmaDirection::FromRam => {
            transfer_block_from_ram(config, memory, |word| gpu.write_gp0_command(word));
        }
        DmaDirection::ToRam => {
            transfer_block_to_ram(config, memory, || gpu.read_port());
        }
    }

    let cpu_wait_cycles = config.block_size * 17 / 16;

    // TODO actual GPU draw timing
    config.next_active_cycles = cpu_cycle_counter + u64::from(cpu_wait_cycles);

    cpu_wait_cycles
}

fn progress_gpu_linked_list_dma(
    config: &mut ChannelConfig,
    gpu: &mut Gpu,
    memory: &mut Memory,
    pgxp_enabled: bool,
    cpu_cycle_counter: u64,
) -> u32 {
    if config.start_address.bit(23) {
        // End marker encountered
        config.transfer_active = false;
        return 0;
    }

    match config.direction {
        DmaDirection::FromRam => {
            let address = config.start_address & !3;
            let node = memory.read_main_ram_u32(address);

            let data_word_count = node >> 24;
            for i in 0..data_word_count {
                let word_addr = address.wrapping_add(4 * (i + 1));
                let word = memory.read_main_ram_u32(word_addr);

                if pgxp_enabled {
                    let vertex = memory.read_main_ram_pgxp(word_addr);
                    gpu.write_gp0_command_pgxp(word, vertex);
                } else {
                    gpu.write_gp0_command(word);
                }
            }

            let next_address = node & 0xFFFFFF;
            config.start_address = next_address;

            // TODO actual GPU command timing
            let (cpu_wait_cycles, next_active_cycles) = if data_word_count == 0 {
                (0, cpu_cycle_counter + 16)
            } else {
                let cpu_wait_cycles = data_word_count * 17 / 16;
                let next_active_cycles = cpu_cycle_counter + u64::from(cpu_wait_cycles) + 128;
                (cpu_wait_cycles, next_active_cycles)
            };
            config.next_active_cycles = next_active_cycles;

            cpu_wait_cycles
        }
        DmaDirection::ToRam => panic!("GPU linked list DMA executed with direction device-to-RAM"),
    }
}

// CD-ROM DMA
// Copies data from the CD controller's data FIFO to main RAM
fn run_cdrom_dma(config: &ChannelConfig, memory: &mut Memory, cd_controller: &mut CdController) {
    let mut address = config.start_address & !3;
    let mut bytes = [0; 4];
    for _ in 0..config.block_size {
        for byte in &mut bytes {
            *byte = cd_controller.read_data_fifo();
        }

        memory.write_main_ram_u32(address, u32::from_le_bytes(bytes));
        address = address.wrapping_add(4);
    }
}

// SPU DMA
// Copies data between main RAM and SPU sound RAM
fn progress_spu_dma(
    config: &mut ChannelConfig,
    memory: &mut Memory,
    spu: &mut Spu,
    cpu_cycle_counter: u64,
) -> u32 {
    if config.num_blocks == 0 {
        config.transfer_active = false;
        return 0;
    }

    match config.direction {
        DmaDirection::FromRam => {
            transfer_block_from_ram(config, memory, |word| {
                spu.write_data_port(word as u16);
                spu.write_data_port((word >> 16) as u16);
            });
        }
        DmaDirection::ToRam => {
            transfer_block_to_ram(config, memory, || {
                let low_halfword = spu.read_data_port();
                let high_halfword = spu.read_data_port();
                u32::from(low_halfword) | (u32::from(high_halfword) << 16)
            });
        }
    }

    let cpu_wait_cycles = 4 * config.block_size;
    config.next_active_cycles = cpu_cycle_counter + u64::from(cpu_wait_cycles) + 64;
    cpu_wait_cycles
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
