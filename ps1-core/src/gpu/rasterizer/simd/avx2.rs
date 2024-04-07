#![allow(clippy::many_single_char_names)]

use crate::gpu::gp0::{SemiTransparencyMode, TextureColorDepthBits, TexturePage, TextureWindow};
use crate::gpu::rasterizer::simd::AlignedVram;
use crate::gpu::rasterizer::{
    Color, LineShading, RectangleTextureMapping, TextureMappingMode, TriangleTextureMapping, Vertex,
};
#[allow(clippy::wildcard_imports)]
use std::arch::x86_64::*;
use std::{cmp, mem};

const DITHER_TABLE: &[[i16; 16]; 4] = &[
    [-4, 0, -3, 1, -4, 0, -3, 1, -4, 0, -3, 1, -4, 0, -3, 1],
    [2, -2, 3, -1, 2, -2, 3, -1, 2, -2, 3, -1, 2, -2, 3, -1],
    [-3, 1, -4, 0, -3, 1, -4, 0, -3, 1, -4, 0, -3, 1, -4, 0],
    [3, -1, 2, -2, 3, -1, 2, -2, 3, -1, 2, -2, 3, -1, 2, -2],
];

pub enum TriangleShadingAvx2 {
    Flat(Color),
    Gouraud { r: [f32; 3], g: [f32; 3], b: [f32; 3] },
}

impl TriangleShadingAvx2 {
    // Determine the color for the given normalized Barycentric coordinates. Return values are
    // 8-bit RGB color components.
    // (f32x8, f32x8, f32x8) -> (i32x8, i32x8, i32x8)
    #[target_feature(enable = "avx2", enable = "fma")]
    unsafe fn shade(&self, barycentric: (__m256, __m256, __m256)) -> (__m256i, __m256i, __m256i) {
        match *self {
            Self::Flat(color) => (
                _mm256_set1_epi32(color.r.into()),
                _mm256_set1_epi32(color.g.into()),
                _mm256_set1_epi32(color.b.into()),
            ),
            Self::Gouraud { r, g, b } => gouraud_shade(barycentric, r, g, b),
        }
    }
}

pub struct TriangleTextureMappingAvx2 {
    mode: TextureMappingMode,
    texpage: TexturePage,
    window: TextureWindow,
    clut_x: u32,
    clut_y: u32,
    u: [f32; 3],
    v: [f32; 3],
}

impl TriangleTextureMappingAvx2 {
    pub fn new(mapping: TriangleTextureMapping) -> Self {
        Self {
            mode: mapping.mode,
            texpage: mapping.texpage,
            window: mapping.window,
            clut_x: mapping.clut_x.into(),
            clut_y: mapping.clut_y.into(),
            u: mapping.u.map(f32::from),
            v: mapping.v.map(f32::from),
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
pub unsafe fn rasterize_triangle(
    vram: &mut AlignedVram,
    x_bounds: (i32, i32),
    y_bounds: (i32, i32),
    vertices: [Vertex; 3],
    shading: TriangleShadingAvx2,
    texture_mapping: Option<TriangleTextureMappingAvx2>,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    dithering_enabled: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    let vram_ptr = vram.as_mut_ptr();

    let inverse_barycentric_determinant = compute_inverse_barycentric_determinant(vertices);
    let forced_mask_bit = i16::from(force_mask_bit) << 15;

    let v01_is_not_bottom_right = is_not_bottom_right_edge(vertices[0], vertices[1]);
    let v12_is_not_bottom_right = is_not_bottom_right_edge(vertices[1], vertices[2]);
    let v20_is_not_bottom_right = is_not_bottom_right_edge(vertices[2], vertices[0]);

    // AVX2 loads/stores must be aligned to a 16-halfword/32-byte boundary
    let min_x_aligned = x_bounds.0 / 16 * 16;
    let max_x_aligned = x_bounds.1 / 16 * 16;

    let zero = _mm256_setzero_si256();
    let negative_one = _mm256_set1_epi32(-1);

    for y in y_bounds.0..=y_bounds.1 {
        let py = _mm256_set1_epi32(y);

        for x in (min_x_aligned..=max_x_aligned).step_by(16) {
            // Determine which X coordinates are inside the triangle.
            // The 16 X coordinates are split up such that vectors can later be converted from
            // two i32x8 vectors to a single i16x16 vector using _mm256_packs_epi32
            let px1 = _mm256_setr_epi32(x, x + 1, x + 2, x + 3, x + 8, x + 9, x + 10, x + 11);
            let inside_mask_1 = compute_write_mask(
                vertices,
                px1,
                py,
                zero,
                v01_is_not_bottom_right,
                v12_is_not_bottom_right,
                v20_is_not_bottom_right,
            );

            let px2 = _mm256_setr_epi32(x + 4, x + 5, x + 6, x + 7, x + 12, x + 13, x + 14, x + 15);
            let inside_mask_2 = compute_write_mask(
                vertices,
                px2,
                py,
                zero,
                v01_is_not_bottom_right,
                v12_is_not_bottom_right,
                v20_is_not_bottom_right,
            );

            let mut inside_mask = _mm256_packs_epi32(inside_mask_1, inside_mask_2);

            // If no points are inside the triangle, bail out early
            if _mm256_testz_si256(inside_mask, negative_one) != 0 {
                continue;
            }

            // Compute normalized Barycentric coordinates if they will be needed
            let (barycentric1, barycentric2) =
                if matches!(shading, TriangleShadingAvx2::Gouraud { .. })
                    || texture_mapping.is_some()
                {
                    (
                        compute_barycentric_coordinates(
                            px1,
                            py,
                            vertices,
                            inverse_barycentric_determinant,
                        ),
                        compute_barycentric_coordinates(
                            px2,
                            py,
                            vertices,
                            inverse_barycentric_determinant,
                        ),
                    )
                } else {
                    let zero_f = _mm256_setzero_ps();
                    ((zero_f, zero_f, zero_f), (zero_f, zero_f, zero_f))
                };

            // Apply shading to determine initial color
            let (r1, g1, b1) = shading.shade(barycentric1);
            let (r2, g2, b2) = shading.shade(barycentric2);
            let (mut r, mut g, mut b) = (
                _mm256_packs_epi32(r1, r2),
                _mm256_packs_epi32(g1, g2),
                _mm256_packs_epi32(b1, b2),
            );

            // Default to values for an untextured triangle: bit 15 is set only if the force
            // mask bit setting is on, and all pixels are semi-transparent
            let mut mask_bits = _mm256_set1_epi16(forced_mask_bit);
            let mut semi_transparency_bits = _mm256_set1_epi16(1 << 15);

            // Apply texture mapping if present
            if let Some(texture_mapping) = &texture_mapping {
                // Interpolate U/V coordinates
                let (u1, v1) = interpolate_uv(barycentric1, texture_mapping.u, texture_mapping.v);
                let (u2, v2) = interpolate_uv(barycentric2, texture_mapping.u, texture_mapping.v);
                let (u, v) = (_mm256_packus_epi32(u1, u2), _mm256_packus_epi32(v1, v2));

                // Read 16 texels from the texture
                let texels = read_texture(
                    vram_ptr,
                    &texture_mapping.texpage,
                    &texture_mapping.window,
                    texture_mapping.clut_x,
                    texture_mapping.clut_y,
                    u,
                    v,
                );

                // Mask out any pixels where the texel value is $0000
                inside_mask = _mm256_andnot_si256(_mm256_cmpeq_epi16(texels, zero), inside_mask);

                // Texels are semi-transparent only if bit 15 is set
                let texture_mask_bits = _mm256_set1_epi16(1 << 15);
                mask_bits = _mm256_or_si256(mask_bits, texture_mask_bits);
                semi_transparency_bits = _mm256_and_si256(texels, texture_mask_bits);

                let (tr, tg, tb) = convert_15bit_to_24bit(texels);

                // Optionally apply texture color modulation
                match texture_mapping.mode {
                    TextureMappingMode::Raw => {
                        r = tr;
                        g = tg;
                        b = tb;
                    }
                    TextureMappingMode::Modulated => {
                        r = modulate_texture_color(tr, r);
                        g = modulate_texture_color(tg, g);
                        b = modulate_texture_color(tb, b);
                    }
                };
            }

            // Load the existing row of 16 pixels
            let vram_addr = vram_ptr.add(1024 * y as usize + x as usize).cast::<__m256i>();
            let existing = _mm256_load_si256(vram_addr);

            if check_mask_bit {
                // Mask out any pixels where the existing pixel has bit 15 set
                inside_mask = _mm256_and_si256(
                    inside_mask,
                    _mm256_cmpeq_epi16(
                        _mm256_and_si256(existing, _mm256_set1_epi16(1 << 15)),
                        zero,
                    ),
                );
            }

            // If semi-transparency is enabled, blend existing colors with new colors
            if let Some(semi_transparency_mode) = semi_transparency_mode {
                if _mm256_testz_si256(semi_transparency_bits, negative_one) == 0 {
                    let (existing_r, existing_g, existing_b) = convert_15bit_to_24bit(existing);
                    let semi_transparency_mask = _mm256_cmpeq_epi16(semi_transparency_bits, zero);

                    (r, g, b) = apply_semi_transparency(
                        (existing_r, existing_g, existing_b),
                        (r, g, b),
                        semi_transparency_mask,
                        semi_transparency_mode,
                    );
                }
            }

            // If dithering is enabled, apply dithering before truncating to RGB555.
            // Dithering is applied only if Gouraud shading or texture color modulation is enabled
            if dithering_enabled
                && (matches!(shading, TriangleShadingAvx2::Gouraud { .. })
                    || texture_mapping
                        .as_ref()
                        .is_some_and(|mapping| mapping.mode == TextureMappingMode::Modulated))
            {
                let dither_vector: __m256i = mem::transmute(DITHER_TABLE[(y & 3) as usize]);

                let u8_max = _mm256_set1_epi16(255);
                r = _mm256_min_epi16(
                    u8_max,
                    _mm256_max_epi16(zero, _mm256_add_epi16(r, dither_vector)),
                );
                g = _mm256_min_epi16(
                    u8_max,
                    _mm256_max_epi16(zero, _mm256_add_epi16(g, dither_vector)),
                );
                b = _mm256_min_epi16(
                    u8_max,
                    _mm256_max_epi16(zero, _mm256_add_epi16(b, dither_vector)),
                );
            }

            // Truncate to RGB555 and OR in bit 15 (either force mask bit or texel bit 15)
            let color = _mm256_or_si256(convert_24bit_to_15bit(r, g, b), mask_bits);

            // Store the row of pixels, using the write mask to control which are written
            _mm256_store_si256(
                vram_addr,
                _mm256_or_si256(
                    _mm256_and_si256(inside_mask, color),
                    _mm256_andnot_si256(inside_mask, existing),
                ),
            );
        }
    }
}

fn is_not_bottom_right_edge(v0: Vertex, v1: Vertex) -> i32 {
    let is_bottom_right = v1.y > v0.y || (v1.y == v0.y && v1.x < v0.x);
    if is_bottom_right { 0 } else { !0 }
}

// Determine which of the 8 points are inside the triangle.
// Input vectors should be i32x8.
// Return value is i32x8 where each lane is all 1s if inside the triangle and all 0s if outside.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn compute_write_mask(
    vertices: [Vertex; 3],
    px: __m256i,
    py: __m256i,
    zero: __m256i,
    v01_is_not_bottom_right: i32,
    v12_is_not_bottom_right: i32,
    v20_is_not_bottom_right: i32,
) -> __m256i {
    _mm256_and_si256(
        check_edge(vertices[0], vertices[1], px, py, zero, v01_is_not_bottom_right),
        _mm256_and_si256(
            check_edge(vertices[1], vertices[2], px, py, zero, v12_is_not_bottom_right),
            check_edge(vertices[2], vertices[0], px, py, zero, v20_is_not_bottom_right),
        ),
    )
}

// Determine which of the 8 points are inside a single triangle edge.
// Input vectors should be i32x8.
// Return value is i32x8 where each lane is all 1s if inside the edge and all 0s if outside.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn check_edge(
    v0: Vertex,
    v1: Vertex,
    px: __m256i,
    py: __m256i,
    zero: __m256i,
    is_not_bottom_right: i32,
) -> __m256i {
    let cpz = cross_product_z(v0, v1, px, py);
    _mm256_or_si256(
        _mm256_cmpgt_epi32(cpz, zero),
        _mm256_and_si256(_mm256_cmpeq_epi32(cpz, zero), _mm256_set1_epi32(is_not_bottom_right)),
    )
}

// Compute the Z component of the cross product (v1 - v0) x (P - v0) for each point P.
// Input vectors should be i32x8 and return value is i32x8.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn cross_product_z(v0: Vertex, v1: Vertex, px: __m256i, py: __m256i) -> __m256i {
    _mm256_sub_epi32(
        _mm256_mullo_epi32(
            _mm256_set1_epi32(v1.x - v0.x),
            _mm256_sub_epi32(py, _mm256_set1_epi32(v0.y)),
        ),
        _mm256_mullo_epi32(
            _mm256_set1_epi32(v1.y - v0.y),
            _mm256_sub_epi32(px, _mm256_set1_epi32(v0.x)),
        ),
    )
}

// Compute 1/det(T) where T is the transformation matrix used to compute Barycentric coordinates.
fn compute_inverse_barycentric_determinant([v0, v1, v2]: [Vertex; 3]) -> f32 {
    let determinant = (v0.x - v2.x) * (v1.y - v2.y) - (v1.x - v2.x) * (v0.y - v2.y);
    if determinant == 0 {
        // TODO what to do here? the points are collinear
        0.0
    } else {
        (1.0 / f64::from(determinant)) as f32
    }
}

// Compute the normalized Barycentric coordinates for the given points.
// Input vectors should be i32x8.
// Return values are f32x8, one vector for each coordinate.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn compute_barycentric_coordinates(
    px: __m256i,
    py: __m256i,
    [v0, v1, v2]: [Vertex; 3],
    inverse_determinant: f32,
) -> (__m256, __m256, __m256) {
    if inverse_determinant.abs() < 1e-6 {
        let one_third = _mm256_set1_ps(1.0 / 3.0);
        return (one_third, one_third, one_third);
    }

    let x_sub = _mm256_sub_epi32(px, _mm256_set1_epi32(v2.x));
    let y_sub = _mm256_sub_epi32(py, _mm256_set1_epi32(v2.y));
    let inverse_determinant = _mm256_set1_ps(inverse_determinant);

    let lambda1_numerator = _mm256_sub_epi32(
        _mm256_mullo_epi32(x_sub, _mm256_set1_epi32(v1.y - v2.y)),
        _mm256_mullo_epi32(y_sub, _mm256_set1_epi32(v1.x - v2.x)),
    );
    let lambda1 = _mm256_mul_ps(_mm256_cvtepi32_ps(lambda1_numerator), inverse_determinant);

    let lambda2_numerator = _mm256_sub_epi32(
        _mm256_mullo_epi32(x_sub, _mm256_set1_epi32(v2.y - v0.y)),
        _mm256_mullo_epi32(y_sub, _mm256_set1_epi32(v2.x - v0.x)),
    );
    let lambda2 = _mm256_mul_ps(_mm256_cvtepi32_ps(lambda2_numerator), inverse_determinant);

    let lambda3 = _mm256_sub_ps(_mm256_set1_ps(1.0), _mm256_add_ps(lambda1, lambda2));

    (lambda1, lambda2, lambda3)
}

// Apply Gouraud shading.
// Input Barycentric coordinates should be f32x8.
// Return values are RGB color components in i32x8 vectors, with each component clamped to [0, 255].
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn gouraud_shade(
    lambda: (__m256, __m256, __m256),
    r_in: [f32; 3],
    g_in: [f32; 3],
    b_in: [f32; 3],
) -> (__m256i, __m256i, __m256i) {
    let zero = _mm256_setzero_si256();
    let u8_max = _mm256_set1_epi32(255);

    let mut r = _mm256_mul_ps(lambda.0, _mm256_set1_ps(r_in[0]));
    r = _mm256_fmadd_ps(lambda.1, _mm256_set1_ps(r_in[1]), r);
    r = _mm256_fmadd_ps(lambda.2, _mm256_set1_ps(r_in[2]), r);
    let r = _mm256_cvtps_epi32(_mm256_round_ps::<_MM_FROUND_TO_NEAREST_INT>(r));
    let r = _mm256_max_epi32(zero, _mm256_min_epi32(r, u8_max));

    let mut g = _mm256_mul_ps(lambda.0, _mm256_set1_ps(g_in[0]));
    g = _mm256_fmadd_ps(lambda.1, _mm256_set1_ps(g_in[1]), g);
    g = _mm256_fmadd_ps(lambda.2, _mm256_set1_ps(g_in[2]), g);
    let g = _mm256_cvtps_epi32(_mm256_round_ps::<_MM_FROUND_TO_NEAREST_INT>(g));
    let g = _mm256_max_epi32(zero, _mm256_min_epi32(g, u8_max));

    let mut b = _mm256_mul_ps(lambda.0, _mm256_set1_ps(b_in[0]));
    b = _mm256_fmadd_ps(lambda.1, _mm256_set1_ps(b_in[1]), b);
    b = _mm256_fmadd_ps(lambda.2, _mm256_set1_ps(b_in[2]), b);
    let b = _mm256_cvtps_epi32(_mm256_round_ps::<_MM_FROUND_TO_NEAREST_INT>(b));
    let b = _mm256_max_epi32(zero, _mm256_min_epi32(b, u8_max));

    (r, g, b)
}

// Apply semi-transparency blending.
// Input color vectors should be i16x16.
// Return values are i16x16, with all color components clamped to [0, 255].
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn apply_semi_transparency(
    (existing_r, existing_g, existing_b): (__m256i, __m256i, __m256i),
    (r, g, b): (__m256i, __m256i, __m256i),
    semi_transparency_mask: __m256i,
    semi_transparency_mode: SemiTransparencyMode,
) -> (__m256i, __m256i, __m256i) {
    let (blended_r, blended_g, blended_b) = match semi_transparency_mode {
        SemiTransparencyMode::Average => (
            blend_average(existing_r, r),
            blend_average(existing_g, g),
            blend_average(existing_b, b),
        ),
        SemiTransparencyMode::Add => {
            (blend_add(existing_r, r), blend_add(existing_g, g), blend_add(existing_b, b))
        }
        SemiTransparencyMode::Subtract => (
            blend_subtract(existing_r, r),
            blend_subtract(existing_g, g),
            blend_subtract(existing_b, b),
        ),
        SemiTransparencyMode::AddQuarter => (
            blend_add_quarter(existing_r, r),
            blend_add_quarter(existing_g, g),
            blend_add_quarter(existing_b, b),
        ),
    };

    let r = _mm256_or_si256(
        _mm256_andnot_si256(semi_transparency_mask, blended_r),
        _mm256_and_si256(semi_transparency_mask, r),
    );
    let g = _mm256_or_si256(
        _mm256_andnot_si256(semi_transparency_mask, blended_g),
        _mm256_and_si256(semi_transparency_mask, g),
    );
    let b = _mm256_or_si256(
        _mm256_andnot_si256(semi_transparency_mask, blended_b),
        _mm256_and_si256(semi_transparency_mask, b),
    );

    (r, g, b)
}

// Interpolate U/V coordinates using normalized Barycentric coordinates.
// Barycentric coordinates should be f32x8.
// Return values are i32x8, with both U and V clamped to [0, 255].
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn interpolate_uv(
    lambda: (__m256, __m256, __m256),
    u_in: [f32; 3],
    v_in: [f32; 3],
) -> (__m256i, __m256i) {
    let zero = _mm256_setzero_si256();
    let u8_max = _mm256_set1_epi32(255);

    let mut u = _mm256_mul_ps(lambda.0, _mm256_set1_ps(u_in[0]));
    u = _mm256_fmadd_ps(lambda.1, _mm256_set1_ps(u_in[1]), u);
    u = _mm256_fmadd_ps(lambda.2, _mm256_set1_ps(u_in[2]), u);
    let u = _mm256_cvtps_epi32(_mm256_round_ps::<_MM_FROUND_TO_NEAREST_INT>(u));
    let u = _mm256_max_epi32(zero, _mm256_min_epi32(u, u8_max));

    let mut v = _mm256_mul_ps(lambda.0, _mm256_set1_ps(v_in[0]));
    v = _mm256_fmadd_ps(lambda.1, _mm256_set1_ps(v_in[1]), v);
    v = _mm256_fmadd_ps(lambda.2, _mm256_set1_ps(v_in[2]), v);
    let v = _mm256_cvtps_epi32(_mm256_round_ps::<_MM_FROUND_TO_NEAREST_INT>(v));
    let v = _mm256_max_epi32(zero, _mm256_min_epi32(v, u8_max));

    (u, v)
}

// Read a row of 16 texels from a texture in VRAM.
// U and V vectors should be i16x16.
// Return value is an i16x16 vector containing raw 16-bit texel values (RGB555 + semi-transparency bit).
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn read_texture(
    vram: *mut u16,
    texpage: &TexturePage,
    texture_window: &TextureWindow,
    clut_x: u32,
    clut_y: u32,
    u: __m256i,
    v: __m256i,
) -> __m256i {
    let x_mask = _mm256_set1_epi16((texture_window.x_mask << 3) as i16);
    let y_mask = _mm256_set1_epi16((texture_window.y_mask << 3) as i16);

    let masked_u = _mm256_or_si256(
        _mm256_andnot_si256(x_mask, u),
        _mm256_and_si256(x_mask, _mm256_set1_epi16((texture_window.x_offset << 3) as i16)),
    );
    let masked_v = _mm256_or_si256(
        _mm256_andnot_si256(y_mask, v),
        _mm256_and_si256(y_mask, _mm256_set1_epi16((texture_window.y_offset << 3) as i16)),
    );

    let (masked_u0, masked_u1) = unpack_epi16_vector(masked_u);
    let (masked_v0, masked_v1) = unpack_epi16_vector(masked_v);

    let (texels0, texels1) = match texpage.color_depth {
        TextureColorDepthBits::Four => (
            read_4bpp_texture(vram, texpage, clut_x, clut_y, masked_u0, masked_v0),
            read_4bpp_texture(vram, texpage, clut_x, clut_y, masked_u1, masked_v1),
        ),
        TextureColorDepthBits::Eight => (
            read_8bpp_texture(vram, texpage, clut_x, clut_y, masked_u0, masked_v0),
            read_8bpp_texture(vram, texpage, clut_x, clut_y, masked_u1, masked_v1),
        ),
        TextureColorDepthBits::Fifteen => (
            read_15bpp_texture(vram, texpage, masked_u0, masked_v0),
            read_15bpp_texture(vram, texpage, masked_u1, masked_v1),
        ),
    };

    _mm256_packus_epi32(texels0, texels1)
}

// Read a row of 8 texels from a 4bpp texture.
// U and V vectors should be i32x8.
// Return value is u16s stored in an i32x8 vector.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn read_4bpp_texture(
    vram: *mut u16,
    texpage: &TexturePage,
    clut_x: u32,
    clut_y: u32,
    u: __m256i,
    v: __m256i,
) -> __m256i {
    let vram_y = _mm256_add_epi32(v, _mm256_set1_epi32(texpage.y_base as i32));
    let vram_x = _mm256_add_epi32(
        _mm256_srli_epi32::<2>(u),
        _mm256_set1_epi32((64 * texpage.x_base) as i32),
    );

    let vram_addr = _mm256_or_si256(_mm256_slli_epi32::<10>(vram_y), vram_x);
    let vram_shift = _mm256_slli_epi32::<2>(_mm256_and_si256(u, _mm256_set1_epi32(3)));

    let vram_addr_scalar: [i32; 8] = mem::transmute(vram_addr);
    let vram_shift_scalar: [i32; 8] = mem::transmute(vram_shift);

    let clut_offset = ((1024 * clut_y) | (16 * clut_x)) as usize;
    let clut_base_addr = vram.add(clut_offset);

    let mut texels = [0_u32; 8];
    for i in 0..8 {
        let vram_halfword = *vram.add(vram_addr_scalar[i] as usize);
        let clut_index = (vram_halfword >> vram_shift_scalar[i]) & 0xF;

        texels[i] = (*clut_base_addr.add(clut_index as usize)).into();
    }

    mem::transmute(texels)
}

// Read a row of 8 texels from an 8bpp texture.
// U and V coordinates should be i32x8.
// Return value is u16s stored in an i32x8 vector.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn read_8bpp_texture(
    vram: *mut u16,
    texpage: &TexturePage,
    clut_x: u32,
    clut_y: u32,
    u: __m256i,
    v: __m256i,
) -> __m256i {
    let vram_y = _mm256_add_epi32(v, _mm256_set1_epi32(texpage.y_base as i32));
    let vram_x = _mm256_and_si256(
        _mm256_add_epi32(
            _mm256_srli_epi32::<1>(u),
            _mm256_set1_epi32((64 * texpage.x_base) as i32),
        ),
        _mm256_set1_epi32(0x3FF),
    );

    let vram_addr = _mm256_or_si256(_mm256_slli_epi32::<10>(vram_y), vram_x);
    let vram_shift = _mm256_slli_epi32::<3>(_mm256_and_si256(u, _mm256_set1_epi32(1)));

    let vram_addr_scalar: [i32; 8] = mem::transmute(vram_addr);
    let vram_shift_scalar: [i32; 8] = mem::transmute(vram_shift);

    let clut_row_addr = (1024 * clut_y) as usize;
    let clut_row = vram.add(clut_row_addr);

    let clut_row_offset = (16 * clut_x) as u16;

    let mut texels = [0_u32; 8];
    for i in 0..8 {
        let vram_halfword = *vram.add(vram_addr_scalar[i] as usize);
        let clut_index = (vram_halfword >> vram_shift_scalar[i]) & 0xFF;

        let color_addr = ((clut_row_offset + clut_index) & 0x3FF) as usize;
        texels[i] = (*clut_row.add(color_addr)).into();
    }

    mem::transmute(texels)
}

// Read a row of 8 texels from a 15bpp texture.
// U and V vectors should be i32x8.
// Return value is u16s stored in an i32x8 vector.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn read_15bpp_texture(
    vram: *mut u16,
    texpage: &TexturePage,
    u: __m256i,
    v: __m256i,
) -> __m256i {
    let vram_y = _mm256_add_epi32(v, _mm256_set1_epi32(texpage.y_base as i32));
    let vram_x = _mm256_and_si256(
        _mm256_add_epi32(u, _mm256_set1_epi32((64 * texpage.x_base) as i32)),
        _mm256_set1_epi32(0x3FF),
    );

    let vram_addr = _mm256_or_si256(_mm256_slli_epi32::<10>(vram_y), vram_x);
    let vram_addr_scalar: [i32; 8] = mem::transmute(vram_addr);

    let mut texels = [0_u32; 8];
    for i in 0..8 {
        texels[i] = (*vram.add(vram_addr_scalar[i] as usize)).into();
    }

    mem::transmute(texels)
}

// Apply texture color modulation to a single color component.
// Input vectors should be i16x16 and the return value is i16x16.
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn modulate_texture_color(tex_color: __m256i, shading_color: __m256i) -> __m256i {
    _mm256_min_epi16(
        _mm256_set1_epi16(255),
        _mm256_srli_epi16::<7>(_mm256_mullo_epi16(tex_color, shading_color)),
    )
}

// Apply average blending: (B + F) / 2
// Input vectors should be i16x16 and the return value is i16x16
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn blend_average(back: __m256i, front: __m256i) -> __m256i {
    _mm256_srli_epi16::<1>(_mm256_add_epi16(back, front))
}

// Apply additive blending: B + F
// Input vectors should be i16x16 and the return value is i16x16, with each lane clamped to [0, 255]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn blend_add(back: __m256i, front: __m256i) -> __m256i {
    _mm256_adds_epu8(back, front)
}

// Apply subtractive blending: B - F
// Input vectors should be i16x16 and the return value is i16x16, with each lane clamped to [0, 255]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn blend_subtract(back: __m256i, front: __m256i) -> __m256i {
    _mm256_subs_epu8(back, front)
}

// Apply partial additive blending: B + F/4
// Input vectors should be i16x16 and the return value is i16x16, with each lane clamped to [0, 255]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn blend_add_quarter(back: __m256i, front: __m256i) -> __m256i {
    _mm256_adds_epu8(back, _mm256_srli_epi16::<2>(front))
}

const LOW_SHUFFLE_MASK: &[u8; 32] = &[
    0, 1, 0x80, 0x80, 2, 3, 0x80, 0x80, 4, 5, 0x80, 0x80, 6, 7, 0x80, 0x80, 16, 17, 0x80, 0x80, 18,
    19, 0x80, 0x80, 20, 21, 0x80, 0x80, 22, 23, 0x80, 0x80,
];

const HIGH_SHUFFLE_MASK: &[u8; 32] = &[
    8, 9, 0x80, 0x80, 10, 11, 0x80, 0x80, 12, 13, 0x80, 0x80, 14, 15, 0x80, 0x80, 24, 25, 0x80,
    0x80, 26, 27, 0x80, 0x80, 28, 29, 0x80, 0x80, 30, 31, 0x80, 0x80,
];

// Unpack an i16x16 vector into two i32x8 vectors such that the two vectors can later be repacked
// using _mm256_packus_epi32 or _mm256_packs_epi32
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn unpack_epi16_vector(v: __m256i) -> (__m256i, __m256i) {
    let low = _mm256_shuffle_epi8(v, mem::transmute(*LOW_SHUFFLE_MASK));
    let high = _mm256_shuffle_epi8(v, mem::transmute(*HIGH_SHUFFLE_MASK));

    (low, high)
}

// Convert a 24-bit color value to 15-bit colors by truncating the lowest 3 bits of each component
// Input vectors should be i16x16 and the return value is i16x16
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn convert_24bit_to_15bit(r: __m256i, g: __m256i, b: __m256i) -> __m256i {
    let mask = _mm256_set1_epi16(0xF8);

    _mm256_or_si256(
        _mm256_srli_epi16::<3>(r),
        _mm256_or_si256(
            _mm256_slli_epi16::<2>(_mm256_and_si256(g, mask)),
            _mm256_slli_epi16::<7>(_mm256_and_si256(b, mask)),
        ),
    )
}

// Convert a raw 15-bit color value from VRAM to individual 8-bit RGB color components
// Input vector should be i16x16 and the return values are i16x16
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn convert_15bit_to_24bit(texels: __m256i) -> (__m256i, __m256i, __m256i) {
    let mask = _mm256_set1_epi16(0x00F8);
    let r = _mm256_and_si256(_mm256_slli_epi16::<3>(texels), mask);
    let g = _mm256_and_si256(_mm256_srli_epi16::<2>(texels), mask);
    let b = _mm256_and_si256(_mm256_srli_epi16::<7>(texels), mask);

    (r, g, b)
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
pub unsafe fn rasterize_rectangle(
    vram: &mut AlignedVram,
    x_range: (i32, i32),
    y_range: (i32, i32),
    color: Color,
    texture_mapping: Option<RectangleTextureMapping>,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    let vram_ptr = vram.as_mut_ptr();

    let forced_mask_bit = i16::from(force_mask_bit) << 15;

    let min_x = x_range.0 as i16;
    let max_x = x_range.1 as i16;

    // AVX2 loads/stores must be aligned to a 16-halfword/32-byte boundary
    let min_x_aligned = min_x / 16 * 16;
    let max_x_aligned = max_x / 16 * 16;

    let min_y = y_range.0 as i16;
    let max_y = y_range.1 as i16;

    let color_r = _mm256_set1_epi16(color.r.into());
    let color_g = _mm256_set1_epi16(color.g.into());
    let color_b = _mm256_set1_epi16(color.b.into());

    let zero = _mm256_setzero_si256();

    for y in min_y..=max_y {
        let vram_row_addr = 1024 * y as usize;
        for x in (min_x_aligned..=max_x_aligned).step_by(16) {
            let px = _mm256_setr_epi16(
                x,
                x + 1,
                x + 2,
                x + 3,
                x + 4,
                x + 5,
                x + 6,
                x + 7,
                x + 8,
                x + 9,
                x + 10,
                x + 11,
                x + 12,
                x + 13,
                x + 14,
                x + 15,
            );

            // Mask out pixels that are outside of the rectangle
            let mut write_mask = _mm256_andnot_si256(
                _mm256_cmpgt_epi16(_mm256_set1_epi16(min_x), px),
                _mm256_cmpgt_epi16(_mm256_set1_epi16(max_x + 1), px),
            );

            // Read existing pixel values from VRAM
            let vram_addr = vram_ptr.add(vram_row_addr + x as usize).cast::<__m256i>();
            let existing = _mm256_load_si256(vram_addr);

            if check_mask_bit {
                // Mask out any pixels where the existing value has bit 15 set
                write_mask = _mm256_and_si256(
                    write_mask,
                    _mm256_cmpeq_epi16(
                        _mm256_and_si256(existing, _mm256_set1_epi16(1 << 15)),
                        zero,
                    ),
                );
            }

            // Initialize color to the color from the command word
            let mut r = color_r;
            let mut g = color_g;
            let mut b = color_b;

            // Default to values for an untextured rectangle: bit 15 is set only if the force
            // mask bit setting is on, and all pixels are semi-transparent
            let mut mask_bits = _mm256_set1_epi16(forced_mask_bit);
            let mut semi_transparency_bits = _mm256_set1_epi16(1 << 15);

            // Apply texture mapping if present
            if let Some(texture_mapping) = texture_mapping {
                // Compute U and V coordinates based on X and Y values, wrapping within [0, 255]
                let u = _mm256_and_si256(
                    _mm256_set1_epi16(0x00FF),
                    _mm256_add_epi16(
                        px,
                        _mm256_set1_epi16(i16::from(texture_mapping.u[0]) - min_x),
                    ),
                );
                let v =
                    _mm256_set1_epi16(texture_mapping.v[0].wrapping_add((y - min_y) as u8).into());

                // Read a row of 16 texels from the texture
                let texels = read_texture(
                    vram_ptr,
                    &texture_mapping.texpage,
                    &texture_mapping.window,
                    texture_mapping.clut_x.into(),
                    texture_mapping.clut_y.into(),
                    u,
                    v,
                );

                // Mask out any pixels where the texel value is $0000
                write_mask = _mm256_andnot_si256(_mm256_cmpeq_epi16(texels, zero), write_mask);

                // Texture pixels are semi-transparent only if texel bit 15 is set
                let bit_15_mask = _mm256_set1_epi16(1 << 15);
                mask_bits = _mm256_or_si256(mask_bits, _mm256_and_si256(texels, bit_15_mask));
                semi_transparency_bits = _mm256_and_si256(texels, bit_15_mask);

                let (tr, tg, tb) = convert_15bit_to_24bit(texels);

                // Optionally apply texture color modulation
                match texture_mapping.mode {
                    TextureMappingMode::Raw => {
                        r = tr;
                        g = tg;
                        b = tb;
                    }
                    TextureMappingMode::Modulated => {
                        r = modulate_texture_color(tr, r);
                        g = modulate_texture_color(tg, g);
                        b = modulate_texture_color(tb, b);
                    }
                }
            }

            // If semi-transparency is enabled, blend existing colors with new colors
            if let Some(semi_transparency_mode) = semi_transparency_mode {
                let semi_transparency_mask = _mm256_cmpeq_epi16(semi_transparency_bits, zero);

                let (existing_r, existing_g, existing_b) = convert_15bit_to_24bit(existing);
                (r, g, b) = apply_semi_transparency(
                    (existing_r, existing_g, existing_b),
                    (r, g, b),
                    semi_transparency_mask,
                    semi_transparency_mode,
                );
            }

            // Truncate to RGB555 and OR in the mask bit (force mask bit or texel bit 15)
            let color = _mm256_or_si256(convert_24bit_to_15bit(r, g, b), mask_bits);

            // Store the row of 16 pixels, using the write mask to control which are written
            _mm256_store_si256(
                vram_addr,
                _mm256_or_si256(
                    _mm256_and_si256(write_mask, color),
                    _mm256_andnot_si256(write_mask, existing),
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
pub unsafe fn rasterize_line(
    vram: &mut AlignedVram,
    vertices: [Vertex; 2],
    drawing_area_x: (i32, i32),
    drawing_area_y: (i32, i32),
    shading: LineShading,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    dithering_enabled: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    let x_diff = vertices[1].x - vertices[0].x;
    let y_diff = vertices[1].y - vertices[0].y;

    if x_diff == 0 && y_diff == 0 {
        // Rasterize a single pixel
        let color = match shading {
            LineShading::Flat(color) | LineShading::Gouraud([color, _]) => color,
        };

        rasterize_line_pixels(
            vram,
            _mm_set1_epi32(vertices[0].x),
            _mm_set1_epi32(vertices[0].y),
            _mm_set1_epi32(color.r.into()),
            _mm_set1_epi32(color.g.into()),
            _mm_set1_epi32(color.b.into()),
            _mm_setr_epi32(!0, 0, 0, 0),
            semi_transparency_mode,
            dithering_enabled,
            force_mask_bit,
            check_mask_bit,
        );

        return;
    }

    if x_diff.abs() > y_diff.abs() {
        rasterize_line_h_oriented(
            vram,
            vertices,
            drawing_area_x,
            drawing_area_y,
            shading,
            semi_transparency_mode,
            dithering_enabled,
            force_mask_bit,
            check_mask_bit,
        );
    } else {
        rasterize_line_v_oriented(
            vram,
            vertices,
            drawing_area_x,
            drawing_area_y,
            shading,
            semi_transparency_mode,
            dithering_enabled,
            force_mask_bit,
            check_mask_bit,
        );
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn rasterize_line_h_oriented(
    vram: &mut AlignedVram,
    mut v: [Vertex; 2],
    drawing_area_x: (i32, i32),
    drawing_area_y: (i32, i32),
    mut shading: LineShading,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    dithering_enabled: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    if v[0].x > v[1].x {
        v.swap(0, 1);

        if let LineShading::Gouraud(colors) = &mut shading {
            colors.swap(0, 1);
        }
    }

    let min_x = cmp::max(v[0].x, drawing_area_x.0);
    let max_x = cmp::min(v[1].x, drawing_area_x.1);
    let min_y = cmp::max(drawing_area_y.0, cmp::min(v[0].y, v[1].y));
    let max_y = cmp::min(drawing_area_y.1, cmp::max(v[0].y, v[1].y));

    let x_interval = v[1].x - v[0].x;

    let y_step = ((v[1].y - v[0].y) << 16) / x_interval;

    let (r_step, g_step, b_step) = match shading {
        LineShading::Flat(_) => (0.0, 0.0, 0.0),
        LineShading::Gouraud(colors) => gouraud_color_steps(colors, x_interval.into()),
    };

    let y_step_v = _mm_set1_epi32(4 * y_step);
    let r_step_v = _mm256_set1_pd(4.0 * r_step);
    let g_step_v = _mm256_set1_pd(4.0 * g_step);
    let b_step_v = _mm256_set1_pd(4.0 * b_step);

    let first_color = match shading {
        LineShading::Flat(color) | LineShading::Gouraud([color, _]) => color,
    };

    let first_y = (v[0].y << 16) | (1 << 15);
    let mut y =
        _mm_setr_epi32(first_y, first_y + y_step, first_y + 2 * y_step, first_y + 3 * y_step);
    let mut r = first_step_vector(first_color.r.into(), r_step);
    let mut g = first_step_vector(first_color.g.into(), g_step);
    let mut b = first_step_vector(first_color.b.into(), b_step);

    for x in (v[0].x..=v[1].x).step_by(4) {
        let xr = _mm_setr_epi32(x, x + 1, x + 2, x + 3);
        let yr = _mm_srai_epi32::<16>(y);

        let write_mask = _mm_and_si128(
            _mm_and_si128(
                _mm_cmpgt_epi32(xr, _mm_set1_epi32(min_x - 1)),
                _mm_cmplt_epi32(xr, _mm_set1_epi32(max_x + 1)),
            ),
            _mm_and_si128(
                _mm_cmpgt_epi32(yr, _mm_set1_epi32(min_y - 1)),
                _mm_cmplt_epi32(yr, _mm_set1_epi32(max_y + 1)),
            ),
        );

        if _mm_testz_si128(write_mask, _mm_set1_epi32(-1)) == 0 {
            let rr = round_pd_to_epi32(r);
            let gr = round_pd_to_epi32(g);
            let br = round_pd_to_epi32(b);

            rasterize_line_pixels(
                vram,
                xr,
                yr,
                rr,
                gr,
                br,
                write_mask,
                semi_transparency_mode,
                dithering_enabled,
                force_mask_bit,
                check_mask_bit,
            );
        }

        y = _mm_add_epi32(y, y_step_v);
        r = _mm256_add_pd(r, r_step_v);
        g = _mm256_add_pd(g, g_step_v);
        b = _mm256_add_pd(b, b_step_v);
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn rasterize_line_v_oriented(
    vram: &mut AlignedVram,
    mut v: [Vertex; 2],
    drawing_area_x: (i32, i32),
    drawing_area_y: (i32, i32),
    mut shading: LineShading,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    dithering_enabled: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    if v[0].y > v[1].y {
        v.swap(0, 1);

        if let LineShading::Gouraud(colors) = &mut shading {
            colors.swap(0, 1);
        }
    }

    let min_x = cmp::max(drawing_area_x.0, cmp::min(v[0].x, v[1].x));
    let max_x = cmp::min(drawing_area_x.1, cmp::max(v[0].x, v[1].x));
    let min_y = cmp::max(v[0].y, drawing_area_y.0);
    let max_y = cmp::min(v[1].y, drawing_area_y.1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    let y_interval = v[1].y - v[0].y;

    let x_step = ((v[1].x - v[0].x) << 16) / y_interval;

    let (r_step, g_step, b_step) = match shading {
        LineShading::Flat(_) => (0.0, 0.0, 0.0),
        LineShading::Gouraud(colors) => gouraud_color_steps(colors, y_interval.into()),
    };

    let x_step_v = _mm_set1_epi32(4 * x_step);
    let r_step_v = _mm256_set1_pd(4.0 * r_step);
    let g_step_v = _mm256_set1_pd(4.0 * g_step);
    let b_step_v = _mm256_set1_pd(4.0 * b_step);

    let first_color = match shading {
        LineShading::Flat(color) | LineShading::Gouraud([color, _]) => color,
    };

    let first_x = (v[0].x << 16) | (1 << 15);
    let mut x =
        _mm_setr_epi32(first_x, first_x + x_step, first_x + 2 * x_step, first_x + 3 * x_step);
    let mut r = first_step_vector(first_color.r.into(), r_step);
    let mut g = first_step_vector(first_color.g.into(), g_step);
    let mut b = first_step_vector(first_color.b.into(), b_step);

    for y in (v[0].y..=v[1].y).step_by(4) {
        let xr = _mm_srai_epi32::<16>(x);
        let yr = _mm_setr_epi32(y, y + 1, y + 2, y + 3);

        let write_mask = _mm_and_si128(
            _mm_and_si128(
                _mm_cmpgt_epi32(xr, _mm_set1_epi32(min_x - 1)),
                _mm_cmplt_epi32(xr, _mm_set1_epi32(max_x + 1)),
            ),
            _mm_and_si128(
                _mm_cmpgt_epi32(yr, _mm_set1_epi32(min_y - 1)),
                _mm_cmplt_epi32(yr, _mm_set1_epi32(max_y + 1)),
            ),
        );

        if _mm_testz_si128(write_mask, _mm_set1_epi32(-1)) == 0 {
            let rr = round_pd_to_epi32(r);
            let gr = round_pd_to_epi32(g);
            let br = round_pd_to_epi32(b);

            rasterize_line_pixels(
                vram,
                xr,
                yr,
                rr,
                gr,
                br,
                write_mask,
                semi_transparency_mode,
                dithering_enabled,
                force_mask_bit,
                check_mask_bit,
            );
        }

        x = _mm_add_epi32(x, x_step_v);
        r = _mm256_add_pd(r, r_step_v);
        g = _mm256_add_pd(g, g_step_v);
        b = _mm256_add_pd(b, b_step_v);
    }
}

#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn round_pd_to_epi32(pd: __m256d) -> __m128i {
    _mm256_cvtpd_epi32(_mm256_round_pd::<_MM_FROUND_TO_NEAREST_INT>(pd))
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn rasterize_line_pixels(
    vram: &mut AlignedVram,
    x: __m128i,
    y: __m128i,
    r: __m128i,
    g: __m128i,
    b: __m128i,
    write_mask: __m128i,
    semi_transparency_mode: Option<SemiTransparencyMode>,
    dithering_enabled: bool,
    force_mask_bit: bool,
    check_mask_bit: bool,
) {
    let forced_mask_bit = u16::from(force_mask_bit) << 15;

    let x_arr: [i32; 4] = mem::transmute(x);
    let y_arr: [i32; 4] = mem::transmute(y);
    let write_mask_arr: [i32; 4] = mem::transmute(write_mask);

    let r_arr: [i32; 4] = mem::transmute(r);
    let g_arr: [i32; 4] = mem::transmute(g);
    let b_arr: [i32; 4] = mem::transmute(b);

    for i in 0..4 {
        if write_mask_arr[i] == 0 {
            continue;
        }

        let vram_addr = (1024 * y_arr[i] + x_arr[i]) as usize;
        if check_mask_bit && vram[vram_addr] & 0x8000 != 0 {
            continue;
        }

        let (r, g, b) = match semi_transparency_mode {
            Some(mode) => {
                let existing = vram[vram_addr];
                let existing_r: i32 = ((existing & 0x1F) << 3).into();
                let existing_g: i32 = (((existing >> 5) & 0x1F) << 3).into();
                let existing_b: i32 = (((existing >> 10) & 0x1F) << 3).into();

                match mode {
                    SemiTransparencyMode::Average => (
                        (existing_r + r_arr[i]) >> 1,
                        (existing_g + g_arr[i]) >> 1,
                        (existing_b + b_arr[i]) >> 1,
                    ),
                    SemiTransparencyMode::Add => (
                        cmp::min(255, existing_r + r_arr[i]),
                        cmp::min(255, existing_g + g_arr[i]),
                        cmp::min(255, existing_b + b_arr[i]),
                    ),
                    SemiTransparencyMode::Subtract => (
                        cmp::max(0, existing_r - r_arr[i]),
                        cmp::max(0, existing_g - g_arr[i]),
                        cmp::max(0, existing_b - b_arr[i]),
                    ),
                    SemiTransparencyMode::AddQuarter => (
                        cmp::min(255, existing_r + (r_arr[i] >> 2)),
                        cmp::min(255, existing_g + (g_arr[i] >> 2)),
                        cmp::min(255, existing_b + (b_arr[i] >> 2)),
                    ),
                }
            }
            None => (r_arr[i], g_arr[i], b_arr[i]),
        };

        let (r, g, b) = if dithering_enabled {
            let dither_value =
                DITHER_TABLE[(y_arr[i] & 3) as usize][(3 - (x_arr[i] & 3)) as usize] as i8;

            (
                (r as u8).saturating_add_signed(dither_value),
                (g as u8).saturating_add_signed(dither_value),
                (b as u8).saturating_add_signed(dither_value),
            )
        } else {
            (r as u8, g as u8, b as u8)
        };

        vram[vram_addr] = u16::from(r >> 3)
            | (u16::from(g & 0xF8) << 2)
            | (u16::from(b & 0xF8) << 7)
            | forced_mask_bit;
    }
}

fn gouraud_color_steps([c0, c1]: [Color; 2], interval: f64) -> (f64, f64, f64) {
    (
        (f64::from(c1.r) - f64::from(c0.r)) / interval,
        (f64::from(c1.g) - f64::from(c0.g)) / interval,
        (f64::from(c1.b) - f64::from(c0.b)) / interval,
    )
}

#[target_feature(enable = "avx2", enable = "fma")]
unsafe fn first_step_vector(first: f64, step: f64) -> __m256d {
    _mm256_setr_pd(first, first + step, first + 2.0 * step, first + 3.0 * step)
}