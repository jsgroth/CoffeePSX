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
            x: self.x as f64,
            y: self.y as f64,
        }
    }
}

impl Color {
    fn from_15_bit(color: u16) -> Self {
        let r = color & 0x1F;
        let g = (color >> 5) & 0x1F;
        let b = (color >> 10) & 0x1F;

        let r = (r as f64 * 255.0 / 31.0).round() as u8;
        let g = (g as f64 * 255.0 / 31.0).round() as u8;
        let b = (b as f64 * 255.0 / 31.0).round() as u8;

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
            Self::Average => Color {
                r: ((u16::from(back.r) + u16::from(front.r)) / 2) as u8,
                g: ((u16::from(back.g) + u16::from(front.g)) / 2) as u8,
                b: ((u16::from(back.b) + u16::from(front.b)) / 2) as u8,
            },
            Self::Add => Color {
                r: (u16::from(back.r) + u16::from(front.r)).clamp(0, 255) as u8,
                g: (u16::from(back.g) + u16::from(front.g)).clamp(0, 255) as u8,
                b: (u16::from(back.b) + u16::from(front.b)).clamp(0, 255) as u8,
            },
            Self::Subtract => Color {
                r: (i16::from(back.r) - i16::from(front.r)).clamp(0, 255) as u8,
                g: (i16::from(back.g) - i16::from(front.g)).clamp(0, 255) as u8,
                b: (i16::from(back.b) - i16::from(front.b)).clamp(0, 255) as u8,
            },
            Self::AddQuarter => Color {
                r: (u16::from(back.r) + u16::from(front.r / 4)).clamp(0, 255) as u8,
                g: (u16::from(back.g) + u16::from(front.g / 4)).clamp(0, 255) as u8,
                b: (u16::from(back.b) + u16::from(front.b / 4)).clamp(0, 255) as u8,
            },
        }
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
    let v = v.map(|vertex| vertex.to_float());

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
        x: px as f64 + 0.5,
        y: py as f64 + 0.5,
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

            let r = texture_pixel & 0x1F;
            let g = (texture_pixel >> 5) & 0x1F;
            let b = (texture_pixel >> 10) & 0x1F;

            let texture_color = match texture_mode {
                TextureMode::Raw => Color {
                    r: (r as f64 * 255.0 / 31.0).round() as u8,
                    g: (g as f64 * 255.0 / 31.0).round() as u8,
                    b: (b as f64 * 255.0 / 31.0).round() as u8,
                },
                TextureMode::Modulated => {
                    // Apply modulation: multiply the texture color by the shading color / 128
                    let r = (r as f64 * 255.0 / 31.0 * shading_color.r as f64 / 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let g = (g as f64 * 255.0 / 31.0 * shading_color.g as f64 / 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let b = (b as f64 * 255.0 / 31.0 * shading_color.b as f64 / 128.0)
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
    // Interpolate between the color of each vertex using Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, v[0], v[1], v[2]);
    let r = alpha * colors[0].r as f64 + beta * colors[1].r as f64 + gamma * colors[2].r as f64;
    let g = alpha * colors[0].g as f64 + beta * colors[1].g as f64 + gamma * colors[2].g as f64;
    let b = alpha * colors[0].b as f64 + beta * colors[1].b as f64 + gamma * colors[2].b as f64;

    let color = Color {
        r: r.round() as u8,
        g: g.round() as u8,
        b: b.round() as u8,
    };

    color
}

fn interpolate_uv_coordinates(
    p: VertexFloat,
    vertices: [VertexFloat; 3],
    u: [u8; 3],
    v: [u8; 3],
) -> (u8, u8) {
    // Similar to Gouraud shading, interpolate the U/V coordinates between vertices by using
    // Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, vertices[0], vertices[1], vertices[2]);

    let u = alpha * u[0] as f64 + beta * u[1] as f64 + gamma * u[2] as f64;

    let v = alpha * v[0] as f64 + beta * v[1] as f64 + gamma * v[2] as f64;

    // Floor rather than round because rounding looks smoother than what the PS1 GPU outputs
    let u = u.floor() as u8;
    let v = v.floor() as u8;

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

    let y = texpage.y_base + u32::from(v);

    match texpage.color_depth {
        TextureColorDepthBits::Four => {
            let vram_addr = 2048 * y + 2 * 64 * texpage.x_base + u32::from(u) / 2;
            let shift = 4 * (u % 2);
            let clut_index: u32 = ((vram[vram_addr as usize] >> shift) & 0xF).into();

            let clut_base_addr = 2048 * clut_y + 2 * 16 * clut_x;
            let clut_addr = clut_base_addr + 2 * clut_index;

            u16::from_le_bytes([vram[clut_addr as usize], vram[(clut_addr + 1) as usize]])
        }
        _ => todo!("color depth {:?}", texpage.color_depth),
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
