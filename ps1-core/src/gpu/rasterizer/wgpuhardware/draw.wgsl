struct DrawSettings {
    force_mask_bit: u32,
    resolution_scale: u32,
}

var<push_constant> draw_settings: DrawSettings;

struct UntexturedVertex {
    @location(0) position: vec2i,
    @location(1) color: vec3u,
}

struct UntexturedVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) color: vec3f,
}

fn vram_position_to_vertex(position: vec2i) -> vec4f {
    let x = f32(position.x - 512) / 512.0;
    let y = -f32(position.y - 256) / 256.0;
    return vec4f(x, y, 0.0, 1.0);
}

@vertex
fn vs_untextured(input: UntexturedVertex) -> UntexturedVertexOutput {
    let position = vram_position_to_vertex(input.position);

    let color = vec3f(input.color) / 255.0;

    return UntexturedVertexOutput(position, color);
}

const TEXTURE_4BPP: u32 = 0;
const TEXTURE_8BPP: u32 = 1;
const TEXTURE_15BPP: u32 = 2;

struct TexturedVertex {
    @location(0) position: vec2i,
    @location(1) color: vec3u,
    @location(2) uv: vec2u,
    @location(3) texpage: vec2u,
    @location(4) tex_window_mask: vec2u,
    @location(5) tex_window_offset: vec2u,
    @location(6) clut: vec2u,
    @location(7) color_depth: u32,
    @location(8) modulated: u32,
    @location(9) other_positions: vec4i,
    @location(10) other_uv: vec4u,
}

struct TexturedRectVertex {
    @location(0) position: vec2i,
    @location(1) color: vec3u,
    @location(2) texpage: vec2u,
    @location(3) tex_window_mask: vec2u,
    @location(4) tex_window_offset: vec2u,
    @location(5) clut: vec2u,
    @location(6) color_depth: u32,
    @location(7) modulated: u32,
    @location(8) base_position: vec2i,
    @location(9) base_uv: vec2u,
}

struct TexturedVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) color: vec3f,
    @location(1) uv: vec2f,
    @location(2) texpage: vec2u,
    @location(3) tex_window_mask: vec2u,
    @location(4) tex_window_offset: vec2u,
    @location(5) clut: vec2u,
    @location(6) color_depth: u32,
    @location(7) modulated: u32,
    @location(8) uv_round_direction: vec2i,
}

struct TexturedRectVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) color: vec3f,
    @location(1) texpage: vec2u,
    @location(2) tex_window_mask: vec2u,
    @location(3) tex_window_offset: vec2u,
    @location(4) clut: vec2u,
    @location(5) color_depth: u32,
    @location(6) modulated: u32,
    @location(7) base_position: vec2i,
    @location(8) base_uv: vec2u,
}

fn compute_dx(component: vec3i, v0: vec2i, v1: vec2i, v2: vec2i) -> i32 {
    return component[0] * (v1.y - v2.y)
        + component[1] * (v2.y - v0.y)
        + component[2] * (v0.y - v1.y);
}

fn compute_dy(component: vec3i, v0: vec2i, v1: vec2i, v2: vec2i) -> i32 {
    return component[0] * (v2.x - v1.x)
        + component[1] * (v0.x - v2.x)
        + component[2] * (v1.x - v0.x);
}

fn compute_uv_round_direction(vert0: vec2i, uv0: vec2u, other_positions: vec4i, other_uv: vec4u) -> vec2i {
    let vert1 = other_positions.xy;
    let vert2 = other_positions.zw;
    let uv1 = other_uv.xy;
    let uv2 = other_uv.zw;

    let cpz_sign = sign((vert1.x - vert0.x) * (vert2.y - vert0.y) - (vert1.y - vert0.y) * (vert2.x - vert0.x));

    let u = vec3i(vec3u(uv0.x, uv1.x, uv2.x));
    let v = vec3i(vec3u(uv0.y, uv1.y, uv2.y));

    let du_dx = cpz_sign * compute_dx(u, vert0, vert1, vert2);
    let du_dy = cpz_sign * compute_dy(u, vert0, vert1, vert2);
    let dv_dx = cpz_sign * compute_dx(v, vert0, vert1, vert2);
    let dv_dy = cpz_sign * compute_dy(v, vert0, vert1, vert2);

    let u_sign = sign(-du_dx - du_dy);
    let v_sign = sign(-dv_dx - dv_dy);
    return vec2i(u_sign, v_sign);
}

@vertex
fn vs_textured(input: TexturedVertex) -> TexturedVertexOutput {
    let position = vram_position_to_vertex(input.position);
    let color = vec3f(input.color) / 255.0;
    let uv = vec2f(input.uv);

    let uv_round_direction = compute_uv_round_direction(
        input.position, input.uv, input.other_positions, input.other_uv,
    );

    return TexturedVertexOutput(
        position,
        color,
        uv,
        input.texpage,
        input.tex_window_mask,
        input.tex_window_offset,
        input.clut,
        input.color_depth,
        input.modulated,
        uv_round_direction,
    );
}

@vertex
fn vs_textured_rect(input: TexturedRectVertex) -> TexturedRectVertexOutput {
    let position = vram_position_to_vertex(input.position);
    let color = vec3f(input.color) / 255.0;

    return TexturedRectVertexOutput(
        position,
        color,
        input.texpage,
        input.tex_window_mask,
        input.tex_window_offset,
        input.clut,
        input.color_depth,
        input.modulated,
        input.base_position,
        input.base_uv,
    );
}

@fragment
fn fs_untextured_opaque(input: UntexturedVertexOutput) -> @location(0) vec4f {
    return vec4f(input.color, f32(draw_settings.force_mask_bit));
}

struct SemiTransparentOutput {
    @location(0) color: vec4f,
    @location(0) @second_blend_source blend: vec4f,
}

@fragment
fn fs_untextured_average(input: UntexturedVertexOutput) -> SemiTransparentOutput {
    let color = vec4f(input.color, f32(draw_settings.force_mask_bit));
    let blend = vec4f(0.5, 0.5, 0.5, 0.5);
    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_untextured_add_quarter(input: UntexturedVertexOutput) -> SemiTransparentOutput {
    let color = vec4f(input.color, f32(draw_settings.force_mask_bit));
    let blend = vec4f(0.25, 0.25, 0.25, 0.25);
    return SemiTransparentOutput(color, blend);
}

@group(0) @binding(0)
var native_vram: texture_storage_2d<r32uint, read>;
@group(0) @binding(1)
var scaled_vram_copy: texture_storage_2d<rgba8unorm, read>;

fn read_4bpp_texture(uv: vec2u, texpage: vec2u, clut: vec2u) -> u32 {
    let x = texpage.x + (uv.x >> 2);
    let y = texpage.y + uv.y;
    let shift = (uv.x & 3) << 2;

    let texel = textureLoad(native_vram, vec2u(x, y)).r;
    let clut_index = (texel >> shift) & 15;
    let clut_position = clut + vec2u(clut_index, 0);
    return textureLoad(native_vram, clut_position).r;
}

fn read_8bpp_texture(uv: vec2u, texpage: vec2u, clut: vec2u) -> u32 {
    let x = (texpage.x + (uv.x >> 1)) & 1023;
    let y = texpage.y + uv.y;
    let shift = (uv.x & 1) << 3;

    let texel = textureLoad(native_vram, vec2u(x, y)).r;
    let clut_index = (texel >> shift) & 255;
    let clut_x = (clut.x + clut_index) & 1023;
    return textureLoad(native_vram, vec2u(clut_x, clut.y)).r;
}

fn read_15bpp_texture(uv: vec2u, texpage: vec2u) -> u32 {
    let x = (texpage.x + uv.x) & 1023;
    let y = texpage.y + uv.y;
    return textureLoad(native_vram, vec2u(x, y)).r;
}

fn apply_texture_window(uv: vec2u, mask: vec2u, offset: vec2u) -> vec2u {
    return (uv & ~mask) | (offset & mask);
}

fn apply_modulation(texel: vec4f, input_color: vec3f) -> vec4f {
    let rgb = floor(texel.rgb * 1.9921875 * input_color * 255.0) / 255.0;
    return vec4f(rgb, texel.a);
}

fn sample_texture(
    input_color: vec3f,
    input_uv: vec2u,
    texpage: vec2u,
    tex_window_mask: vec2u,
    tex_window_offset: vec2u,
    clut: vec2u,
    color_depth: u32,
    modulated: u32,
) -> vec4f {
    let uv = apply_texture_window(input_uv, tex_window_mask, tex_window_offset);

    var color: u32;
    switch (color_depth) {
        case 0u: {
            color = read_4bpp_texture(uv, texpage, clut);
        }
        case 1u: {
            color = read_8bpp_texture(uv, texpage, clut);
        }
        default: {
            discard;
        }
    }

    if color == 0 {
        discard;
    }

    let r = f32(color & 0x1F) / 31.0;
    let g = f32((color >> 5) & 0x1F) / 31.0;
    let b = f32((color >> 10) & 0x1F) / 31.0;
    let a = f32((color >> 15) & 1);
    var texel = vec4f(r, g, b, a);

    if modulated != 0 {
        texel = apply_modulation(texel, input_color);
    }

    return texel;
}

fn sample_15bpp_texture(
    input_color: vec3f,
    scaled_uv: vec2u,
    texpage: vec2u,
    modulated: u32,
) -> vec4f {
    let scale = draw_settings.resolution_scale;
    let x = (scale * texpage.x + scaled_uv.x) % (scale * 1024);
    let y = scale * texpage.y + scaled_uv.y;
    var texel = textureLoad(scaled_vram_copy, vec2u(x, y));

    if texel.r == 0.0 && texel.g == 0.0 && texel.b == 0.0 && texel.a == 0.0 {
        discard;
    }

    if modulated != 0 {
        texel = apply_modulation(texel, input_color);
    }

    return texel;
}

fn round_uv(uv: vec2f, round_direction: vec2i) -> vec2u {
    let u = select(floor(uv.x), ceil(uv.x), round_direction.x >= 0);
    let v = select(floor(uv.y), ceil(uv.y), round_direction.y >= 0);
    return vec2u(u32(u), u32(v));
}

fn sample_texture_triangle(input: TexturedVertexOutput) -> vec4f {
    if input.color_depth == TEXTURE_15BPP {
        let fractional_uv = fract(input.uv);
        let integral_uv = vec2u(input.uv);
        let masked_uv = apply_texture_window(integral_uv, input.tex_window_mask, input.tex_window_offset);

        let scale = draw_settings.resolution_scale;
        let fractional_uv_scaled = round_uv(f32(scale) * fractional_uv, input.uv_round_direction);
        let scaled_uv = scale * masked_uv + fractional_uv_scaled;

        return sample_15bpp_texture(input.color, scaled_uv, input.texpage, input.modulated);
    }

    let uv = round_uv(input.uv, input.uv_round_direction);

    return sample_texture(
        input.color,
        uv,
        input.texpage,
        input.tex_window_mask,
        input.tex_window_offset,
        input.clut,
        input.color_depth,
        input.modulated,
    );
}

fn sample_texture_rect(input: TexturedRectVertexOutput) -> vec4f {
    let uv_offset = (vec2i(input.position.xy) - i32(draw_settings.resolution_scale) * input.base_position)
        / i32(draw_settings.resolution_scale);
    let uv = (input.base_uv + vec2u(uv_offset)) & vec2u(255, 255);

    if input.color_depth == TEXTURE_15BPP {
        let scale = draw_settings.resolution_scale;
        let scaled_uv = scale * uv + (vec2u(input.position.xy) % scale);
        return sample_15bpp_texture(input.color, scaled_uv, input.texpage, input.modulated);
    }

    return sample_texture(
        input.color,
        uv,
        input.texpage,
        input.tex_window_mask,
        input.tex_window_offset,
        input.clut,
        input.color_depth,
        input.modulated,
    );
}

@fragment
fn fs_textured_opaque(input: TexturedVertexOutput) -> @location(0) vec4f {
    var color = sample_texture_triangle(input);
    color.a = max(color.a, f32(draw_settings.force_mask_bit));
    return color;
}

@fragment
fn fs_textured_rect_opaque(input: TexturedRectVertexOutput) -> @location(0) vec4f {
    var color = sample_texture_rect(input);
    color.a = max(color.a, f32(draw_settings.force_mask_bit));
    return color;
}

fn average_blend(texel: vec4f) -> vec4f {
    let factor = select(1.0, 0.5, texel.a != 0.0);
    return vec4f(factor, factor, factor, factor);
}

@fragment
fn fs_textured_average(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_triangle(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = average_blend(texel);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_rect_average(input: TexturedRectVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_rect(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = average_blend(texel);

    return SemiTransparentOutput(color, blend);
}

fn additive_blend(texel: vec4f) -> vec4f {
    let factor = select(0.0, 1.0, texel.a != 0.0);
    return vec4f(factor, factor, factor, factor);
}

@fragment
fn fs_textured_add(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_triangle(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = additive_blend(texel);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_rect_add(input: TexturedRectVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_rect(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = additive_blend(texel);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_subtract_opaque_texels(input: TexturedVertexOutput) -> @location(0) vec4f {
    let color = sample_texture_triangle(input);
    if color.a != 0.0 {
        discard;
    }

    return vec4f(color.rgb, f32(draw_settings.force_mask_bit));
}

@fragment
fn fs_textured_subtract_transparent_texels(input: TexturedVertexOutput) -> @location(0) vec4f {
    let color = sample_texture_triangle(input);
    if color.a == 0.0 {
        discard;
    }

    return vec4f(color.rgb, 1.0);
}

@fragment
fn fs_textured_rect_subtract_opaque_texels(input: TexturedRectVertexOutput) -> @location(0) vec4f {
    let color = sample_texture_rect(input);
    if color.a != 0.0 {
        discard;
    }

    return vec4f(color.rgb, f32(draw_settings.force_mask_bit));
}

@fragment
fn fs_textured_rect_subtract_transparent_texels(input: TexturedRectVertexOutput) -> @location(0) vec4f {
    let color = sample_texture_rect(input);
    if color.a == 0.0 {
        discard;
    }

    return vec4f(color.rgb, 1.0);
}

fn add_quarter_premultiply(texel: vec4f) -> vec3f {
    return select(texel.rgb, texel.rgb * 0.25, texel.a != 0.0);
}

@fragment
fn fs_textured_add_quarter(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_triangle(input);
    let premultiplied_color = add_quarter_premultiply(texel);
    let color = vec4f(premultiplied_color, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = additive_blend(texel);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_rect_add_quarter(input: TexturedRectVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture_rect(input);
    let premultiplied_color = add_quarter_premultiply(texel);
    let color = vec4f(premultiplied_color, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend = additive_blend(texel);

    return SemiTransparentOutput(color, blend);
}