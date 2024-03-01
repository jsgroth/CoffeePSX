use crate::gpu::gp0::{Color, Vertex};
use crate::gpu::Gpu;
use std::{cmp, mem};

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

impl Gpu {
    pub(super) fn rasterize_triangle(
        &mut self,
        v0: Vertex,
        v1: Vertex,
        v2: Vertex,
        shading: Shading,
    ) {
        // if !self.gp0_state.draw_settings.drawing_enabled {
        //     return;
        // }

        let (draw_min_x, draw_min_y) = self.gp0_state.draw_settings.draw_area_top_left;
        let (draw_max_x, draw_max_y) = self.gp0_state.draw_settings.draw_area_bottom_right;

        if draw_min_x > draw_max_x || draw_min_y > draw_max_y {
            return;
        }

        let (x_offset, y_offset) = self.gp0_state.draw_settings.draw_offset;

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
        log::trace!("Bounding box: X=[{min_x}, {max_x}], Y=[{min_y}, {max_y}]");

        let mut v0 = v0.to_float();
        let mut v1 = v1.to_float();
        let v2 = v2.to_float();

        // Ensure vertices are ordered correctly; the PS1 GPU does not cull based on facing
        if cross_product_z(v0, v1, v2) < 0.0 {
            mem::swap(&mut v0, &mut v1);
        }

        for py in min_y..=max_y {
            'x: for px in min_x..=max_x {
                // The sampling point is in the center of the pixel (add 0.5 to both coordinates)
                let p = VertexFloat {
                    x: px as f64 + 0.5,
                    y: py as f64 + 0.5,
                };

                for (edge_0, edge_1) in [(v0, v1), (v1, v2), (v2, v0)] {
                    if cross_product_z(edge_0, edge_1, p) < 0.0 {
                        continue 'x;
                    }
                }

                // TODO actually implement Gouraud shading, and also make this more efficient
                let [color_lsb, color_msb] = match shading {
                    Shading::Flat(color) => color.truncate_to_15_bit().to_le_bytes(),
                    Shading::Gouraud(color, _, _) => color.truncate_to_15_bit().to_le_bytes(),
                };

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
