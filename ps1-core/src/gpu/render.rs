use crate::api::ColorDepthBits;
use crate::gpu;
use crate::gpu::registers::{Registers, VerticalResolution};
use crate::gpu::{Gpu, Vram};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::rc::Rc;
use std::{cmp, iter};

const SCREEN_LEFT: i32 = 0x260;
const SCREEN_RIGHT: i32 = 0xC60;

const NTSC_SCREEN_TOP: i32 = 16;
const NTSC_SCREEN_BOTTOM: i32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameCoords {
    // Frame buffer coordinates
    frame_x: u32,
    frame_y: u32,
    // Pixel offsets to apply to the frame buffer coordinates (if X1/Y1 are less than standard values)
    display_x_offset: u32,
    display_y_offset: u32,
    // First X/Y pixel values to display (if X1/Y1 are greater than standard values)
    display_x_start: u32,
    display_y_start: u32,
    // Number of pixels to render from the frame buffer in each dimension
    display_width: u32,
    display_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FrameSize {
    width: u32,
    height: u32,
}

impl Display for FrameSize {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ width: {}, height: {} }}", self.width, self.height)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Color {
    const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };

    fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayConfig {
    pub crop_vertical_overscan: bool,
    pub dump_vram: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { crop_vertical_overscan: true, dump_vram: false }
    }
}

type FrameBuffer = [Color; gpu::VRAM_LEN_HALFWORDS];

#[derive(Debug)]
pub struct WgpuResources {
    pub device: Rc<wgpu::Device>,
    pub queue: Rc<wgpu::Queue>,
    pub display_config: DisplayConfig,
    frame_buffer: Box<FrameBuffer>,
    frame_textures: HashMap<FrameSize, wgpu::Texture>,
    clear_pipeline: wgpu::RenderPipeline,
}

impl WgpuResources {
    pub fn new(
        device: Rc<wgpu::Device>,
        queue: Rc<wgpu::Queue>,
        display_config: DisplayConfig,
    ) -> Self {
        let clear_pipeline = create_clear_pipeline(&device);

        Self {
            device,
            queue,
            display_config,
            frame_buffer: vec![Color::default(); gpu::VRAM_LEN_HALFWORDS]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            frame_textures: HashMap::new(),
            clear_pipeline,
        }
    }

    fn clear_frame(&mut self, frame_size: FrameSize) -> &wgpu::Texture {
        let texture =
            get_or_create_frame_texture(&self.device, frame_size, &mut self.frame_textures);
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: "clear_encoder".into(),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: "clear_render_pass".into(),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.clear_pipeline);

            render_pass.draw(0..4, 0..1);
        }

        self.queue.submit(iter::once(encoder.finish()));

        texture
    }

    fn write_frame(
        &mut self,
        frame_size: FrameSize,
        frame_coords: FrameCoords,
        color_depth: ColorDepthBits,
        vram: &Vram,
    ) -> &wgpu::Texture {
        populate_frame_buffer(frame_size, frame_coords, color_depth, vram, &mut self.frame_buffer);

        let frame_texture =
            get_or_create_frame_texture(&self.device, frame_size, &mut self.frame_textures);

        self.queue.write_texture(
            frame_texture.as_image_copy(),
            bytemuck::cast_slice(self.frame_buffer.as_ref()),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(1024 * 4),
                rows_per_image: None,
            },
            frame_texture.size(),
        );

        frame_texture
    }
}

impl Gpu {
    pub(super) fn write_frame_texture(&mut self) -> &wgpu::Texture {
        if self.wgpu_resources.display_config.dump_vram {
            return self.wgpu_resources.write_frame(
                FrameSize { width: 1024, height: 512 },
                FrameCoords {
                    frame_x: 0,
                    frame_y: 0,
                    display_x_offset: 0,
                    display_y_offset: 0,
                    display_x_start: 0,
                    display_y_start: 0,
                    display_width: 1024,
                    display_height: 512,
                },
                ColorDepthBits::Fifteen,
                &self.vram,
            );
        }

        let (frame_coords, frame_size) =
            compute_frame_location(&self.registers, self.wgpu_resources.display_config);
        let Some(frame_coords) = frame_coords else {
            return self.wgpu_resources.clear_frame(frame_size);
        };

        if !self.registers.display_enabled {
            return self.wgpu_resources.clear_frame(frame_size);
        }

        return self.wgpu_resources.write_frame(
            frame_size,
            frame_coords,
            self.registers.display_area_color_depth,
            &self.vram,
        );
    }
}

fn compute_frame_location(
    registers: &Registers,
    display_config: DisplayConfig,
) -> (Option<FrameCoords>, FrameSize) {
    let crop_v_overscan = display_config.crop_vertical_overscan;
    let screen_top = if crop_v_overscan { NTSC_SCREEN_TOP + 8 } else { NTSC_SCREEN_TOP };
    let screen_bottom = if crop_v_overscan { NTSC_SCREEN_BOTTOM - 8 } else { NTSC_SCREEN_BOTTOM };

    let dot_clock_divider: i32 = registers.dot_clock_divider().into();
    let frame_width = (SCREEN_RIGHT - SCREEN_LEFT) / dot_clock_divider;

    let height_multipler =
        if registers.interlaced && registers.v_resolution == VerticalResolution::Double {
            2
        } else {
            1
        };
    let frame_height = (if crop_v_overscan { 224 } else { 240 }) * height_multipler;

    let frame_size = FrameSize { width: frame_width as u32, height: frame_height as u32 };

    let x1 = registers.x_display_range.0 as i32;
    let x2 = cmp::min(SCREEN_RIGHT, registers.x_display_range.1 as i32);
    let y1 = registers.y_display_range.0 as i32;
    let y2 = cmp::min(screen_bottom, registers.y_display_range.1 as i32);

    let mut display_width_clocks = x2 - x1;
    let mut display_height = (y2 - y1) * height_multipler;

    let mut display_x_offset = 0;
    if x1 < SCREEN_LEFT {
        display_x_offset = (SCREEN_LEFT - x1) / dot_clock_divider;
        display_width_clocks -= SCREEN_LEFT - x1;
    }

    let mut display_y_offset = 0;
    if y1 < screen_top {
        display_y_offset = (screen_top - y1) * height_multipler;
        display_height -= (screen_top - y1) * height_multipler;
    }

    let mut display_width = display_width_clocks / dot_clock_divider;
    if display_width <= 0 || display_height <= 0 {
        return (None, frame_size);
    }

    let display_x_start = cmp::max(0, (x1 - SCREEN_LEFT) / dot_clock_divider);
    let display_y_start = cmp::max(0, (y1 - screen_top) * height_multipler);

    // Clamp display width in case of errors caused by dot clock division
    display_width = cmp::min(display_width, frame_width - display_x_start);

    assert!(
        display_y_start + display_height <= frame_height,
        "Vertical display range: {display_y_start} + {display_height} <= {frame_height}"
    );

    (
        Some(FrameCoords {
            frame_x: registers.display_area_x,
            frame_y: registers.display_area_y,
            display_x_offset: display_x_offset as u32,
            display_y_offset: display_y_offset as u32,
            display_x_start: display_x_start as u32,
            display_y_start: display_y_start as u32,
            display_width: display_width as u32,
            display_height: display_height as u32,
        }),
        frame_size,
    )
}

fn create_clear_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let clear_module = device.create_shader_module(wgpu::include_wgsl!("clear.wgsl"));

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: "clear_pipeline".into(),
        layout: None,
        vertex: wgpu::VertexState { module: &clear_module, entry_point: "vs_main", buffers: &[] },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &clear_module,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}

fn get_or_create_frame_texture<'a>(
    device: &wgpu::Device,
    frame_size: FrameSize,
    map: &'a mut HashMap<FrameSize, wgpu::Texture>,
) -> &'a wgpu::Texture {
    map.entry(frame_size).or_insert_with(|| {
        log::info!("Creating PS1 GPU frame texture of size {frame_size}");

        device.create_texture(&wgpu::TextureDescriptor {
            label: "frame_texture".into(),
            size: wgpu::Extent3d {
                width: frame_size.width,
                height: frame_size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        })
    })
}

const RGB_5_TO_8: &[u8; 32] = &[
    0, 8, 16, 25, 33, 41, 49, 58, 66, 74, 82, 90, 99, 107, 115, 123, 132, 140, 148, 156, 165, 173,
    181, 189, 197, 206, 214, 222, 230, 239, 247, 255,
];

fn populate_frame_buffer(
    frame_size: FrameSize,
    frame_coords: FrameCoords,
    color_depth: ColorDepthBits,
    vram: &Vram,
    frame_buffer: &mut FrameBuffer,
) {
    let x_range =
        frame_coords.display_x_start..frame_coords.display_x_start + frame_coords.display_width;
    let y_range =
        frame_coords.display_y_start..frame_coords.display_y_start + frame_coords.display_height;

    for y in 0..frame_size.height {
        if !y_range.contains(&y) {
            frame_buffer[1024 * y as usize..1024 * (y + 1) as usize].fill(Color::BLACK);
            continue;
        }

        let vram_y = ((frame_coords.frame_y + y + frame_coords.display_y_offset)
            .wrapping_sub(frame_coords.display_y_start))
            & 0x1FF;
        let vram_row_addr = (1024 * vram_y) as usize;

        frame_buffer[vram_row_addr..vram_row_addr + frame_coords.display_x_start as usize]
            .fill(Color::BLACK);
        frame_buffer[vram_row_addr
            + (frame_coords.display_x_start + frame_coords.display_width) as usize
            ..vram_row_addr + 1024]
            .fill(Color::BLACK);

        for x in x_range.clone() {
            let frame_buffer_addr = (1024 * y + x) as usize;

            match color_depth {
                ColorDepthBits::Fifteen => {
                    let vram_x = ((frame_coords.frame_x + x + frame_coords.display_x_offset)
                        .wrapping_sub(frame_coords.display_x_start))
                        & 0x3FF;
                    let vram_addr = (1024 * vram_y + vram_x) as usize;
                    let vram_color = vram[vram_addr];

                    let r = RGB_5_TO_8[(vram_color & 0x1F) as usize];
                    let g = RGB_5_TO_8[((vram_color >> 5) & 0x1F) as usize];
                    let b = RGB_5_TO_8[((vram_color >> 10) & 0x1F) as usize];
                    frame_buffer[frame_buffer_addr] = Color::rgb(r, g, b);
                }
                ColorDepthBits::TwentyFour => {
                    let effective_x = (x + frame_coords.display_x_offset)
                        .wrapping_sub(frame_coords.display_x_start)
                        & 0x3FF;
                    let vram_x = frame_coords.frame_x + 3 * effective_x / 2;
                    let first_halfword = vram[vram_row_addr | (vram_x & 0x3FF) as usize];
                    let second_halfword = vram[vram_row_addr | ((vram_x + 1) & 0x3FF) as usize];

                    let color = if effective_x % 2 == 0 {
                        Color::rgb(
                            first_halfword as u8,
                            (first_halfword >> 8) as u8,
                            second_halfword as u8,
                        )
                    } else {
                        Color::rgb(
                            (first_halfword >> 8) as u8,
                            second_halfword as u8,
                            (second_halfword >> 8) as u8,
                        )
                    };
                    frame_buffer[frame_buffer_addr] = color;
                }
            }
        }
    }
}
