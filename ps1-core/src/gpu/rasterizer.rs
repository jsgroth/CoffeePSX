//! Rasterizer interface and dispatch code

use bincode::{Decode, Encode};

use crate::gpu::gp0::{DrawSettings, SemiTransparencyMode, TexturePage, TextureWindow};
use crate::gpu::rasterizer::naive::NaiveSoftwareRasterizer;
use crate::gpu::registers::Registers;
use crate::gpu::{Vram, WgpuResources};

pub mod naive;
mod render;

#[derive(Debug, Clone, Copy, Default)]
pub struct Vertex {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Shading<const N: usize> {
    Flat(Color),
    Gouraud([Color; N]),
}

pub type LineShading = Shading<2>;
pub type TriangleShading = Shading<3>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureMappingMode {
    Raw,
    Modulated,
}

#[derive(Debug, Clone, Copy)]
pub struct TextureMapping<const N: usize> {
    pub mode: TextureMappingMode,
    pub texpage: TexturePage,
    pub window: TextureWindow,
    pub clut_x: u16,
    pub clut_y: u16,
    pub u: [u8; N],
    pub v: [u8; N],
}

pub type TriangleTextureMapping = TextureMapping<3>;
pub type RectangleTextureMapping = TextureMapping<1>;

#[derive(Debug)]
pub struct DrawTriangleArgs {
    pub vertices: [Vertex; 3],
    pub shading: TriangleShading,
    pub semi_transparent: bool,
    pub semi_transparency_mode: SemiTransparencyMode,
    pub texture_mapping: Option<TriangleTextureMapping>,
}

#[derive(Debug)]
pub struct DrawLineArgs {
    pub vertices: [Vertex; 2],
    pub shading: LineShading,
    pub semi_transparent: bool,
    pub semi_transparency_mode: SemiTransparencyMode,
}

#[derive(Debug)]
pub struct DrawRectangleArgs {
    pub top_left: Vertex,
    pub width: u32,
    pub height: u32,
    pub color: Color,
    pub semi_transparent: bool,
    pub semi_transparency_mode: SemiTransparencyMode,
    pub texture_mapping: Option<RectangleTextureMapping>,
}

#[derive(Debug)]
pub struct CpuVramBlitArgs {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub force_mask_bit: bool,
    pub check_mask_bit: bool,
}

#[derive(Debug)]
pub struct VramVramBlitArgs {
    pub source_x: u32,
    pub source_y: u32,
    pub dest_x: u32,
    pub dest_y: u32,
    pub width: u32,
    pub height: u32,
    pub force_mask_bit: bool,
    pub check_mask_bit: bool,
}

pub trait RasterizerInterface {
    fn draw_triangle(&mut self, args: DrawTriangleArgs, draw_settings: &DrawSettings);

    fn draw_line(&mut self, args: DrawLineArgs, draw_settings: &DrawSettings);

    fn draw_rectangle(&mut self, args: DrawRectangleArgs, draw_settings: &DrawSettings);

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color);

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]);

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>);

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs);

    fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &WgpuResources,
    ) -> &wgpu::Texture;
}

#[derive(Debug)]
pub enum Rasterizer {
    NaiveSoftware(NaiveSoftwareRasterizer),
}

impl Rasterizer {
    pub fn to_state(&self) -> RasterizerState {
        match self {
            Self::NaiveSoftware(rasterizer) => {
                RasterizerState::NaiveSoftware { vram: rasterizer.clone_vram() }
            }
        }
    }

    pub fn from_state(state: RasterizerState, wgpu_device: &wgpu::Device) -> Self {
        match state {
            RasterizerState::NaiveSoftware { vram } => {
                Rasterizer::NaiveSoftware(NaiveSoftwareRasterizer::from_vram(wgpu_device, vram))
            }
        }
    }
}

impl RasterizerInterface for Rasterizer {
    fn draw_triangle(&mut self, args: DrawTriangleArgs, draw_settings: &DrawSettings) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.draw_triangle(args, draw_settings),
        }
    }

    fn draw_line(&mut self, args: DrawLineArgs, draw_settings: &DrawSettings) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.draw_line(args, draw_settings),
        }
    }

    fn draw_rectangle(&mut self, args: DrawRectangleArgs, draw_settings: &DrawSettings) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.draw_rectangle(args, draw_settings),
        }
    }

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.vram_fill(x, y, width, height, color),
        }
    }

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.cpu_to_vram_blit(args, data),
        }
    }

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>) {
        match self {
            Self::NaiveSoftware(rasterizer) => {
                rasterizer.vram_to_cpu_blit(x, y, width, height, out);
            }
        }
    }

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs) {
        match self {
            Self::NaiveSoftware(rasterizer) => rasterizer.vram_to_vram_blit(args),
        }
    }

    fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &WgpuResources,
    ) -> &wgpu::Texture {
        match self {
            Self::NaiveSoftware(rasterizer) => {
                rasterizer.generate_frame_texture(registers, wgpu_resources)
            }
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum RasterizerState {
    NaiveSoftware { vram: Box<Vram> },
}
