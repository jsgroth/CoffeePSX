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

@vertex
fn vs_textured(input: TexturedVertex) -> TexturedVertexOutput {
    let position = vram_position_to_vertex(input.position);
    let color = vec3f(input.color) / 255.0;
    let uv = vec2f(input.uv);

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
    let uv = (input_uv & ~tex_window_mask)
        | (tex_window_offset & tex_window_mask);

    var color: u32;
    switch (color_depth) {
        case 0u: {
            color = read_4bpp_texture(uv, texpage, clut);
        }
        case 1u: {
            color = read_8bpp_texture(uv, texpage, clut);
        }
        case 2u: {
            color = read_15bpp_texture(uv, texpage);
        }
        default: {
            discard;
        }
    }

    if color == 0 {
        discard;
    }

    var r = f32((color & 0x1F) << 3) / 255.0;
    var g = f32(((color >> 5) & 0x1F) << 3) / 255.0;
    var b = f32(((color >> 10) & 0x1F) << 3) / 255.0;

    if modulated != 0 {
        r *= 1.9921875 * input_color.r;
        g *= 1.9921875 * input_color.g;
        b *= 1.9921875 * input_color.b;
    }

    let a = f32((color >> 15) & 1);
    return vec4f(r, g, b, a);
}

fn sample_texture_triangle(input: TexturedVertexOutput) -> vec4f {
    return sample_texture(
        input.color,
        vec2u(floor(input.uv)),
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