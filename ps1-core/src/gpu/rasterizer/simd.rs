//! A software rasterizer that uses x86_64 SIMD intrinsics (AVX and AVX2)

#![allow(clippy::many_single_char_names)]

#[cfg(target_arch = "x86_64")]
mod avx2;

use crate::gpu;
use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::software::SoftwareRenderer;
use crate::gpu::rasterizer::{
    cross_product_z, software, swap_vertices, vertices_valid, Color, CpuVramBlitArgs, DrawLineArgs,
    DrawRectangleArgs, DrawTriangleArgs, LineShading, RasterizerInterface, RectangleTextureMapping,
    TriangleShading, Vertex, VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{Vram, WgpuResources};
use std::cmp;
use std::ops::{Deref, DerefMut};

// AVX2 loads/stores must be aligned to a 32-byte boundary
#[repr(align(32))]
#[derive(Debug, Clone)]
struct AlignedVram(Vram);

impl Deref for AlignedVram {
    type Target = Vram;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AlignedVram {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug)]
pub struct SimdSoftwareRasterizer {
    vram: Box<AlignedVram>,
    renderer: SoftwareRenderer,
}

impl SimdSoftwareRasterizer {
    #[allow(clippy::large_stack_arrays)]
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            vram: Box::new(AlignedVram([0; gpu::VRAM_LEN_HALFWORDS])),
            renderer: SoftwareRenderer::new(device),
        }
    }

    #[allow(clippy::large_stack_arrays)]
    pub fn from_vram(device: &wgpu::Device, vram: &Vram) -> Self {
        let mut aligned_vram = Box::new(AlignedVram([0; gpu::VRAM_LEN_HALFWORDS]));
        aligned_vram.0.copy_from_slice(vram.as_ref());

        Self { vram: aligned_vram, renderer: SoftwareRenderer::new(device) }
    }

    #[allow(clippy::large_stack_arrays)]
    pub fn clone_vram(&self) -> Box<Vram> {
        self.vram.0.to_vec().into_boxed_slice().try_into().unwrap()
    }
}

macro_rules! cpu_supports_required_features {
    () => {
        is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")
    };
}

impl RasterizerInterface for SimdSoftwareRasterizer {
    #[cfg(target_arch = "x86_64")]
    fn draw_triangle(
        &mut self,
        DrawTriangleArgs {
            vertices: mut v,
            mut shading,
            semi_transparent,
            semi_transparency_mode,
            mut texture_mapping,
        }: DrawTriangleArgs,
        draw_settings: &DrawSettings,
    ) {
        if !cpu_supports_required_features!() {
            log::error!("CPU does not support AVX2 and/or FMA instructions");
            return;
        }

        if !draw_settings.is_drawing_area_valid() {
            return;
        }

        if !vertices_valid(v[0], v[1]) || !vertices_valid(v[1], v[2]) || !vertices_valid(v[2], v[0])
        {
            return;
        }

        // Determine if the vertices are in clockwise order; if not, swap the first 2
        if cross_product_z(v[0], v[1], v[2]) < 0 {
            swap_vertices(&mut v, &mut shading, texture_mapping.as_mut());
        }

        let (draw_min_x, draw_min_y) = draw_settings.draw_area_top_left;
        let (draw_max_x, draw_max_y) = draw_settings.draw_area_bottom_right;

        let (x_offset, y_offset) = draw_settings.draw_offset;

        // Apply drawing offset to vertices
        let v = v.map(|vertex| Vertex { x: vertex.x + x_offset, y: vertex.y + y_offset });

        log::trace!("Triangle vertices: {v:?}");

        // Compute bounding box, clamped to display area
        let min_x =
            cmp::min(v[0].x, cmp::min(v[1].x, v[2].x)).clamp(draw_min_x as i32, draw_max_x as i32);
        let max_x =
            cmp::max(v[0].x, cmp::max(v[1].x, v[2].x)).clamp(draw_min_x as i32, draw_max_x as i32);
        let min_y =
            cmp::min(v[0].y, cmp::min(v[1].y, v[2].y)).clamp(draw_min_y as i32, draw_max_y as i32);
        let max_y =
            cmp::max(v[0].y, cmp::max(v[1].y, v[2].y)).clamp(draw_min_y as i32, draw_max_y as i32);

        if min_x > max_x || min_y > max_y {
            // Bounding box is empty, which can happen if the natural bounding box is entirely outside
            // of the drawing area
            return;
        }

        log::trace!("Bounding box: ({min_x}, {min_y}) to ({max_x}, {max_y})");

        let shading_avx2 = match shading {
            TriangleShading::Flat(color) => avx2::TriangleShadingAvx2::Flat(color),
            TriangleShading::Gouraud(colors) => {
                let r = colors.map(|color| f32::from(color.r));
                let g = colors.map(|color| f32::from(color.g));
                let b = colors.map(|color| f32::from(color.b));
                avx2::TriangleShadingAvx2::Gouraud { r, g, b }
            }
        };

        unsafe {
            avx2::rasterize_triangle(
                &mut self.vram,
                (min_x, max_x),
                (min_y, max_y),
                v,
                shading_avx2,
                texture_mapping.map(avx2::TriangleTextureMappingAvx2::new),
                semi_transparent.then_some(semi_transparency_mode),
                draw_settings.dithering_enabled,
                draw_settings.force_mask_bit,
                draw_settings.check_mask_bit,
            );
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn draw_triangle(&mut self, _args: DrawTriangleArgs, _draw_settings: &DrawSettings) {}

    // TODO write an AVX2 implementation of this
    fn draw_line(
        &mut self,
        DrawLineArgs { vertices, shading, semi_transparent, semi_transparency_mode }: DrawLineArgs,
        draw_settings: &DrawSettings,
    ) {
        if !draw_settings.is_drawing_area_valid() {
            return;
        }

        if !vertices_valid(vertices[0], vertices[1]) {
            return;
        }

        // Apply drawing offset
        let vertices = vertices.map(|vertex| Vertex {
            x: vertex.x + draw_settings.draw_offset.0,
            y: vertex.y + draw_settings.draw_offset.1,
        });

        let x_diff = vertices[1].x - vertices[0].x;
        let y_diff = vertices[1].y - vertices[0].y;

        if x_diff == 0 && y_diff == 0 {
            // Draw a single pixel with the color of the first vertex (if it's inside the drawing area)
            let color = match shading {
                LineShading::Flat(color) | LineShading::Gouraud([color, _]) => color,
            };
            crate::gpu::rasterizer::naive::draw_line_pixel(
                vertices[0],
                color,
                semi_transparent,
                semi_transparency_mode,
                draw_settings,
                &mut self.vram,
            );
            return;
        }

        let (r_diff, g_diff, b_diff) = match shading {
            LineShading::Flat(_) => (0, 0, 0),
            LineShading::Gouraud([color0, color1]) => (
                i32::from(color1.r) - i32::from(color0.r),
                i32::from(color1.g) - i32::from(color0.g),
                i32::from(color1.b) - i32::from(color0.b),
            ),
        };

        let (x_step, y_step, r_step, g_step, b_step) = if x_diff.abs() >= y_diff.abs() {
            let y_step = f64::from(y_diff) / f64::from(x_diff.abs());
            let r_step = f64::from(r_diff) / f64::from(x_diff.abs());
            let g_step = f64::from(g_diff) / f64::from(x_diff.abs());
            let b_step = f64::from(b_diff) / f64::from(x_diff.abs());
            (f64::from(x_diff.signum()), y_step, r_step, g_step, b_step)
        } else {
            let x_step = f64::from(x_diff) / f64::from(y_diff.abs());
            let r_step = f64::from(r_diff) / f64::from(y_diff.abs());
            let g_step = f64::from(g_diff) / f64::from(y_diff.abs());
            let b_step = f64::from(b_diff) / f64::from(y_diff.abs());
            (x_step, f64::from(y_diff.signum()), r_step, g_step, b_step)
        };

        let first_color = match shading {
            LineShading::Flat(color) | LineShading::Gouraud([color, _]) => color,
        };
        let mut r = f64::from(first_color.r);
        let mut g = f64::from(first_color.g);
        let mut b = f64::from(first_color.b);

        let mut x = f64::from(vertices[0].x);
        let mut y = f64::from(vertices[0].y);
        while x.round() as i32 != vertices[1].x || y.round() as i32 != vertices[1].y {
            let vertex = Vertex { x: x.round() as i32, y: y.round() as i32 };
            let color = Color::rgb(r.round() as u8, g.round() as u8, b.round() as u8);
            crate::gpu::rasterizer::naive::draw_line_pixel(
                vertex,
                color,
                semi_transparent,
                semi_transparency_mode,
                draw_settings,
                &mut self.vram,
            );

            x += x_step;
            y += y_step;
            r += r_step;
            g += g_step;
            b += b_step;
        }

        // Draw the last pixel
        let color = Color::rgb(r.round() as u8, g.round() as u8, b.round() as u8);
        crate::gpu::rasterizer::naive::draw_line_pixel(
            vertices[1],
            color,
            semi_transparent,
            semi_transparency_mode,
            draw_settings,
            &mut self.vram,
        );
    }

    #[cfg(target_arch = "x86_64")]
    fn draw_rectangle(
        &mut self,
        DrawRectangleArgs {
            top_left,
            width,
            height,
            color,
            semi_transparent,
            semi_transparency_mode,
            texture_mapping,
        }: DrawRectangleArgs,
        draw_settings: &DrawSettings,
    ) {
        if !cpu_supports_required_features!() {
            log::error!("CPU does not support AVX2 and/or FMA instructions");
            return;
        }

        let position = Vertex {
            x: top_left.x + draw_settings.draw_offset.0,
            y: top_left.y + draw_settings.draw_offset.1,
        };

        let (draw_min_x, draw_min_y) = draw_settings.draw_area_top_left;
        let (draw_max_x, draw_max_y) = draw_settings.draw_area_bottom_right;

        let min_x = cmp::max(draw_min_x as i32, position.x);
        let max_x = cmp::min(draw_max_x as i32, position.x + width as i32 - 1);
        let min_y = cmp::max(draw_min_y as i32, position.y);
        let max_y = cmp::min(draw_max_y as i32, position.y + height as i32 - 1);
        if min_x > max_x || min_y > max_y {
            // Drawing area is invalid
            return;
        }

        unsafe {
            avx2::rasterize_rectangle(
                &mut self.vram,
                (min_x, max_x),
                (min_y, max_y),
                color,
                texture_mapping.map(|mapping| RectangleTextureMapping {
                    u: [mapping.u[0].wrapping_add((min_x - position.x) as u8)],
                    v: [mapping.v[0].wrapping_add((min_y - position.y) as u8)],
                    ..mapping
                }),
                semi_transparent.then_some(semi_transparency_mode),
                draw_settings.force_mask_bit,
                draw_settings.check_mask_bit,
            );
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn draw_rectangle(&mut self, _args: DrawRectangleArgs, _draw_settings: &DrawSettings) {}

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        software::vram_fill(&mut self.vram, x, y, width, height, color);
    }

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]) {
        software::cpu_to_vram_blit(&mut self.vram, args, data);
    }

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>) {
        software::vram_to_cpu_blit(&self.vram, x, y, width, height, out);
    }

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs) {
        software::vram_to_vram_blit(&mut self.vram, args);
    }

    fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &WgpuResources,
    ) -> &wgpu::Texture {
        self.renderer.generate_frame_texture(registers, wgpu_resources, &self.vram)
    }
}
