mod rasterize;

use crate::gpu::gp0::rasterize::{Shading, TextureMode, TextureParameters};
use crate::gpu::Gpu;
use crate::num::U32Ext;
use std::array;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Vertex {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    fn truncate_to_15_bit(self) -> u16 {
        let r: u16 = (self.r >> 3).into();
        let g: u16 = (self.g >> 3).into();
        let b: u16 = (self.b >> 3).into();

        // TODO mask bit?
        r | (g << 5) | (b << 10)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolygonVertices {
    Three,
    Four,
}

impl PolygonVertices {
    fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Four
        } else {
            Self::Three
        }
    }
}

impl From<PolygonVertices> for u8 {
    fn from(value: PolygonVertices) -> Self {
        match value {
            PolygonVertices::Three => 3,
            PolygonVertices::Four => 4,
        }
    }
}

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
pub struct PolygonCommandParameters {
    pub vertices: PolygonVertices,
    pub gouraud_shading: bool,
    pub textured: bool,
    pub semi_transparent: bool,
    pub raw_texture: bool,
    pub color: Color,
}

#[derive(Debug, Clone, Copy)]
pub enum DrawCommand {
    DrawPolygon(PolygonCommandParameters),
    DrawRectangle {
        size: RectangleSize,
        textured: bool,
        semi_transparent: bool,
        raw_texture: bool,
        color: Color,
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
        command: DrawCommand,
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
        command: DrawCommand::CpuToVramBlit,
        index: 0,
        remaining: 2,
    };

    const VRAM_TO_CPU_BLIT: Self = Self::WaitingForParameters {
        command: DrawCommand::VramToCpuBlit,
        index: 0,
        remaining: 2,
    };

    fn draw_polygon(value: u32) -> Self {
        let gouraud_shading = value.bit(28);
        let vertices = PolygonVertices::from_bit(value.bit(27));
        let textured = value.bit(26);
        let semi_transparent = value.bit(25);
        let raw_texture = value.bit(24);
        let color = parse_command_color(value);

        let vertex_count: u8 = vertices.into();

        // Each vertex requires 1-3 parameters:
        // - 1 parameter for the coordinates
        // - 1 parameter for the color (only if Gouraud shading is enabled, and not for the first
        //   vertex because it uses the color from the command word)
        // - 1 parameter for the U/V texture coordinates (only for textured polygons)
        let parameters = vertex_count * (1 + u8::from(textured))
            + (vertex_count - 1) * u8::from(gouraud_shading);

        let command = DrawCommand::DrawPolygon(PolygonCommandParameters {
            vertices,
            gouraud_shading,
            textured,
            semi_transparent,
            raw_texture,
            color,
        });

        Self::WaitingForParameters {
            command,
            index: 0,
            remaining: parameters,
        }
    }

    fn draw_rectangle(value: u32) -> Self {
        let size = RectangleSize::from_bits(value >> 27);
        let textured = value.bit(26);
        let semi_transparent = value.bit(25);
        let raw_texture = value.bit(24);
        let color = parse_command_color(value);

        let parameters = 1 + u8::from(textured) + u8::from(size == RectangleSize::Variable);

        let command = DrawCommand::DrawRectangle {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SemiTransparencyMode {
    // B/2 + F/2
    #[default]
    Average = 0,
    // B + F
    Add = 1,
    // B - F
    Subtract = 2,
    // B + F/4
    AddQuarter = 3,
}

impl SemiTransparencyMode {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Average,
            1 => Self::Add,
            2 => Self::Subtract,
            3 => Self::AddQuarter,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextureColorDepthBits {
    #[default]
    Four = 0,
    Eight = 1,
    Fifteen = 2,
}

impl TextureColorDepthBits {
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Four,
            1 => Self::Eight,
            // Setting 3 ("reserved") functions the same as setting 2 (15-bit)
            2 | 3 => Self::Fifteen,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TexturePage {
    // In 64-halfword steps
    pub x_base: u32,
    // 0 or 256 (can technically also be 512 or 768 but those are not supported on retail consoles)
    pub y_base: u32,
    pub semi_transparency_mode: SemiTransparencyMode,
    pub color_depth: TextureColorDepthBits,
    pub rectangle_x_flip: bool,
    pub rectangle_y_flip: bool,
}

impl TexturePage {
    fn from_command_word(command: u32) -> Self {
        Self {
            x_base: command & 0xF,
            y_base: 256 * ((command >> 4) & 1),
            semi_transparency_mode: SemiTransparencyMode::from_bits(command >> 5),
            color_depth: TextureColorDepthBits::from_bits(command >> 7),
            rectangle_x_flip: command.bit(12),
            rectangle_y_flip: command.bit(13),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextureWindow {
    // All values in 8-pixel steps
    pub x_mask: u32,
    pub y_mask: u32,
    pub x_offset: u32,
    pub y_offset: u32,
}

impl TextureWindow {
    fn from_command_word(command: u32) -> Self {
        Self {
            x_mask: command & 0x1F,
            y_mask: (command >> 5) & 0x1F,
            x_offset: (command >> 10) & 0x1F,
            y_offset: (command >> 15) & 0x1F,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DrawSettings {
    pub drawing_in_display_allowed: bool,
    pub dithering_enabled: bool,
    pub draw_area_top_left: (u32, u32),
    pub draw_area_bottom_right: (u32, u32),
    pub draw_offset: (i32, i32),
    pub force_mask_bit: bool,
    pub check_mask_bit: bool,
}

const PARAMETERS_LEN: usize = 8;

#[derive(Debug, Clone)]
pub struct Gp0State {
    pub command_state: Gp0CommandState,
    pub parameters: [u32; PARAMETERS_LEN],
    pub global_texture_page: TexturePage,
    pub texture_window: TextureWindow,
    pub draw_settings: DrawSettings,
}

impl Gp0State {
    pub fn new() -> Self {
        Self {
            command_state: Gp0CommandState::default(),
            parameters: array::from_fn(|_| 0),
            global_texture_page: TexturePage::default(),
            texture_window: TextureWindow::default(),
            draw_settings: DrawSettings::default(),
        }
    }
}

impl Gpu {
    pub(super) fn read_vram_word_for_cpu(&mut self, mut fields: VramTransferFields) -> u32 {
        let mut word = 0_u32;
        for shift in [0, 16] {
            let vram_addr = fields.vram_addr() as usize;
            let halfword = u16::from_le_bytes([self.vram[vram_addr], self.vram[vram_addr + 1]]);
            word |= u32::from(halfword) << shift;

            if fields.increment() == IncrementEffect::Finished {
                log::trace!("VRAM-to-CPU blit finished, sending word {word:08X} to CPU");

                self.gp0.command_state = Gp0CommandState::WaitingForCommand;
                return word;
            }
        }

        log::trace!("VRAM-to-CPU blit in progress, sending word {word:08X} to CPU");

        self.gp0.command_state = Gp0CommandState::SendingToCpu(fields);
        word
    }

    pub fn write_gp0_command(&mut self, value: u32) {
        log::trace!("GP0 command write: {value:08X}");

        self.gp0.command_state = match self.gp0.command_state {
            Gp0CommandState::WaitingForCommand => match value >> 29 {
                0 => {
                    match value >> 24 {
                        0x00 => {
                            // GP0($00): Apparently a no-op? Functionally unknown
                            Gp0CommandState::WaitingForCommand
                        }
                        0x01 => {
                            // GP0($01): Clear texture cache
                            // TODO emulate texture cache?
                            Gp0CommandState::WaitingForCommand
                        }
                        0x1F => {
                            // GP0($1F): Set GPU IRQ flag
                            // Apparently nothing uses this feature? Except for one game that seems
                            // to accidentally send a GP0($1F) command
                            todo!("GP0($1F) - set GPU IRQ")
                        }
                        _ => todo!("GP0 command: {value:08X}"),
                    }
                }
                1 => Gp0CommandState::draw_polygon(value),
                3 => Gp0CommandState::draw_rectangle(value),
                5 => Gp0CommandState::CPU_TO_VRAM_BLIT,
                6 => Gp0CommandState::VRAM_TO_CPU_BLIT,
                7 => {
                    // All commands starting with 111 are settings commands that take no parameters
                    self.execute_settings_command(value);
                    Gp0CommandState::WaitingForCommand
                }
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
                self.gp0.parameters[index as usize] = value;
                if remaining == 1 {
                    self.execute_draw_command(command)
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

    fn execute_draw_command(&mut self, command: DrawCommand) -> Gp0CommandState {
        log::trace!("Executing GP0 command {command:?}");

        match command {
            DrawCommand::DrawPolygon(parameters) => {
                if parameters.semi_transparent || parameters.raw_texture {
                    todo!("draw polygon {command:?}");
                }

                self.draw_polygon(parameters);

                Gp0CommandState::WaitingForCommand
            }
            DrawCommand::DrawRectangle {
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
            DrawCommand::CpuToVramBlit => {
                let (destination_x, destination_y) = parse_vram_position(self.gp0.parameters[0]);
                let (x_size, y_size) = parse_vram_size(self.gp0.parameters[1]);

                Gp0CommandState::ReceivingFromCpu(VramTransferFields {
                    destination_x,
                    destination_y,
                    x_size,
                    y_size,
                    row: 0,
                    col: 0,
                })
            }
            DrawCommand::VramToCpuBlit => {
                let (destination_x, destination_y) = parse_vram_position(self.gp0.parameters[0]);
                let (x_size, y_size) = parse_vram_size(self.gp0.parameters[1]);

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

    fn execute_settings_command(&mut self, command: u32) {
        // Highest 8 bits determine operation
        match command >> 24 {
            0xE1 => {
                // GP0($E1): Texture page & draw mode settings
                self.gp0.global_texture_page = TexturePage::from_command_word(command);
                self.gp0.draw_settings.drawing_in_display_allowed = command.bit(10);
                self.gp0.draw_settings.dithering_enabled = command.bit(9);

                log::trace!("Executed texture page / draw mode command: {command:08X}");
                log::trace!("  Global texture page: {:?}", self.gp0.global_texture_page);
                log::trace!(
                    "  Drawing allowed in display area: {}",
                    self.gp0.draw_settings.drawing_in_display_allowed
                );
                log::trace!(
                    "  Dithering from 24-bit to 15-bit enabled: {}",
                    self.gp0.draw_settings.dithering_enabled
                );
            }
            0xE2 => {
                // GP0($E2): Texture window settings
                self.gp0.texture_window = TextureWindow::from_command_word(command);

                log::trace!("Executed texture window settings command: {command:08X}");
                log::trace!("  Texture window: {:?}", self.gp0.texture_window);
            }
            0xE3 => {
                // GP0($E3): Drawing area top-left coordinates
                let x1 = command & 0x3FF;
                let y1 = (command >> 10) & 0x1FF;
                self.gp0.draw_settings.draw_area_top_left = (x1, y1);

                log::trace!("Executed drawing area top-left command: {command:08X}");
                log::trace!(
                    "  (X1, Y1) = {:?}",
                    self.gp0.draw_settings.draw_area_top_left
                );
            }
            0xE4 => {
                // GP0($E4): Drawing area bottom-right coordinates
                let x2 = command & 0x3FF;
                let y2 = (command >> 10) & 0x1FF;
                self.gp0.draw_settings.draw_area_bottom_right = (x2, y2);

                log::trace!("Executed drawing area bottom-right command: {command:08X}");
                log::trace!(
                    "  (X2, Y2) = {:?}",
                    self.gp0.draw_settings.draw_area_bottom_right
                );
            }
            0xE5 => {
                // GP0($E5): Drawing offset
                // Both values are signed 11-bit integers (-1024 to +1023)
                let x_offset = parse_signed_11_bit(command);
                let y_offset = parse_signed_11_bit(command >> 11);
                self.gp0.draw_settings.draw_offset = (x_offset, y_offset);

                log::trace!("Executed draw offset command: {command:08X}");
                log::trace!(
                    "  (X offset, Y offset) = {:?}",
                    self.gp0.draw_settings.draw_offset
                );
            }
            0xE6 => {
                // GP0($E6): Mask bit settings
                self.gp0.draw_settings.force_mask_bit = command.bit(0);
                self.gp0.draw_settings.check_mask_bit = command.bit(1);

                log::trace!("Executed mask bit settings command: {command:08X}");
                log::trace!(
                    "  Force mask bit: {}",
                    self.gp0.draw_settings.force_mask_bit
                );
                log::trace!(
                    "  Check mask bit on draw: {}",
                    self.gp0.draw_settings.check_mask_bit
                );
            }
            _ => todo!("GP0 settings command {command:08X}"),
        }
    }

    fn draw_polygon(&mut self, command_parameters: PolygonCommandParameters) {
        let (first_params, second_params) =
            parse_draw_polygon_parameters(command_parameters, &self.gp0.parameters);
        rasterize::triangle(first_params, &self.gp0.draw_settings, &mut self.vram);
        if let Some(second_params) = second_params {
            rasterize::triangle(second_params, &self.gp0.draw_settings, &mut self.vram);
        }
    }

    fn draw_pixel(&mut self, color: Color) {
        let (x, y) = parse_vram_position(self.gp0.parameters[0]);

        log::trace!("Drawing pixel at X={x}, Y={y} with color {color:02X?}");

        let vram_addr = (2048 * y + 2 * x) as usize;
        let [lsb, msb] = color.truncate_to_15_bit().to_le_bytes();
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

fn parse_command_color(value: u32) -> Color {
    let r = value as u8;
    let g = (value >> 8) as u8;
    let b = (value >> 16) as u8;

    Color { r, g, b }
}

fn parse_vertex_coordinates(parameter: u32) -> Vertex {
    // Vertex coordinates are signed 11-bit values, X in the low halfword and Y in the high halfword
    let x = parse_signed_11_bit(parameter);
    let y = parse_signed_11_bit(parameter >> 16);

    Vertex { x, y }
}

fn parse_signed_11_bit(word: u32) -> i32 {
    ((word as i32) << 21) >> 21
}

#[derive(Debug, Clone)]
pub struct DrawPolygonParameters {
    vertices: [Vertex; 3],
    shading: Shading,
    texture_params: TextureParameters,
    texture_mode: TextureMode,
}

fn parse_draw_polygon_parameters(
    command_parameters: PolygonCommandParameters,
    mut parameters: &[u32],
) -> (DrawPolygonParameters, Option<DrawPolygonParameters>) {
    let mut vertices = [Vertex::default(); 4];
    let mut colors = [Color::default(); 4];
    let mut u = [0; 4];
    let mut v = [0; 4];
    let mut clut_x = 0;
    let mut clut_y = 0;
    let mut texpage = TexturePage::default();

    colors[0] = command_parameters.color;

    for vertex_idx in 0..command_parameters.vertices.into() {
        if vertex_idx != 0 && command_parameters.gouraud_shading {
            colors[vertex_idx as usize] = parse_command_color(parameters[0]);
            parameters = &parameters[1..];
        }

        vertices[vertex_idx as usize] = parse_vertex_coordinates(parameters[0]);
        parameters = &parameters[1..];

        if command_parameters.textured {
            match vertex_idx {
                0 => {
                    clut_x = ((parameters[0] >> 16) & 0x3F) as u16;
                    clut_y = ((parameters[0] >> 22) & 0x1FF) as u16;
                }
                1 => {
                    texpage = TexturePage::from_command_word(parameters[0] >> 16);
                }
                _ => {}
            }

            u[vertex_idx as usize] = parameters[0] as u8;
            v[vertex_idx as usize] = (parameters[0] >> 8) as u8;
            parameters = &parameters[1..];
        }
    }

    let texture_mode = TextureMode::from_command_params(command_parameters);

    let first_parameters = DrawPolygonParameters {
        vertices: [vertices[0], vertices[1], vertices[2]],
        shading: if command_parameters.gouraud_shading {
            Shading::Gouraud(colors[0], colors[1], colors[2])
        } else {
            Shading::Flat(colors[0])
        },
        texture_params: TextureParameters {
            texpage: texpage.clone(),
            clut_x,
            clut_y,
            u: [u[0], u[1], u[2]],
            v: [v[0], v[1], v[2]],
            semi_transparent: command_parameters.semi_transparent,
        },
        texture_mode,
    };

    match command_parameters.vertices {
        PolygonVertices::Three => (first_parameters, None),
        PolygonVertices::Four => {
            let second_parameters = DrawPolygonParameters {
                vertices: [vertices[1], vertices[2], vertices[3]],
                shading: if command_parameters.gouraud_shading {
                    Shading::Gouraud(colors[1], colors[2], colors[3])
                } else {
                    Shading::Flat(colors[0])
                },
                texture_params: TextureParameters {
                    texpage,
                    clut_x,
                    clut_y,
                    u: [u[1], u[2], u[3]],
                    v: [v[1], v[2], v[3]],
                    semi_transparent: command_parameters.semi_transparent,
                },
                texture_mode,
            };

            (first_parameters, Some(second_parameters))
        }
    }
}
