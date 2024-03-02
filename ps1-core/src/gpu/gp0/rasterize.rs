use crate::gpu::gp0::{
    Color, DrawPolygonParameters, PolygonCommandParameters, TextureColorDepthBits, TexturePage,
    Vertex,
};
use crate::gpu::{Gpu, Vram};
use std::{cmp, mem};

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
    pub semi_transparent: bool,
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

impl Gpu {
    // TODO rewrite this, code is terrible
    pub(super) fn rasterize_triangle(
        &mut self,
        DrawPolygonParameters {
            vertices,
            shading,
            texture_params,
            texture_mode,
        }: DrawPolygonParameters,
    ) {
        let (draw_min_x, draw_min_y) = self.gp0.draw_settings.draw_area_top_left;
        let (draw_max_x, draw_max_y) = self.gp0.draw_settings.draw_area_bottom_right;

        if draw_min_x > draw_max_x || draw_min_y > draw_max_y {
            return;
        }

        let (x_offset, y_offset) = self.gp0.draw_settings.draw_offset;

        let [v0, v1, v2] = vertices;
        let v0 = Vertex {
            x: v0.x + x_offset,
            y: v0.y + y_offset,
        };
        let v1 = Vertex {
            x: v1.x + x_offset,
            y: v1.y + y_offset,
        };
        let v2 = Vertex {
            x: v2.x + x_offset,
            y: v2.y + y_offset,
        };

        let min_x =
            cmp::min(v0.x, cmp::min(v1.x, v2.x)).clamp(draw_min_x as i32, draw_max_x as i32);
        let max_x =
            cmp::max(v0.x, cmp::max(v1.x, v2.x)).clamp(draw_min_x as i32, draw_max_x as i32);
        let min_y =
            cmp::min(v0.y, cmp::min(v1.y, v2.y)).clamp(draw_min_y as i32, draw_max_y as i32);
        let max_y =
            cmp::max(v0.y, cmp::max(v1.y, v2.y)).clamp(draw_min_y as i32, draw_max_y as i32);

        log::trace!("Vertices: {v0:?}, {v1:?}, {v2:?}");
        log::trace!("Shading: {shading:?}");
        log::trace!("Texpage: {:?}", texture_params.texpage);
        log::trace!(
            "U/V coordinates: {:?} {:?}",
            texture_params.u,
            texture_params.v
        );
        log::trace!("Bounding box: X=[{min_x}, {max_x}], Y=[{min_y}, {max_y}]");

        let mut v0 = v0.to_float();
        let mut v1 = v1.to_float();
        let v2 = v2.to_float();

        let mut tex_u = texture_params.u;
        let mut tex_v = texture_params.v;

        // Ensure vertices are ordered correctly; the PS1 GPU does not cull based on facing
        let mut swapped = false;
        if cross_product_z(v0, v1, v2) < 0.0 {
            mem::swap(&mut v0, &mut v1);
            tex_u.swap(0, 1);
            tex_v.swap(0, 1);
            swapped = true;
        }

        for py in min_y..=max_y {
            'x: for px in min_x..=max_x {
                // The sampling point is in the center of the pixel (add 0.5 to both coordinates)
                let p = VertexFloat {
                    x: px as f64 + 0.5,
                    y: py as f64 + 0.5,
                };

                for (edge_0, edge_1) in [(v0, v1), (v1, v2), (v2, v0)] {
                    let cpz = cross_product_z(edge_0, edge_1, p);
                    if cpz < 0.0 {
                        continue 'x;
                    }

                    if cpz.abs() < 1e-3 {
                        if (edge_0.x - edge_1.x).abs() < 1e-3 && edge_1.y > edge_0.y {
                            continue 'x;
                        }

                        if edge_1.x < edge_0.x {
                            continue 'x;
                        }
                    }
                }

                let mut shading_color = match shading {
                    Shading::Flat(color) => color,
                    Shading::Gouraud(mut color0, mut color1, color2) => {
                        if swapped {
                            mem::swap(&mut color0, &mut color1);
                        }

                        let (alpha, beta, gamma) = compute_affine_coordinates(p, v0, v1, v2);
                        let r = alpha * color0.r as f64
                            + beta * color1.r as f64
                            + gamma * color2.r as f64;
                        let g = alpha * color0.g as f64
                            + beta * color1.g as f64
                            + gamma * color2.g as f64;
                        let b = alpha * color0.b as f64
                            + beta * color1.b as f64
                            + gamma * color2.b as f64;

                        let color = Color {
                            r: r.round() as u8,
                            g: g.round() as u8,
                            b: b.round() as u8,
                        };

                        color
                    }
                };

                if self.gp0.draw_settings.dithering_enabled
                    && (matches!(shading, Shading::Gouraud(..))
                        || texture_mode == TextureMode::Modulated)
                {
                    let dither = DITHER_TABLE[(py & 3) as usize][(px & 3) as usize];
                    shading_color.r = shading_color.r.saturating_add_signed(dither);
                    shading_color.g = shading_color.g.saturating_add_signed(dither);
                    shading_color.b = shading_color.b.saturating_add_signed(dither);
                }

                let truncated_color = match texture_mode {
                    TextureMode::None => shading_color.truncate_to_15_bit(),
                    TextureMode::Raw | TextureMode::Modulated => {
                        let (alpha, beta, gamma) = compute_affine_coordinates(p, v0, v1, v2);

                        let u = alpha * tex_u[0] as f64
                            + beta * tex_u[1] as f64
                            + gamma * tex_u[2] as f64;

                        let v = alpha * tex_v[0] as f64
                            + beta * tex_v[1] as f64
                            + gamma * tex_v[2] as f64;

                        let u = u.round() as u8;
                        let v = v.round() as u8;

                        let tex_pixel = sample_texture(
                            &self.vram,
                            &texture_params.texpage,
                            texture_params.clut_x.into(),
                            texture_params.clut_y.into(),
                            u.into(),
                            v.into(),
                        );
                        if tex_pixel == 0 {
                            continue;
                        }

                        let tex_r = tex_pixel & 0x1F;
                        let tex_g = (tex_pixel >> 5) & 0x1F;
                        let tex_b = (tex_pixel >> 10) & 0x1F;

                        match texture_mode {
                            TextureMode::Raw => tex_r | (tex_g << 5) | (tex_b << 10),
                            TextureMode::Modulated => {
                                let r = tex_r as f64 * shading_color.r as f64 / 128.0;
                                let g = tex_g as f64 * shading_color.g as f64 / 128.0;
                                let b = tex_b as f64 * shading_color.b as f64 / 128.0;

                                let r = r.round().clamp(0.0, 31.0) as u16;
                                let g = g.round().clamp(0.0, 31.0) as u16;
                                let b = b.round().clamp(0.0, 31.0) as u16;

                                r | (g << 5) | (b << 10)
                            }
                            TextureMode::None => unreachable!(),
                        }
                    }
                };

                if texture_mode != TextureMode::None && truncated_color == 0 {
                    continue;
                }

                let [color_lsb, color_msb] = truncated_color.to_le_bytes();
                let vram_addr = (2048 * py + 2 * px) as usize;
                self.vram[vram_addr] = color_lsb;
                self.vram[vram_addr + 1] = color_msb;
            }
        }
    }
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
