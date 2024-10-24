//! A software rasterizer that uses x86_64 SIMD intrinsics (AVX and AVX2)

#![allow(clippy::many_single_char_names)]

mod avx2;

use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::software::SoftwareRenderer;
use crate::gpu::rasterizer::{
    CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs, DrawTriangleArgs, RasterizerInterface,
    VramVramBlitArgs, cross_product_z, software, swap_vertices, vertices_valid,
};
use crate::gpu::registers::Registers;
use crate::gpu::{Color, Vertex, Vram, VramArray, WgpuResources};
use std::cmp;
use std::ops::{Deref, DerefMut};

// AVX2 loads/stores must be aligned to a 32-byte boundary
#[repr(align(32), C)]
#[derive(Debug, Clone)]
struct AlignedVram(VramArray);

impl AlignedVram {
    fn new_on_heap() -> Box<Self> {
        let mut vram = Box::<Self>::new_uninit();

        // SAFETY: AlignedVram's only field is zerofilled before assuming initialized
        unsafe {
            (*vram.as_mut_ptr()).0.fill(0);
            vram.assume_init()
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
    fn draw_triangle(
        &mut self,
        DrawTriangleArgs {
            vertices: mut v,
            mut shading,
            semi_transparent,
            semi_transparency_mode,
            mut texture_mapping,
            ..
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

        log::trace!("Triangle vertices: {v:?}");

        // Compute bounding box, clamped to display area
        let min_x = cmp::min(v[0].x, cmp::min(v[1].x, v[2].x));
        let max_x = cmp::max(v[0].x, cmp::max(v[1].x, v[2].x));
        let min_y = cmp::min(v[0].y, cmp::min(v[1].y, v[2].y));
        let max_y = cmp::max(v[0].y, cmp::max(v[1].y, v[2].y));

        if min_x > max_x || min_y > max_y {
            // Bounding box is empty, which can happen if the natural bounding box is entirely outside
            // of the drawing area
            return;
        }

        log::trace!("Bounding box: ({min_x}, {min_y}) to ({max_x}, {max_y})");

        // SAFETY: Guarded by is_x86_feature_detected!("avx2")
        unsafe {
            avx2::rasterize_triangle(
                &mut self.vram,
                draw_settings,
                (min_x, max_x),
                (min_y, max_y),
                v,
                shading,
                texture_mapping,
                semi_transparent.then_some(semi_transparency_mode),
            );
        }
    }

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
            x: vertex.x + draw_settings.draw_offset.x,
            y: vertex.y + draw_settings.draw_offset.y,
        });

        // SAFETY: Guarded by is_x86_feature_detected!("avx2")
        unsafe {
            avx2::rasterize_line(
                &mut self.vram,
                vertices,
                draw_settings.draw_area_top_left,
                draw_settings.draw_area_bottom_right,
                shading,
                semi_transparent.then_some(semi_transparency_mode),
                draw_settings.dithering_enabled,
                draw_settings.force_mask_bit,
                draw_settings.check_mask_bit,
            );
        }
    }

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

        if !draw_settings.is_drawing_area_valid() {
            return;
        }

        // SAFETY: Guarded by is_x86_feature_detected!("avx2")
        unsafe {
            avx2::rasterize_rectangle(
                &mut self.vram,
                draw_settings,
                top_left,
                width as i32,
                height as i32,
                color,
                texture_mapping,
                semi_transparent.then_some(semi_transparency_mode),
            );
        }
    }

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
        wgpu_resources: &mut WgpuResources,
    ) -> &wgpu::Texture {
        self.renderer.generate_frame_texture(registers, wgpu_resources, &self.vram)
    }

    fn clone_vram(&mut self) -> Vram {
        let vram_array: Box<VramArray> =
            self.vram.0.to_vec().into_boxed_slice().try_into().unwrap();
        vram_array.into()
    }
}
