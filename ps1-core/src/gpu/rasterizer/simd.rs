//! A software rasterizer that uses x86_64 SIMD intrinsics (AVX and AVX2)

#![allow(clippy::many_single_char_names)]

#[cfg(target_arch = "x86_64")]
mod avx2;

use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::software::SoftwareRenderer;
use crate::gpu::rasterizer::{
    cross_product_z, software, swap_vertices, vertices_valid, Color, CpuVramBlitArgs, DrawLineArgs,
    DrawRectangleArgs, DrawTriangleArgs, RasterizerInterface, RectangleTextureMapping, Vertex,
    VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{Vram, VramArray, WgpuResources};
use std::alloc::Layout;
use std::ops::{Deref, DerefMut};
use std::{alloc, cmp};

// AVX2 loads/stores must be aligned to a 32-byte boundary
#[repr(align(32), C)]
#[derive(Debug, Clone)]
struct AlignedVram(VramArray);

impl AlignedVram {
    fn new_on_heap() -> Box<Self> {
        // SAFETY: The pointer is allocated using Layout of Self (which is not a zero-sized type)
        // and then dereferenced only as a *mut Self. The struct's only field is zerofilled before
        // returning the pointer inside a Box.
        // TODO use Box functions when Box::new_uninit() is stabilized
        unsafe {
            let layout = Layout::new::<Self>();
            let memory = alloc::alloc(layout).cast::<Self>();
            if memory.is_null() {
                alloc::handle_alloc_error(layout);
            }

            (*memory).0.fill(0);

            Box::from_raw(memory)
        }
    }
}

impl Deref for AlignedVram {
    type Target = VramArray;

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
        Self { vram: AlignedVram::new_on_heap(), renderer: SoftwareRenderer::new(device) }
    }

    #[allow(clippy::large_stack_arrays)]
    pub fn from_vram(device: &wgpu::Device, vram: &Vram) -> Self {
        let mut aligned_vram = AlignedVram::new_on_heap();
        aligned_vram.0.copy_from_slice(vram.as_ref());

        Self { vram: aligned_vram, renderer: SoftwareRenderer::new(device) }
    }
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
        if !is_x86_feature_detected!("avx2") {
            log::error!("CPU does not support AVX2 instructions");
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

        unsafe {
            avx2::rasterize_triangle(
                &mut self.vram,
                (min_x, max_x),
                (min_y, max_y),
                v,
                shading,
                texture_mapping,
                semi_transparent.then_some(semi_transparency_mode),
                draw_settings.dithering_enabled,
                draw_settings.force_mask_bit,
                draw_settings.check_mask_bit,
            );
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn draw_triangle(&mut self, _args: DrawTriangleArgs, _draw_settings: &DrawSettings) {}

    #[cfg(target_arch = "x86_64")]
    fn draw_line(
        &mut self,
        DrawLineArgs { vertices, shading, semi_transparent, semi_transparency_mode }: DrawLineArgs,
        draw_settings: &DrawSettings,
    ) {
        if !is_x86_feature_detected!("avx2") {
            log::error!("CPU does not support AVX2 instructions");
            return;
        }

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

        unsafe {
            avx2::rasterize_line(
                &mut self.vram,
                vertices,
                (
                    draw_settings.draw_area_top_left.0 as i32,
                    draw_settings.draw_area_bottom_right.0 as i32,
                ),
                (
                    draw_settings.draw_area_top_left.1 as i32,
                    draw_settings.draw_area_bottom_right.1 as i32,
                ),
                shading,
                semi_transparent.then_some(semi_transparency_mode),
                draw_settings.dithering_enabled,
                draw_settings.force_mask_bit,
                draw_settings.check_mask_bit,
            );
        }
    }

    #[cfg(not_target_arch = "x86_64")]
    fn draw_line(&mut self, _args: DrawLineArgs, _draw_settings: &DrawSettings) {}

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
        if !is_x86_feature_detected!("avx2") {
            log::error!("CPU does not support AVX2 instructions");
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

    fn clone_vram(&self) -> Vram {
        let vram_array: Box<VramArray> =
            self.vram.0.to_vec().into_boxed_slice().try_into().unwrap();
        vram_array.into()
    }
}
