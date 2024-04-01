//! A (quite slow) software rasterizer. Needs to be rewritten

#![allow(clippy::many_single_char_names)]

use crate::gpu::gp0::{
    Color, DrawSettings, PolygonCommandParameters, RectangleCommandParameters,
    SemiTransparencyMode, TextureColorDepthBits, TexturePage, TextureWindow, Vertex,
};
use crate::gpu::Vram;
use std::cmp;

const DITHER_TABLE: &[[i8; 4]; 4] =
    &[[-4, 0, -3, 1], [2, -2, 3, -1], [-3, 1, -4, 0], [3, -1, 2, -2]];

#[derive(Debug, Clone, Copy, PartialEq)]
struct VertexFloat {
    x: f64,
    y: f64,
}

impl Vertex {
    fn to_float(self) -> VertexFloat {
        VertexFloat { x: self.x.into(), y: self.y.into() }
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
    Color { r: op(back.r, front.r), g: op(back.g, front.g), b: op(back.b, front.b) }
}

#[derive(Debug, Clone, Copy)]
pub enum LineShading {
    Flat(Color),
    Gouraud(Color, Color),
}

#[derive(Debug, Clone, Copy)]
pub enum PolygonShading {
    Flat(Color),
    Gouraud(Color, Color, Color),
}

#[derive(Debug, Clone)]
pub struct DrawLineParameters {
    pub vertices: [Vertex; 2],
    pub shading: LineShading,
    pub semi_transparent: bool,
}

pub fn line(
    DrawLineParameters { vertices, shading, semi_transparent }: DrawLineParameters,
    draw_settings: &DrawSettings,
    global_texpage: TexturePage,
    vram: &mut Vram,
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
            LineShading::Flat(color) | LineShading::Gouraud(color, _) => color,
        };
        draw_line_pixel(
            vertices[0],
            color,
            semi_transparent,
            global_texpage.semi_transparency_mode,
            draw_settings,
            vram,
        );
        return;
    }

    let (r_diff, g_diff, b_diff) = match shading {
        LineShading::Flat(_) => (0, 0, 0),
        LineShading::Gouraud(color0, color1) => (
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
        LineShading::Flat(color) | LineShading::Gouraud(color, _) => color,
    };
    let mut r = f64::from(first_color.r);
    let mut g = f64::from(first_color.g);
    let mut b = f64::from(first_color.b);

    let mut x = f64::from(vertices[0].x);
    let mut y = f64::from(vertices[0].y);
    while x.round() as i32 != vertices[1].x || y.round() as i32 != vertices[1].y {
        let vertex = Vertex { x: x.round() as i32, y: y.round() as i32 };
        let color = Color { r: r.round() as u8, g: g.round() as u8, b: b.round() as u8 };
        draw_line_pixel(
            vertex,
            color,
            semi_transparent,
            global_texpage.semi_transparency_mode,
            draw_settings,
            vram,
        );

        x += x_step;
        y += y_step;
        r += r_step;
        g += g_step;
        b += b_step;
    }

    // Draw the last pixel
    let color = Color { r: r.round() as u8, g: g.round() as u8, b: b.round() as u8 };
    draw_line_pixel(
        vertices[1],
        color,
        semi_transparent,
        global_texpage.semi_transparency_mode,
        draw_settings,
        vram,
    );
}

fn vertices_valid(v0: Vertex, v1: Vertex) -> bool {
    // The GPU will not render any lines or polygons where the distance between any two vertices is
    // larger than 1023 horizontally or 511 vertically
    (v0.x - v1.x).abs() < 1024 && (v0.y - v1.y).abs() < 512
}

fn draw_line_pixel(
    v: Vertex,
    raw_color: Color,
    semi_transparency: bool,
    semi_transparency_mode: SemiTransparencyMode,
    draw_settings: &DrawSettings,
    vram: &mut Vram,
) {
    if !draw_settings.drawing_area_contains_vertex(v) {
        return;
    }

    let vram_addr = (2048 * v.y + 2 * v.x) as usize;
    if draw_settings.check_mask_bit && vram[vram_addr + 1] & 0x80 != 0 {
        return;
    }

    let color = if semi_transparency {
        let existing_color =
            Color::from_15_bit(u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]));
        semi_transparency_mode.apply(existing_color, raw_color)
    } else {
        raw_color
    };

    let dithered_color = if draw_settings.dithering_enabled {
        let dither_value = DITHER_TABLE[(v.y & 3) as usize][(v.x & 3) as usize];
        color.dither(dither_value)
    } else {
        color
    };

    let [color_lsb, color_msb] = dithered_color.truncate_to_15_bit().to_le_bytes();
    vram[vram_addr] = color_lsb;
    vram[vram_addr + 1] = color_msb | (u8::from(draw_settings.force_mask_bit) << 7);
}

#[derive(Debug, Clone)]
pub struct PolygonTextureParameters {
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
    pub fn from_polygon_params(params: PolygonCommandParameters) -> Self {
        Self::from_flags(params.textured, params.raw_texture)
    }

    pub fn from_rectangle_params(params: RectangleCommandParameters) -> Self {
        Self::from_flags(params.textured, params.raw_texture)
    }

    fn from_flags(textured: bool, raw_texture: bool) -> Self {
        match (textured, raw_texture) {
            (false, _) => Self::None,
            (true, false) => Self::Modulated,
            (true, true) => Self::Raw,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawPolygonParameters {
    pub vertices: [Vertex; 3],
    pub shading: PolygonShading,
    pub semi_transparent: bool,
    pub texture_params: PolygonTextureParameters,
    pub texture_mode: TextureMode,
}

pub fn triangle(
    DrawPolygonParameters {
        vertices: v,
        shading,
        semi_transparent,
        texture_params,
        texture_mode,
    }: DrawPolygonParameters,
    draw_settings: &DrawSettings,
    global_texpage: &TexturePage,
    texture_window: TextureWindow,
    vram: &mut Vram,
) {
    if !vertices_valid(v[0], v[1]) || !vertices_valid(v[1], v[2]) || !vertices_valid(v[0], v[2]) {
        return;
    }

    // Determine if the vertices are in clockwise order; if not, swap the first 2
    if cross_product_z(v[0].to_float(), v[1].to_float(), v[2].to_float()) < 0.0 {
        triangle_swapped_vertices(
            v,
            shading,
            semi_transparent,
            texture_params,
            texture_mode,
            draw_settings,
            global_texpage,
            texture_window,
            vram,
        );
        return;
    }

    if !draw_settings.is_drawing_area_valid() {
        return;
    }

    let (draw_min_x, draw_min_y) = draw_settings.draw_area_top_left;
    let (draw_max_x, draw_max_y) = draw_settings.draw_area_bottom_right;

    let (x_offset, y_offset) = draw_settings.draw_offset;

    // Apply drawing offset to vertices
    let v = v.map(|vertex| Vertex { x: vertex.x + x_offset, y: vertex.y + y_offset });

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
                vram,
                RasterizePixelArgs {
                    v,
                    shading,
                    semi_transparent,
                    global_semi_transparency_mode: global_texpage.semi_transparency_mode,
                    texture_params: &texture_params,
                    texture_window,
                    texture_mode,
                    dithering: draw_settings.dithering_enabled,
                    force_mask_bit: draw_settings.force_mask_bit,
                    check_mask_bit: draw_settings.check_mask_bit,
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn triangle_swapped_vertices(
    v: [Vertex; 3],
    shading: PolygonShading,
    semi_transparent: bool,
    texture_params: PolygonTextureParameters,
    texture_mode: TextureMode,
    draw_settings: &DrawSettings,
    global_texpage: &TexturePage,
    texture_window: TextureWindow,
    vram: &mut Vram,
) {
    let vertices = [v[1], v[0], v[2]];
    let texture_u = [texture_params.u[1], texture_params.u[0], texture_params.u[2]];
    let texture_v = [texture_params.v[1], texture_params.v[0], texture_params.v[2]];
    let shading = match shading {
        PolygonShading::Flat(color) => PolygonShading::Flat(color),
        PolygonShading::Gouraud(color0, color1, color2) => {
            PolygonShading::Gouraud(color1, color0, color2)
        }
    };

    triangle(
        DrawPolygonParameters {
            vertices,
            shading,
            semi_transparent,
            texture_params: PolygonTextureParameters {
                u: texture_u,
                v: texture_v,
                ..texture_params
            },
            texture_mode,
        },
        draw_settings,
        global_texpage,
        texture_window,
        vram,
    );
}

struct RasterizePixelArgs<'a> {
    v: [VertexFloat; 3],
    shading: PolygonShading,
    semi_transparent: bool,
    global_semi_transparency_mode: SemiTransparencyMode,
    texture_params: &'a PolygonTextureParameters,
    texture_window: TextureWindow,
    texture_mode: TextureMode,
    dithering: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
}

#[allow(clippy::too_many_arguments)]
fn rasterize_pixel(
    px: i32,
    py: i32,
    vram: &mut Vram,
    RasterizePixelArgs {
        v,
        shading,
        semi_transparent,
        global_semi_transparency_mode,
        texture_params,
        texture_window,
        texture_mode,
        dithering,
        force_mask_bit,
        check_mask_bit,
    }: RasterizePixelArgs<'_>,
) {
    let vram_addr = (2048 * py + 2 * px) as usize;
    if check_mask_bit && vram[vram_addr + 1] & 0x80 != 0 {
        return;
    }

    let p = VertexFloat { x: px.into(), y: py.into() };

    // A given point is contained within the triangle if the Z component of the cross-product of
    // v0->p and v0->v1 is non-negative for each edge v0->v1 (assuming the vertices are ordered
    // clockwise)
    for edge in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
        let cpz = cross_product_z(edge.0, edge.1, p);
        if cpz < 0.0 {
            return;
        }

        // If the cross product is 0, the point is collinear with these two vertices.
        // The PS1 GPU does not draw edges on the bottom of the triangle when this happens,
        // nor does it draw a vertical right edge
        if cpz.abs() < 1e-3 {
            // Since the vertices are clockwise, decreasing Y means this edge is on the
            // bottom or right of the triangle.
            if edge.1.y < edge.0.y {
                return;
            }

            // If the Y values are equal and X is decreasing, this is a horizontal bottom edge
            if (edge.1.y - edge.0.y).abs() < 1e-3 && edge.1.x < edge.0.x {
                return;
            }
        }
    }

    let shading_color = match shading {
        PolygonShading::Flat(color) => color,
        PolygonShading::Gouraud(color0, color1, color2) => {
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
                &texture_window,
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
                TextureMode::Modulated => modulate_color(raw_texture_color, shading_color),
                TextureMode::None => unreachable!("nested match expressions"),
            };

            (texture_color, texture_pixel & 0x8000 != 0)
        }
    };

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
        && (matches!(shading, PolygonShading::Gouraud(..))
            || texture_mode == TextureMode::Modulated)
    {
        let dither_value = DITHER_TABLE[(py & 3) as usize][(px & 3) as usize];
        masked_color.dither(dither_value)
    } else {
        masked_color
    };

    let [color_lsb, color_msb] = dithered_color.truncate_to_15_bit().to_le_bytes();
    vram[vram_addr] = color_lsb;
    vram[vram_addr + 1] = color_msb | (u8::from(mask_bit || force_mask_bit) << 7);
}

fn modulate(texture_color: u8, shading_color: u8) -> u8 {
    cmp::min(255, u16::from(texture_color) * u16::from(shading_color) / 128) as u8
}

fn modulate_color(texture_color: Color, shading_color: Color) -> Color {
    Color {
        r: modulate(texture_color.r, shading_color.r),
        g: modulate(texture_color.g, shading_color.g),
        b: modulate(texture_color.b, shading_color.b),
    }
}

fn apply_gouraud_shading(p: VertexFloat, v: [VertexFloat; 3], colors: [Color; 3]) -> Color {
    if colors[0] == colors[1] && colors[1] == colors[2] {
        return colors[0];
    }

    let rf = colors.map(|color| f64::from(color.r));
    let gf = colors.map(|color| f64::from(color.g));
    let bf = colors.map(|color| f64::from(color.b));

    // Interpolate between the color of each vertex using Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, v[0], v[1], v[2]);
    let r = alpha * rf[0] + beta * rf[1] + gamma * rf[2];
    let g = alpha * gf[0] + beta * gf[1] + gamma * gf[2];
    let b = alpha * bf[0] + beta * bf[1] + gamma * bf[2];

    Color { r: r.round() as u8, g: g.round() as u8, b: b.round() as u8 }
}

fn interpolate_uv_coordinates(
    p: VertexFloat,
    vertices: [VertexFloat; 3],
    u: [u8; 3],
    v: [u8; 3],
) -> (u8, u8) {
    let uf = u.map(f64::from);
    let vf = v.map(f64::from);

    // Similar to Gouraud shading, interpolate the U/V coordinates between vertices by using
    // Barycentric/affine coordinates
    let (alpha, beta, gamma) = compute_affine_coordinates(p, vertices[0], vertices[1], vertices[2]);

    let u = alpha * uf[0] + beta * uf[1] + gamma * uf[2];
    let v = alpha * vf[0] + beta * vf[1] + gamma * vf[2];

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
    texture_window: &TextureWindow,
    clut_x: u32,
    clut_y: u32,
    u: u32,
    v: u32,
) -> u16 {
    let u = (u & !(texture_window.x_mask << 3))
        | ((texture_window.x_offset & texture_window.x_mask) << 3);
    let v = (v & !(texture_window.y_mask << 3))
        | ((texture_window.y_offset & texture_window.y_mask) << 3);

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

#[derive(Debug, Clone, Default)]
pub struct RectangleTextureParameters {
    pub clut_x: u16,
    pub clut_y: u16,
    pub u: u8,
    pub v: u8,
}

#[derive(Debug, Clone)]
pub struct DrawRectangleParameters {
    pub position: Vertex,
    pub width: u32,
    pub height: u32,
    pub color: Color,
    pub semi_transparent: bool,
    pub texture_params: RectangleTextureParameters,
    pub texture_mode: TextureMode,
}

pub fn rectangle(
    DrawRectangleParameters {
        position,
        width,
        height,
        color,
        semi_transparent,
        texture_params,
        texture_mode,
    }: DrawRectangleParameters,
    draw_settings: &DrawSettings,
    global_texpage: TexturePage,
    texture_window: TextureWindow,
    vram: &mut Vram,
) {
    let (draw_min_x, draw_min_y) = draw_settings.draw_area_top_left;
    let (draw_max_x, draw_max_y) = draw_settings.draw_area_bottom_right;
    if draw_min_x > draw_max_x || draw_min_y > draw_max_y {
        return;
    }

    // Apply drawing offset
    let position = Vertex {
        x: position.x + draw_settings.draw_offset.0,
        y: position.y + draw_settings.draw_offset.1,
    };

    let min_x = cmp::max(draw_min_x as i32, position.x);
    let max_x = cmp::min(draw_max_x as i32, position.x + width as i32 - 1);
    let min_y = cmp::max(draw_min_y as i32, position.y);
    let max_y = cmp::min(draw_max_y as i32, position.y + height as i32 - 1);
    if min_x > max_x || min_y > max_y {
        // Drawing area is invalid (or size is 0 in one or both dimensions); do nothing
        return;
    }

    let args = RectangleArgs {
        x_range: (min_x as u32, max_x as u32),
        y_range: (min_y as u32, max_y as u32),
        color,
        semi_transparent,
        semi_transparency_mode: global_texpage.semi_transparency_mode,
        force_mask_bit: draw_settings.force_mask_bit,
        check_mask_bit: draw_settings.check_mask_bit,
    };
    match texture_mode {
        TextureMode::None => rectangle_solid_color(args, vram),
        TextureMode::Raw | TextureMode::Modulated => rectangle_textured(
            args,
            RectangleTextureParameters {
                u: texture_params.u.wrapping_add((min_x - position.x) as u8),
                v: texture_params.v.wrapping_add((min_y - position.y) as u8),
                ..texture_params
            },
            global_texpage,
            texture_window,
            texture_mode,
            vram,
        ),
    }
}

struct RectangleArgs {
    x_range: (u32, u32),
    y_range: (u32, u32),
    color: Color,
    semi_transparent: bool,
    semi_transparency_mode: SemiTransparencyMode,
    force_mask_bit: bool,
    check_mask_bit: bool,
}

fn rectangle_solid_color(
    RectangleArgs {
        x_range,
        y_range,
        color,
        semi_transparent,
        semi_transparency_mode,
        force_mask_bit,
        check_mask_bit,
    }: RectangleArgs,
    vram: &mut Vram,
) {
    for y in y_range.0..=y_range.1 {
        for x in x_range.0..=x_range.1 {
            let vram_addr = (2048 * y + 2 * x) as usize;
            if check_mask_bit && vram[vram_addr + 1] & 0x80 != 0 {
                continue;
            }

            let color = if semi_transparent {
                let existing_color =
                    Color::from_15_bit(u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]));
                semi_transparency_mode.apply(existing_color, color)
            } else {
                color
            };

            let [color_lsb, color_msb] = color.truncate_to_15_bit().to_le_bytes();
            vram[vram_addr] = color_lsb;
            vram[vram_addr + 1] = color_msb | (u8::from(force_mask_bit) << 7);
        }
    }
}

fn rectangle_textured(
    RectangleArgs {
        x_range,
        y_range,
        color: rectangle_color,
        semi_transparent,
        semi_transparency_mode,
        force_mask_bit,
        check_mask_bit,
    }: RectangleArgs,
    texture_params: RectangleTextureParameters,
    global_texpage: TexturePage,
    texture_window: TextureWindow,
    texture_mode: TextureMode,
    vram: &mut Vram,
) {
    for y in y_range.0..=y_range.1 {
        let v = texture_params.v.wrapping_add((y - y_range.0) as u8);
        for x in x_range.0..=x_range.1 {
            let vram_addr = (2048 * y + 2 * x) as usize;
            if check_mask_bit && vram[vram_addr + 1] & 0x80 != 0 {
                continue;
            }

            let u = texture_params.u.wrapping_add((x - x_range.0) as u8);
            let texture_pixel = sample_texture(
                vram,
                &global_texpage,
                &texture_window,
                texture_params.clut_x.into(),
                texture_params.clut_y.into(),
                u.into(),
                v.into(),
            );
            if texture_pixel == 0x0000 {
                continue;
            }

            let raw_texture_color = Color::from_15_bit(texture_pixel);

            let texture_color = if texture_mode == TextureMode::Modulated {
                modulate_color(raw_texture_color, rectangle_color)
            } else {
                raw_texture_color
            };

            let texture_mask_bit = texture_pixel & 0x8000 != 0;
            let masked_color = if semi_transparent && texture_mask_bit {
                let existing_color =
                    Color::from_15_bit(u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]));
                semi_transparency_mode.apply(existing_color, texture_color)
            } else {
                texture_color
            };

            let [color_lsb, color_msb] = masked_color.truncate_to_15_bit().to_le_bytes();
            vram[vram_addr] = color_lsb;
            vram[vram_addr + 1] = color_msb | (u8::from(texture_mask_bit || force_mask_bit) << 7);
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
