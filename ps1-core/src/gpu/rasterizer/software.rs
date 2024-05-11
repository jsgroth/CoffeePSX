use crate::api::{ColorDepthBits, DisplayConfig};
use crate::gpu::rasterizer::{CpuVramBlitArgs, VramVramBlitArgs};
use crate::gpu::registers::{Registers, VerticalResolution};
use crate::gpu::{Color, VideoMode, VramArray, WgpuResources};
use bytemuck::{Pod, Zeroable};
use std::cmp;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use wgpu::{CommandBuffer, PipelineCompilationOptions};

struct ScreenSize {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
    v_overscan_rows: i32,
}

impl ScreenSize {
    const NTSC: Self = Self { left: 0x260, right: 0xC60, top: 16, bottom: 256, v_overscan_rows: 8 };

    const PAL: Self = Self { left: 0x274, right: 0xC74, top: 20, bottom: 308, v_overscan_rows: 10 };
}

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
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RgbaColor {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl RgbaColor {
    const BLACK: Self = Self::rgb(0, 0, 0);

    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

const FRAME_BUFFER_LEN: usize =
    (1024 * 2 * (ScreenSize::PAL.bottom - ScreenSize::PAL.top)) as usize;

type FrameBuffer = [RgbaColor; FRAME_BUFFER_LEN];

#[derive(Debug)]
pub struct SoftwareRenderer {
    frame_buffer: Box<FrameBuffer>,
    frame_textures: HashMap<FrameSize, wgpu::Texture>,
    clear_pipeline: wgpu::RenderPipeline,
}

impl SoftwareRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let clear_pipeline = create_clear_pipeline(device);

        Self {
            frame_buffer: vec![RgbaColor::BLACK; FRAME_BUFFER_LEN]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            frame_textures: HashMap::new(),
            clear_pipeline,
        }
    }

    pub fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &mut WgpuResources,
        vram: &VramArray,
    ) -> &wgpu::Texture {
        if wgpu_resources.display_config.dump_vram {
            return self.write_frame(
                &wgpu_resources.device,
                &wgpu_resources.queue,
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
                vram,
            );
        }

        let (frame_coords, frame_size) =
            compute_frame_location(registers, wgpu_resources.display_config);
        let Some(frame_coords) = frame_coords else {
            return self.clear_frame(
                &wgpu_resources.device,
                &mut wgpu_resources.queued_command_buffers,
                frame_size,
            );
        };

        log::debug!(
            "Computed frame coords {frame_coords:?} and frame_size {frame_size:?} from video_mode={}, X1={}, X2={}, Y1={}, Y2={}, dot_clock_divider={}, v_resolution={:?}",
            registers.video_mode,
            registers.x_display_range.0,
            registers.x_display_range.1,
            registers.y_display_range.0,
            registers.y_display_range.1,
            registers.dot_clock_divider(),
            registers.v_resolution
        );

        if !registers.display_enabled {
            return self.clear_frame(
                &wgpu_resources.device,
                &mut wgpu_resources.queued_command_buffers,
                frame_size,
            );
        }

        return self.write_frame(
            &wgpu_resources.device,
            &wgpu_resources.queue,
            frame_size,
            frame_coords,
            registers.display_area_color_depth,
            vram,
        );
    }

    fn clear_frame(
        &mut self,
        device: &wgpu::Device,
        command_buffers: &mut Vec<CommandBuffer>,
        frame_size: FrameSize,
    ) -> &wgpu::Texture {
        let texture = get_or_create_frame_texture(device, frame_size, &mut self.frame_textures);
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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

        command_buffers.push(encoder.finish());

        texture
    }

    fn write_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_size: FrameSize,
        frame_coords: FrameCoords,
        color_depth: ColorDepthBits,
        vram: &VramArray,
    ) -> &wgpu::Texture {
        populate_frame_buffer(frame_size, frame_coords, color_depth, vram, &mut self.frame_buffer);

        let frame_texture =
            get_or_create_frame_texture(device, frame_size, &mut self.frame_textures);

        queue.write_texture(
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

fn compute_frame_location(
    registers: &Registers,
    display_config: DisplayConfig,
) -> (Option<FrameCoords>, FrameSize) {
    let crop_v_overscan = display_config.crop_vertical_overscan;
    let screen_size = match registers.video_mode {
        VideoMode::Ntsc => ScreenSize::NTSC,
        VideoMode::Pal => ScreenSize::PAL,
    };

    let screen_top = if crop_v_overscan {
        screen_size.top + screen_size.v_overscan_rows
    } else {
        screen_size.top
    };
    let screen_bottom = if crop_v_overscan {
        screen_size.bottom - screen_size.v_overscan_rows
    } else {
        screen_size.bottom
    };

    let dot_clock_divider: i32 = registers.dot_clock_divider().into();
    let frame_width = (screen_size.right - screen_size.left) / dot_clock_divider;

    let height_multipler =
        if registers.interlaced && registers.v_resolution == VerticalResolution::Double {
            2
        } else {
            1
        };
    let frame_height = (if crop_v_overscan {
        screen_size.bottom - screen_size.top - 2 * screen_size.v_overscan_rows
    } else {
        screen_size.bottom - screen_size.top
    }) * height_multipler;

    let frame_size = FrameSize { width: frame_width as u32, height: frame_height as u32 };

    let x1 = registers.x_display_range.0 as i32;
    let x2 = cmp::min(screen_size.right, registers.x_display_range.1 as i32);
    let y1 = registers.y_display_range.0 as i32;
    let y2 = cmp::min(screen_bottom, registers.y_display_range.1 as i32);

    let mut display_width_clocks = x2 - x1;
    let mut display_height = (y2 - y1) * height_multipler;

    let mut display_x_offset = 0;
    if x1 < screen_size.left {
        display_x_offset = (screen_size.left - x1) / dot_clock_divider;
        display_width_clocks -= screen_size.left - x1;
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

    let display_x_start = cmp::max(0, (x1 - screen_size.left) / dot_clock_divider);
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
        vertex: wgpu::VertexState {
            module: &clear_module,
            entry_point: "vs_main",
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
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
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
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
    vram: &VramArray,
    frame_buffer: &mut FrameBuffer,
) {
    let x_range =
        frame_coords.display_x_start..frame_coords.display_x_start + frame_coords.display_width;
    let y_range =
        frame_coords.display_y_start..frame_coords.display_y_start + frame_coords.display_height;

    for y in 0..frame_size.height {
        if !y_range.contains(&y) {
            frame_buffer[1024 * y as usize..1024 * (y + 1) as usize].fill(RgbaColor::BLACK);
            continue;
        }

        let vram_y = ((frame_coords.frame_y + y + frame_coords.display_y_offset)
            .wrapping_sub(frame_coords.display_y_start))
            & 0x1FF;
        let vram_row_addr = (1024 * vram_y) as usize;

        // Fill pixels outside of the horizontal display range with solid black
        let fb_row_addr = 1024 * y as usize;
        frame_buffer[fb_row_addr..fb_row_addr + x_range.start as usize].fill(RgbaColor::BLACK);
        frame_buffer[fb_row_addr + x_range.end as usize..fb_row_addr + frame_size.width as usize]
            .fill(RgbaColor::BLACK);

        for x in x_range.clone() {
            let frame_buffer_addr = fb_row_addr + x as usize;

            match color_depth {
                ColorDepthBits::Fifteen => {
                    let vram_x = ((frame_coords.frame_x + x + frame_coords.display_x_offset)
                        .wrapping_sub(frame_coords.display_x_start))
                        & 0x3FF;
                    let vram_addr = vram_row_addr | (vram_x as usize);
                    let vram_color = vram[vram_addr];

                    let r = RGB_5_TO_8[(vram_color & 0x1F) as usize];
                    let g = RGB_5_TO_8[((vram_color >> 5) & 0x1F) as usize];
                    let b = RGB_5_TO_8[((vram_color >> 10) & 0x1F) as usize];
                    frame_buffer[frame_buffer_addr] = RgbaColor::rgb(r, g, b);
                }
                ColorDepthBits::TwentyFour => {
                    let effective_x = (x + frame_coords.display_x_offset)
                        .wrapping_sub(frame_coords.display_x_start)
                        & 0x3FF;
                    let vram_x = frame_coords.frame_x + 3 * effective_x / 2;
                    let first_halfword = vram[vram_row_addr | (vram_x & 0x3FF) as usize];
                    let second_halfword = vram[vram_row_addr | ((vram_x + 1) & 0x3FF) as usize];

                    let color = if effective_x % 2 == 0 {
                        RgbaColor::rgb(
                            first_halfword as u8,
                            (first_halfword >> 8) as u8,
                            second_halfword as u8,
                        )
                    } else {
                        RgbaColor::rgb(
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

pub fn vram_fill(vram: &mut VramArray, x: u32, y: u32, width: u32, height: u32, color: Color) {
    let fill_x = x & 0x3F0;
    let fill_y = y & 0x1FF;
    let width = ((width & 0x3FF) + 0xF) & !0xF;
    let height = height & 0x1FF;

    let color = color.truncate_to_15_bit();

    for y_offset in 0..height {
        for x_offset in 0..width {
            let x = (fill_x + x_offset) & 0x3FF;
            let y = (fill_y + y_offset) & 0x1FF;

            let vram_addr = (1024 * y + x) as usize;
            vram[vram_addr] = color;
        }
    }
}

pub fn cpu_to_vram_blit(
    vram: &mut VramArray,
    CpuVramBlitArgs { x, y, width, height, force_mask_bit, check_mask_bit }: CpuVramBlitArgs,
    data: &[u16],
) {
    let forced_mask_bit = u16::from(force_mask_bit) << 15;

    let mut row = 0;
    let mut col = 0;

    for &halfword in data {
        let vram_x = (x + col) & 0x3FF;
        let vram_y = (y + row) & 0x1FF;
        let vram_addr = (1024 * vram_y + vram_x) as usize;

        if !check_mask_bit || vram[vram_addr] & 0x8000 == 0 {
            vram[vram_addr] = halfword | forced_mask_bit;
        }

        col += 1;
        if col == width {
            col = 0;
            row += 1;

            if row == height {
                return;
            }
        }
    }
}

pub fn vram_to_cpu_blit(
    vram: &VramArray,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    out: &mut Vec<u16>,
) {
    for row in 0..height {
        let vram_y = (y + row) & 0x1FF;
        for col in 0..width {
            let vram_x = (x + col) & 0x3FF;
            let vram_addr = (1024 * vram_y + vram_x) as usize;
            out.push(vram[vram_addr]);
        }
    }
}

pub fn vram_to_vram_blit(vram: &mut VramArray, args: VramVramBlitArgs) {
    let forced_mask_bit = u16::from(args.force_mask_bit) << 15;

    let mut source_y = args.source_y;
    let mut dest_y = args.dest_y;

    for _ in 0..args.height {
        let mut source_x = args.source_x;
        let mut dest_x = args.dest_x;

        for _ in 0..args.width {
            let source_addr = (1024 * source_y + source_x) as usize;
            let dest_addr = (1024 * dest_y + dest_x) as usize;

            if !args.check_mask_bit || vram[dest_addr] & 0x8000 == 0 {
                vram[dest_addr] = vram[source_addr] | forced_mask_bit;
            }

            source_x = source_x.wrapping_add(1) & 0x3FF;
            dest_x = dest_x.wrapping_add(1) & 0x3FF;
        }

        source_y = source_y.wrapping_add(1) & 0x1FF;
        dest_y = dest_y.wrapping_add(1) & 0x1FF;
    }
}
