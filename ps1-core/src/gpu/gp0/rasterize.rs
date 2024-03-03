#![allow(clippy::many_single_char_names)]

use crate::gpu::gp0::{
    Color, DrawPolygonParameters, DrawSettings, PolygonCommandParameters, SemiTransparencyMode,
    TextureColorDepthBits, TexturePage, Vertex,
};
use crate::gpu::Vram;
use std::cmp;

const DITHER_TABLE: &[[i8; 4]; 4] = &[
    [-4, 0, -3, 1],
    [2, -2, 3, -1],
    [-3, 1, -4, 0],
    [3, -1, 2, -2],
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct VertexFloat {
    x: f64,
    y: f64,
}

impl Vertex {
    fn to_float(self) -> VertexFloat {
        VertexFloat {
            x: self.x.into(),
            y: self.y.into(),
        }
    }
}

impl Color {
    fn from_15_bit(color: u16) -> Self {
        let r = color & 0x1F;
        let g = (color >> 5) & 0x1F;
        let b = (color >> 10) & 0x1F;

        let r = (f64::from(r) * 255.0 / 31.0).round() as u8;
        let g = (f64::from(g) * 255.0 / 31.0).round() as u8;
        let b = (f64::from(b) * 255.0 / 31.0).round() as u8;

        Self { r, g, b }
    }

    fn dither(self, dither_value: i8) -> Self {
        Self {
            r: self.r.saturating_add_signed(dither_value),
            g: self.g.saturating_add_signed(dither_value),
            b: self.b.saturating_add_signed(dither_value),
        }
    }
}

impl SemiTransparencyMode {
    fn apply(self, back: Color, front: Color) -> Color {
        match self {
            Self::Average => apply_semi_transparency(back, front, |b, f| {
                ((u16::from(b) + u16::from(f)) / 2) as u8
            }),
            Self::Add => apply_semi_transparency(back, front, |b, f| {
                cmp::min(255, u16::from(b) + u16::from(f)) as u8
            }),
            Self::Subtract => apply_semi_transparency(back, front, |b, f| {
                cmp::max(0, i16::from(b) - i16::from(f)) as u8
            }),
            Self::AddQuarter => apply_semi_transparency(back, front, |b, f| {
                cmp::min(255, u16::from(b) + u16::from(f / 4)) as u8
            }),
        }
    }
}

fn apply_semi_transparency<F>(back: Color, front: Color, op: F) -> Color
where
    F: Fn(u8, u8) -> u8,
{
    Color {
        r: op(back.r, front.r),
        g: op(back.g, front.g),
        b: op(back.b, front.b),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Shading {
    Flat(Color),
    Gouraud(Color, Color, Color),
}

#[derive(Debug, Clone)]
pub struct TextureParameters {
    pub texpage: TexturePage,
    pub clut_x: u16,
    pub clut_y: u16,
    pub u: [u8; 3],
    pub v: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureMode {
    None,
    Raw,
    Modulated,
}

impl TextureMode {
    pub fn from_command_params(params: PolygonCommandParameters) -> Self {
        match (params.textured, params.raw_texture) {
            (false, _) => Self::None,
            (true, false) => Self::Modulated,
            (true, true) => Self::Raw,
        }
    }
}

pub fn triangle(
    DrawPolygonParameters {
        vertices: v,
        shading,
        semi_transparent,
        global_semi_transparency_mode,
        texture_params,
        texture_mode,
    }: DrawPolygonParameters,
    draw_settings: &DrawSettings,
    vram: &mut Vram,
) {
    // Determine if the vertices are in clockwise order; if not, swap the first 2
    if cross_product_z(v[0].to_float(), v[1].to_float(), v[2].to_float()) < 0.0 {
        triangle_swapped_vertices(
            v,
            shading,
            semi_transparent,
            global_semi_transparency_mode,
            texture_params,
            texture_mode,
            draw_settings,
            vram,
        );
        return;
    }

    let (draw_min_x, draw_min_y) = draw_settings.draw_area_top_left;
    let (draw_max_x, draw_max_y) = draw_settings.draw_area_bottom_right;

    if draw_min_x > draw_max_x || draw_min_y > draw_max_y {
        // Invalid drawing area; do nothing
        return;
    }

    let (x_offset, y_offset) = draw_settings.draw_offset;

    // Apply drawing offset to vertices
    let v = v.map(|vertex| Vertex {
        x: vertex.x + x_offset,
        y: vertex.y + y_offset,
    });

    log::trace!("Triangle vertices: {v:?}");
    log::trace!("Bounding box: ({draw_min_x}, {draw_min_y}) to ({draw_max_x}, {draw_max_y}");

    // Compute bounding box, clamped to display area
    let min_x =
        cmp::min(v[0].x, cmp::min(v[1].x, v[2].x)).clamp(draw_min_x as i32, draw_max_x as i32);
    let max_x =
        cmp::max(v[0].x, cmp::max(v[1].x, v[2].x)).clamp(draw_min_x as i32, draw_max_x as i32);
    let min_y =
        cmp::min(v[0].y, cmp::min(v[1].y, v[2].y)).clamp(draw_min_y as i32, draw_max_y as i32);
    let max_y =
        cmp::max(v[0].y, cmp::max(v[1].y, v[2].y)).clamp(draw_min_y as i32, draw_max_y as i32);

    // Operate in floating-point from here on
    let v = v.map(Vertex::to_float);

    // Iterate over every pixel in the bounding box to determine which ones to rasterize
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            rasterize_pixel(
                px,
                py,
                v,
                shading,
                semi_transparent,
                global_semi_transparency_mode,
                &texture_params,
                texture_mode,
                draw_settings.dithering_enabled,
                vram,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn triangle_swapped_vertices(
    v: [Vertex; 3],
    shading: Shading,
    semi_transparent: bool,
    global_semi_transparency_mode: SemiTransparencyMode,
    texture_params: TextureParameters,
    texture_mode: TextureMode,
    draw_settings: &DrawSettings,
    vram: &mut Vram,
) {
    let vertices = [v[1], v[0], v[2]];
    let texture_u = [
        texture_params.u[1],
        texture_params.u[0],
        texture_params.u[2],
    ];
    let texture_v = [
        texture_params.v[1],
        texture_params.v[0],
        texture_params.v[2],
    ];
    let shading = match shading {
        Shading::Flat(color) => Shading::Flat(color),
        Shading::Gouraud(color0, color1, color2) => Shading::Gouraud(color1, color0, color2),
    };

    triangle(
        DrawPolygonParameters {
            vertices,
            shading,
            semi_transparent,
            global_semi_transparency_mode,
            texture_params: TextureParameters {
                u: texture_u,
                v: texture_v,
                ..texture_params
            },
            texture_mode,
        },
        draw_settings,
        vram,
    );
}

#[allow(clippy::too_many_arguments)]
fn rasterize_pixel(
    px: i32,
    py: i32,
    v: [VertexFloat; 3],
    shading: Shading,
    semi_transparent: bool,
    global_semi_transparency_mode: SemiTransparencyMode,
    texture_params: &TextureParameters,
    texture_mode: TextureMode,
    dithering: bool,
    vram: &mut Vram,
) {
    // The sampling point is in the center of the pixel, so add 0.5 to both coordinates
    let p = VertexFloat {
        x: f64::from(px) + 0.5,
        y: f64::from(py) + 0.5,
    };

    // A given point is contained within the triangle if the cross-product of v0->p and
    // v0->v1 is non-negative for each edge v0->v1
    for edge in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
        let cpz = cross_product_z(edge.0, edge.1, p);
        if cpz < 0.0 {
            return;
        }

        // If the cross product is 0, the point is collinear with these two vertices.
        // The PS1 GPU does not draw edges on the bottom of the triangle when this happens,
        // nor does it draw a vertical right edge
        if cpz.abs() < 1e-3 {
            // Since the vertices are clockwise, decreasing X means this edge is on the
            // bottom of the triangle.
            if edge.1.x < edge.0.x {
                return;
            }

            // If the X values are equal and Y is increasing, this is a vertical right edge
            if (edge.1.x - edge.0.x) < 1e-3 && edge.1.y > edge.0.y {
                return;
            }
        }
    }

    let shading_color = match shading {
        Shading::Flat(color) => color,
        Shading::Gouraud(color0, color1, color2) => {
            apply_gouraud_shading(p, v, [color0, color1, color2])
        }
    };

    let (textured_color, mask_bit) = match texture_mode {
        TextureMode::None => (shading_color, false),
        TextureMode::Raw | TextureMode::Modulated => {
            let (tex_u, tex_v) =
                interpolate_uv_coordinates(p, v, texture_params.u, texture_params.v);

            let texture_pixel = sample_texture(
                vram,
                &texture_params.texpage,
                texture_params.clut_x.into(),
                texture_params.clut_y.into(),
                tex_u.into(),
                tex_v.into(),
            );
            if texture_pixel == 0x0000 {
                // Pixel values of $0000 are fully transparent and are not written to VRAM
                return;
            }

            // TODO semi-transparency / mask bit

            let raw_texture_color = Color::from_15_bit(texture_pixel);

            let texture_color = match texture_mode {
                TextureMode::Raw => raw_texture_color,
                TextureMode::Modulated => {
                    // Apply modulation: multiply the texture color by the shading color / 128
                    let r = (f64::from(raw_texture_color.r) * f64::from(shading_color.r) / 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let g = (f64::from(raw_texture_color.g) * f64::from(shading_color.g) / 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let b = (f64::from(raw_texture_color.b) * f64::from(shading_color.b) / 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;

                    Color { r, g, b }
                }
                TextureMode::None => unreachable!("nested match expressions"),
            };

            (texture_color, texture_pixel & 0x8000 != 0)
        }
    };

    let vram_addr = (2048 * py + 2 * px) as usize;
    let masked_color = if semi_transparent && (texture_mode == TextureMode::None || mask_bit) {
        let existing_pixel = u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]);
        let existing_color = Color::from_15_bit(existing_pixel);

        let semi_transparency_mode = match texture_mode {
            TextureMode::None => global_semi_transparency_mode,
            TextureMode::Raw | TextureMode::Modulated => {
                texture_params.texpage.semi_transparency_mode
            }
        };

        semi_transparency_mode.apply(existing_color, textured_color)
    } else {
        textured_color
    };

    // Dithering is applied if the dither flag is set and either Gouraud shading or texture
    // modulation is used
    let dithered_color = if dithering
        && (matches!(shading, Shading::Gouraud(..)) || texture_mode == TextureMode::Modulated)
    {
        let dither_value = DITHER_TABLE[(py & 3) as usize][(px & 3) as usize];
        masked_color.dither(dither_value)
    } else {
        masked_color
    };

    let [color_lsb, color_msb] = dithered_color.truncate_to_15_bit().to_le_bytes();
    vram[vram_addr] = color_lsb;
    vram[vram_addr + 1] = color_msb | (u8::from(mask_bit) << 7);
}

fn apply_gouraud_shading(p: VertexFloat, v: [VertexFloat; 3], colors: [Color; 3]) -> Color {
    let rf = colors.map(|color| f64::from(color.r));
    let gf = colors.map(|color| f64::from(color.g));
    let bf = colors.map(|color| f64::from(color.b));

    // Interpolate between the color of each vertex using Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, v[0], v[1], v[2]);
    let r = alpha * rf[0] + beta * rf[1] + gamma * rf[2];
    let g = alpha * gf[0] + beta * gf[1] + gamma * gf[2];
    let b = alpha * bf[0] + beta * bf[1] + gamma * bf[2];

    Color {
        r: r.round() as u8,
        g: g.round() as u8,
        b: b.round() as u8,
    }
}

fn interpolate_uv_coordinates(
    p: VertexFloat,
    vertices: [VertexFloat; 3],
    u: [u8; 3],
    v: [u8; 3],
) -> (u8, u8) {
    let vertices = vertices.map(|vertex| VertexFloat {
        x: vertex.x + 0.5,
        y: vertex.y + 0.5,
    });

    let uf = u.map(f64::from);
    let vf = v.map(f64::from);

    // Similar to Gouraud shading, interpolate the U/V coordinates between vertices by using
    // Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, vertices[0], vertices[1], vertices[2]);

    let u = alpha * uf[0] + beta * uf[1] + gamma * uf[2];
    let v = alpha * vf[0] + beta * vf[1] + gamma * vf[2];

    // Floor rather than round because rounding looks smoother than what the PS1 GPU outputs
    let u = u.round() as u8;
    let v = v.round() as u8;

    (u, v)
}

// Z component of the cross product between v0->v1 and v0->v2
fn cross_product_z(v0: VertexFloat, v1: VertexFloat, v2: VertexFloat) -> f64 {
    (v1.x - v0.x) * (v2.y - v0.y) - (v1.y - v0.y) * (v2.x - v0.x)
}

fn compute_affine_coordinates(
    p: VertexFloat,
    v1: VertexFloat,
    v2: VertexFloat,
    v3: VertexFloat,
) -> (f64, f64, f64) {
    let determinant = (v1.x - v3.x) * (v2.y - v3.y) - (v2.x - v3.x) * (v1.y - v3.y);
    if determinant.abs() < 1e-6 {
        // TODO what to do when points are collinear?
        let one_third = 1.0 / 3.0;
        return (one_third, one_third, one_third);
    }

    let alpha = ((p.x - v3.x) * (v2.y - v3.y) - (p.y - v3.y) * (v2.x - v3.x)) / determinant;
    let beta = ((p.x - v3.x) * (v3.y - v1.y) - (p.y - v3.y) * (v3.x - v1.x)) / determinant;
    let gamma = 1.0 - alpha - beta;

    (alpha, beta, gamma)
}

fn sample_texture(
    vram: &Vram,
    texpage: &TexturePage,
    clut_x: u32,
    clut_y: u32,
    u: u32,
    v: u32,
) -> u16 {
    // TODO texture window mask/offset

    let y = texpage.y_base + v;

    match texpage.color_depth {
        TextureColorDepthBits::Four => {
            let vram_addr = 2048 * y + 2 * 64 * texpage.x_base + u / 2;
            let shift = 4 * (u % 2);
            let clut_index: u32 = ((vram[vram_addr as usize] >> shift) & 0xF).into();

            let clut_base_addr = 2048 * clut_y + 2 * 16 * clut_x;
            let clut_addr = clut_base_addr + 2 * clut_index;

            u16::from_le_bytes([vram[clut_addr as usize], vram[(clut_addr + 1) as usize]])
        }
        TextureColorDepthBits::Eight => {
            let vram_x_bytes = (2 * 64 * texpage.x_base + u) & 0x7FF;
            let vram_addr = 2048 * y + vram_x_bytes;
            let clut_index: u32 = vram[vram_addr as usize].into();

            let clut_base_addr = 2048 * clut_y + 2 * 16 * clut_x;
            let clut_addr = clut_base_addr + 2 * clut_index;

            u16::from_le_bytes([vram[clut_addr as usize], vram[(clut_addr + 1) as usize]])
        }
        TextureColorDepthBits::Fifteen => {
            let vram_x_pixels = (64 * texpage.x_base + u) & 0x3FF;
            let vram_addr = (2048 * y + 2 * vram_x_pixels) as usize;
            u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]])
        }
    }
}

pub fn fill(fill_x: u32, fill_y: u32, width: u32, height: u32, color: Color, vram: &mut Vram) {
    let fill_x = fill_x & 0x3F0;
    let fill_y = fill_y & 0x1FF;
    let width = ((width & 0x3FF) + 0xF) & !0xF;
    let height = height & 0x1FF;

    let [color_lsb, color_msb] = color.truncate_to_15_bit().to_le_bytes();

    for y_offset in 0..height {
        for x_offset in 0..width {
            let x = (fill_x + x_offset) & 0x3FF;
            let y = (fill_y + y_offset) & 0x1FF;

            let vram_addr = (2048 * y + 2 * x) as usize;
            vram[vram_addr] = color_lsb;
            vram[vram_addr + 1] = color_msb;
        }
    }
}
