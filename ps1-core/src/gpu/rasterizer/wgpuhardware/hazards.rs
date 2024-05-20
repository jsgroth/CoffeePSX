use crate::gpu::rasterizer::wgpuhardware::VRAM_HEIGHT;
use crate::gpu::Vertex;
use std::cmp;

#[derive(Debug)]
pub struct HazardTracker {
    // 1 bit per 8-pixel-wide group in each line
    pub atlas: Box<[u128; VRAM_HEIGHT as usize]>,
}

impl HazardTracker {
    pub fn new() -> Self {
        Self { atlas: vec![0; VRAM_HEIGHT as usize].into_boxed_slice().try_into().unwrap() }
    }

    pub fn any_marked_rendered(&self, top_left: Vertex, bottom_right: Vertex) -> bool {
        let (tl_x, br_x) = x_tile_coordinates(top_left.x, bottom_right.x);

        let x_mask = x_bit_mask(tl_x, br_x);
        self.atlas[top_left.y as usize..bottom_right.y as usize]
            .iter()
            .any(|&row| row & x_mask != 0)
    }

    pub fn mark_rendered(&mut self, top_left: Vertex, bottom_right: Vertex) {
        let (tl_x, br_x) = x_tile_coordinates(top_left.x, bottom_right.x);

        let x_mask = x_bit_mask(tl_x, br_x);
        for row in &mut self.atlas[top_left.y as usize..bottom_right.y as usize] {
            *row |= x_mask;
        }
    }

    pub fn bounding_box(&self) -> Option<(Vertex, Vertex)> {
        let mut min_x = 1024;
        let mut max_x = 0;
        let mut min_y = 512;
        let mut max_y = 0;

        for (y, &row) in self.atlas.iter().enumerate() {
            if row == 0 {
                continue;
            }

            min_y = cmp::min(min_y, y as i32);
            max_y = cmp::max(max_y, (y + 1) as i32);

            let start_x = row.trailing_zeros() as i32;
            let end_x = (128 - row.leading_zeros()) as i32;

            min_x = cmp::min(min_x, start_x);
            max_x = cmp::max(max_x, end_x);
        }

        if min_x > max_x || min_y > max_y {
            return None;
        }

        Some((Vertex::new(min_x * 8, min_y), Vertex::new(max_x * 8, max_y)))
    }

    pub fn clear(&mut self) {
        self.atlas.fill(0);
    }
}

fn x_tile_coordinates(top_left_x: i32, bottom_right_x: i32) -> (i32, i32) {
    (top_left_x / 8, (bottom_right_x + 7) / 8)
}

fn x_bit_mask(min_x: i32, max_x: i32) -> u128 {
    if max_x >= 128 {
        return if min_x >= 128 { 0 } else { !((1 << min_x) - 1) };
    }

    ((1 << max_x) - 1) & !((1 << min_x) - 1)
}
