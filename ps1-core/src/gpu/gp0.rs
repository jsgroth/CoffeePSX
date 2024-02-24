use crate::gpu::Gpu;
use crate::num::U32Ext;
use std::array;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectangleSize {
    Variable,
    One,
    Eight,
    Sixteen,
}

impl RectangleSize {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Variable,
            1 => Self::One,
            2 => Self::Eight,
            3 => Self::Sixteen,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Gp0Command {
    DrawRectangle {
        size: RectangleSize,
        textured: bool,
        semi_transparent: bool,
        raw_texture: bool,
        color: u16,
    },
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

    fn draw_rectangle(value: u32) -> Self {
        let size = RectangleSize::from_bits(value >> 27);
        let textured = value.bit(26);
        let semi_transparent = value.bit(25);
        let raw_texture = value.bit(24);
        let color = parse_command_color(value);

        let parameters = 1 + u8::from(textured) + u8::from(size == RectangleSize::Variable);

        let command = Gp0Command::DrawRectangle {
            size,
            textured,
            semi_transparent,
            raw_texture,
            color,
        };

        Self::WaitingForParameters {
            command,
            index: 0,
            remaining: parameters,
        }
    }
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
                3 => Gp0CommandState::draw_rectangle(value),
                5 => Gp0CommandState::CPU_TO_VRAM_BLIT,
                6 => Gp0CommandState::VRAM_TO_CPU_BLIT,
                _ => {
                    log::error!("unimplemented GP0 command {value:08X}");
                    Gp0CommandState::WaitingForCommand
                }
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
            Gp0Command::DrawRectangle {
                size,
                textured,
                semi_transparent,
                raw_texture,
                color,
            } => {
                if textured || semi_transparent || raw_texture || size != RectangleSize::One {
                    todo!("draw rectangle {command:?}");
                }

                self.draw_pixel(color);

                Gp0CommandState::WaitingForCommand
            }
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

    fn draw_pixel(&mut self, color: u16) {
        let (x, y) = parse_vram_position(self.gp0_state.parameters[0]);

        log::trace!("Drawing pixel at X={x}, Y={y} with color {color:04X}");

        let vram_addr = (2048 * y + 2 * x) as usize;
        let [lsb, msb] = color.to_le_bytes();
        self.vram[vram_addr] = lsb;
        self.vram[vram_addr + 1] = msb;
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

fn parse_command_color(value: u32) -> u16 {
    // Drop the lowest 3 bits of each component
    let r = ((value >> 3) & 0x1F) as u16;
    let g = ((value >> 11) & 0x1F) as u16;
    let b = ((value >> 19) & 0x1F) as u16;

    r | (g << 5) | (b << 10)
}
