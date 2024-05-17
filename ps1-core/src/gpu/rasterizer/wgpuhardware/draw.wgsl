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

struct DrawSettings {
    draw_area_top_left: vec2f,
    draw_area_bottom_right: vec2f,
    force_mask_bit: u32,
}

var<push_constant> draw_settings: DrawSettings;

fn is_out_of_bounds(position: vec2f) -> bool {
    return position.x < draw_settings.draw_area_top_left.x
       || position.x > draw_settings.draw_area_bottom_right.x
       || position.y < draw_settings.draw_area_top_left.y
       || position.y > draw_settings.draw_area_bottom_right.y;
}

@fragment
fn fs_untextured_opaque(input: UntexturedVertexOutput) -> @location(0) vec4f {
    if is_out_of_bounds(input.position.xy) {
        discard;
    }

    return vec4f(input.color, f32(draw_settings.force_mask_bit));
}

struct SemiTransparentOutput {
    @location(0) color: vec4f,
    @location(0) @second_blend_source blend: vec4f,
}

@fragment
fn fs_untextured_average(input: UntexturedVertexOutput) -> SemiTransparentOutput {
    if is_out_of_bounds(input.position.xy) {
        discard;
    }

    let color = vec4f(input.color, f32(draw_settings.force_mask_bit));
    let blend = vec4f(0.5, 0.5, 0.5, 0.5);
    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_untextured_add_quarter(input: UntexturedVertexOutput) -> SemiTransparentOutput {
    if is_out_of_bounds(input.position.xy) {
        discard;
    }

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

fn sample_texture(input: TexturedVertexOutput) -> vec4f {
    if is_out_of_bounds(input.position.xy) {
        discard;
    }

    let uv = (vec2u(floor(input.uv)) & ~input.tex_window_mask)
        | (input.tex_window_offset & input.tex_window_mask);

    var color: u32;
    if input.color_depth == TEXTURE_4BPP {
        color = read_4bpp_texture(uv, input.texpage, input.clut);
    } else if input.color_depth == TEXTURE_8BPP {
        color = read_8bpp_texture(uv, input.texpage, input.clut);
    } else {
        color = read_15bpp_texture(uv, input.texpage);
    }

    if color == 0 {
        discard;
    }

    var r = f32((color & 0x1F) << 3) / 255.0;
    var g = f32(((color >> 5) & 0x1F) << 3) / 255.0;
    var b = f32(((color >> 10) & 0x1F) << 3) / 255.0;

    if input.modulated != 0 {
        r *= 1.9921875 * input.color.r;
        g *= 1.9921875 * input.color.g;
        b *= 1.9921875 * input.color.b;
    }

    let a = f32((color >> 15) & 1);
    return vec4f(r, g, b, a);
}

@fragment
fn fs_textured_opaque(input: TexturedVertexOutput) -> @location(0) vec4f {
    var color = sample_texture(input);
    color.a = max(color.a, f32(draw_settings.force_mask_bit));
    return color;
}

@fragment
fn fs_textured_average(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend_factor = select(1.0, 0.5, texel.a != 0.0);
    let blend = vec4f(blend_factor, blend_factor, blend_factor, blend_factor);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_add(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture(input);
    let color = vec4f(texel.rgb, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend_factor = select(0.0, 1.0, texel.a != 0.0);
    let blend = vec4f(blend_factor, blend_factor, blend_factor, blend_factor);

    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_textured_subtract_opaque_texels(input: TexturedVertexOutput) -> @location(0) vec4f {
    let color = sample_texture(input);
    if color.a != 0.0 {
        discard;
    }

    return vec4f(color.rgb, f32(draw_settings.force_mask_bit));
}

@fragment
fn fs_textured_subtract_transparent_texels(input: TexturedVertexOutput) -> @location(0) vec4f {
    let color = sample_texture(input);
    if color.a == 0.0 {
        discard;
    }

    return vec4f(color.rgb, 1.0);
}

@fragment
fn fs_textured_add_quarter(input: TexturedVertexOutput) -> SemiTransparentOutput {
    let texel = sample_texture(input);
    let premultiplied_color = select(texel.rgb, texel.rgb * 0.25, texel.a != 0.0);
    let color = vec4f(premultiplied_color, max(texel.a, f32(draw_settings.force_mask_bit)));

    let blend_factor = select(0.0, 1.0, texel.a != 0.0);
    let blend = vec4f(blend_factor, blend_factor, blend_factor, blend_factor);

    return SemiTransparentOutput(color, blend);
}