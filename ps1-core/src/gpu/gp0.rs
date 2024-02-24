use crate::gpu::Gpu;
use std::array;

#[derive(Debug, Clone, Copy)]
pub enum Gp0Command {
    CpuToVramBlit,
    VramToCpuBlit,
}

#[derive(Debug, Clone, Copy)]
pub struct VramTransferFields {
    destination_x: u32,
    destination_y: u32,
    x_size: u32,
    y_size: u32,
    row: u32,
    col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncrementEffect {
    None,
    Finished,
}

impl VramTransferFields {
    fn vram_addr(&self) -> u32 {
        let vram_x = (self.destination_x + self.col) & 0x3FF;
        let vram_y = (self.destination_y + self.row) & 0x1FF;

        2048 * vram_y + 2 * vram_x
    }

    #[must_use]
    fn increment(&mut self) -> IncrementEffect {
        self.col += 1;
        if self.col == self.x_size {
            self.col = 0;

            self.row += 1;
            if self.row == self.y_size {
                return IncrementEffect::Finished;
            }
        }

        IncrementEffect::None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Gp0CommandState {
    WaitingForCommand,
    WaitingForParameters {
        command: Gp0Command,
        index: u8,
        remaining: u8,
    },
    ReceivingFromCpu(VramTransferFields),
    SendingToCpu(VramTransferFields),
}

impl Default for Gp0CommandState {
    fn default() -> Self {
        Self::WaitingForCommand
    }
}

impl Gp0CommandState {
    const CPU_TO_VRAM_BLIT: Self = Self::WaitingForParameters {
        command: Gp0Command::CpuToVramBlit,
        index: 0,
        remaining: 2,
    };

    const VRAM_TO_CPU_BLIT: Self = Self::WaitingForParameters {
        command: Gp0Command::VramToCpuBlit,
        index: 0,
        remaining: 2,
    };
}

#[derive(Debug, Clone)]
pub struct Gp0State {
    pub command_state: Gp0CommandState,
    pub parameters: [u32; 2],
}

impl Gp0State {
    pub fn new() -> Self {
        Self {
            command_state: Gp0CommandState::default(),
            parameters: array::from_fn(|_| 0),
        }
    }
}

impl Gpu {
    pub(super) fn read_vram_word(&mut self, mut fields: VramTransferFields) -> u32 {
        let mut word = 0_u32;
        for shift in [0, 16] {
            let vram_addr = fields.vram_addr() as usize;
            let halfword = u16::from_le_bytes([self.vram[vram_addr], self.vram[vram_addr + 1]]);
            word |= u32::from(halfword) << shift;

            if fields.increment() == IncrementEffect::Finished {
                log::trace!("VRAM-to-CPU blit finished, sending word {word:08X} to CPU");

                self.gp0_state.command_state = Gp0CommandState::WaitingForCommand;
                return word;
            }
        }

        log::trace!("VRAM-to-CPU blit in progress, sending word {word:08X} to CPU");

        self.gp0_state.command_state = Gp0CommandState::SendingToCpu(fields);
        word
    }

    pub fn write_gp0_command(&mut self, value: u32) {
        log::trace!("GP0 command write: {value:08X}");

        self.gp0_state.command_state = match self.gp0_state.command_state {
            Gp0CommandState::WaitingForCommand => match value >> 29 {
                5 => Gp0CommandState::CPU_TO_VRAM_BLIT,
                6 => Gp0CommandState::VRAM_TO_CPU_BLIT,
                _ => todo!("GP0 command {value:08X}"),
            },
            Gp0CommandState::WaitingForParameters {
                command,
                index,
                remaining,
            } => {
                self.gp0_state.parameters[index as usize] = value;
                if remaining == 1 {
                    self.execute_command(command)
                } else {
                    Gp0CommandState::WaitingForParameters {
                        command,
                        index: index + 1,
                        remaining: remaining - 1,
                    }
                }
            }
            Gp0CommandState::ReceivingFromCpu(fields) => {
                self.receive_vram_word_from_cpu(value, fields)
            }
            Gp0CommandState::SendingToCpu(..) => {
                panic!("unexpected write to GP0 command buffer during VRAM-to-CPU blit")
            }
        };
    }

    fn execute_command(&mut self, command: Gp0Command) -> Gp0CommandState {
        log::trace!("Executing GP0 command {command:?}");

        match command {
            Gp0Command::CpuToVramBlit => {
                let (destination_x, destination_y) =
                    parse_vram_position(self.gp0_state.parameters[0]);
                let (x_size, y_size) = parse_vram_size(self.gp0_state.parameters[1]);

                Gp0CommandState::ReceivingFromCpu(VramTransferFields {
                    destination_x,
                    destination_y,
                    x_size,
                    y_size,
                    row: 0,
                    col: 0,
                })
            }
            Gp0Command::VramToCpuBlit => {
                let (destination_x, destination_y) =
                    parse_vram_position(self.gp0_state.parameters[0]);
                let (x_size, y_size) = parse_vram_size(self.gp0_state.parameters[1]);

                Gp0CommandState::SendingToCpu(VramTransferFields {
                    destination_x,
                    destination_y,
                    x_size,
                    y_size,
                    row: 0,
                    col: 0,
                })
            }
        }
    }

    fn receive_vram_word_from_cpu(
        &mut self,
        value: u32,
        mut fields: VramTransferFields,
    ) -> Gp0CommandState {
        for halfword in [value & 0xFFFF, value >> 16] {
            let vram_addr = fields.vram_addr() as usize;
            self.vram[vram_addr] = halfword as u8;
            self.vram[vram_addr + 1] = (halfword >> 8) as u8;

            if fields.increment() == IncrementEffect::Finished {
                return Gp0CommandState::WaitingForCommand;
            }
        }

        Gp0CommandState::ReceivingFromCpu(fields)
    }
}

fn parse_vram_position(value: u32) -> (u32, u32) {
    let x = value & 0x3FF;
    let y = (value >> 16) & 0x1FF;
    (x, y)
}

fn parse_vram_size(value: u32) -> (u32, u32) {
    let x = (value.wrapping_sub(1) & 0x3FF) + 1;
    let y = ((value >> 16).wrapping_sub(1) & 0x1FF) + 1;
    (x, y)
}
