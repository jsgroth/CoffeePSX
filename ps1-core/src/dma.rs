use crate::cpu::bus::OpSize;
use crate::gpu::Gpu;
use crate::memory::Memory;
use crate::num::U32Ext;
use std::array;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DmaDirection {
    #[default]
    ToRam = 0,
    FromRam = 1,
}

impl DmaDirection {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::FromRam
        } else {
            Self::ToRam
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Step {
    #[default]
    Forwards = 0,
    Backwards = 1,
}

impl Step {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Backwards
        } else {
            Self::Forwards
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SyncMode {
    // Transfer all at once; used for CD-ROM and OTC
    #[default]
    Zero = 0,
    // Transfer in blocks according to DMA request signal; used for GPU (data), SPU, and MDEC
    One = 1,
    // Linked list mode; used for GPU (command lists)
    Two = 2,
}

impl SyncMode {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Zero,
            1 => Self::One,
            2 => Self::Two,
            3 => {
                log::error!("Unexpected DMA sync mode of 3");
                Self::Zero
            }
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ChannelConfig {
    start_address: u32,
    // total length for SyncMode=0, block size for SyncMode=1
    block_size: u32,
    // only applicable for SyncMode=1
    num_blocks: u32,
    direction: DmaDirection,
    step: Step,
    sync_mode: SyncMode,
    chopping_enabled: bool,
    // Both chopping sizes are in 2^N units, e.g. N=3 -> 8
    chopping_dma_window_size: u32,
    chopping_cpu_window_size: u32,
}

impl ChannelConfig {
    fn otc() -> Self {
        Self {
            step: Step::Backwards,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
struct ControlRegister {
    channel_priority: [u8; 7],
    channel_enabled: [bool; 7],
}

impl ControlRegister {
    fn new() -> Self {
        // Control register value at power-on should be $07654321:
        //   Channel 0 has priority 1, channel 1 has priority 2, etc.
        //   All channels are disabled
        Self {
            channel_priority: array::from_fn(|i| (i + 1) as u8),
            channel_enabled: array::from_fn(|_| false),
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
    }
}

#[derive(Debug, Clone)]
pub struct DmaController {
    control: ControlRegister,
    channel_configs: [ChannelConfig; 7],
}

impl DmaController {
    pub fn new() -> Self {
        let mut channel_configs = array::from_fn(|_| ChannelConfig::default());
        channel_configs[6] = ChannelConfig::otc();

        Self {
            control: ControlRegister::new(),
            channel_configs,
        }
    }

    pub fn read_control(&self) -> u32 {
        self.control.read()
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
            | ((channel_config.sync_mode as u32) << 9)
            | (channel_config.chopping_dma_window_size << 16)
            | (channel_config.chopping_cpu_window_size << 20)
    }

    pub fn write_channel_control(
        &mut self,
        address: u32,
        value: u32,
        gpu: &mut Gpu,
        memory: &mut Memory,
    ) {
        let channel = (address >> 4) & 7;
        assert!(channel < 7, "DMA channel should always be 0-6");

        if channel != 6 {
            // Only channels 0-5 are allowed to change most channel settings
            // Channel 6 (OTC) can only start DMA through the control register, nothing else
            let channel_config = &mut self.channel_configs[channel as usize];
            channel_config.direction = DmaDirection::from_bit(value.bit(0));
            channel_config.step = Step::from_bit(value.bit(1));
            channel_config.chopping_enabled = value.bit(8);
            channel_config.sync_mode = SyncMode::from_bits(value >> 9);
            channel_config.chopping_dma_window_size = (value >> 16) & 7;
            channel_config.chopping_cpu_window_size = (value >> 20) & 7;

            log::trace!("DMA{channel} channel control write: {value:08X}");
            log::trace!("  Direction: {:?}", channel_config.direction);
            log::trace!("  Step: {:?}", channel_config.step);
            log::trace!("  Sync mode: {:?}", channel_config.sync_mode);
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

        if value.bit(24) {
            // DMA started
            match channel {
                2 => {
                    log::trace!("Running GPU DMA");
                    run_gpu_dma(&mut self.channel_configs[2], gpu, memory);
                    log::trace!("GPU DMA complete");
                }
                6 => {
                    log::trace!("Running OTC DMA");
                    run_otc_dma(&self.channel_configs[6], memory);
                    log::trace!("OTC DMA complete");
                }
                _ => todo!("DMA start on channel {channel}"),
            }
        }
    }
}

fn run_gpu_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &mut Memory) {
    match config.sync_mode {
        SyncMode::One => run_gpu_block_dma(config, gpu, memory),
        SyncMode::Two => run_gpu_linked_list_dma(config, gpu, memory),
        SyncMode::Zero => todo!("sync mode 0 unexpected for GPU DMA"),
    }
}

fn run_gpu_block_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &Memory) {
    match config.direction {
        DmaDirection::FromRam => {
            let mut address = config.start_address & !3;
            for _ in 0..config.block_size * config.num_blocks {
                let word = memory.read_main_ram(address, OpSize::Word);
                gpu.write_gp0_command(word);

                address = match config.step {
                    Step::Forwards => address.wrapping_add(4) & 0x1FFFFF,
                    Step::Backwards => address.wrapping_sub(4) & 0x1FFFFF,
                }
            }

            config.start_address = address;
        }
        DmaDirection::ToRam => todo!("GPU block DMA from VRAM to CPU RAM"),
    }
}

fn run_gpu_linked_list_dma(config: &mut ChannelConfig, gpu: &mut Gpu, memory: &Memory) {
    match config.direction {
        DmaDirection::FromRam => {
            let mut address = config.start_address & !3;
            loop {
                // TODO timing, don't do all at once
                let node = memory.read_main_ram(address, OpSize::Word);

                let word_count = node >> 24;
                for i in 0..word_count {
                    let word_addr = address.wrapping_add(4 * (i + 1));
                    let word = memory.read_main_ram(word_addr, OpSize::Word);
                    gpu.write_gp0_command(word);
                }

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

        memory.write_main_ram(address, next_addr, OpSize::Word);

        address = next_addr;
    }
}
