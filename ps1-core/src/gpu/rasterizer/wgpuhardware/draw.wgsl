var<push_constant> draw_settings: DrawSettings;

@vertex
fn vs_untextured(input: UntexturedVertex) -> UntexturedVertexOutput {
    let position = vram_position_to_vertex(input.position);

    let color = vec3f(input.color) / 255.0;

    return UntexturedVertexOutput(position, color);
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

@fragment
fn fs_untextured_average(input: UntexturedVertexOutput) -> SemiTransparentOutput {
    let color = vec4f(input.color, f32(draw_settings.force_mask_bit));
    let blend = vec4f(0.5);
    return SemiTransparentOutput(color, blend);
}

@fragment
fn fs_untextured_add_quarter(input: UntexturedVertexOutput) -> @location(0) vec4f {
    return vec4f(0.25 * input.color, f32(draw_settings.force_mask_bit));
}

@group(0) @binding(0)
var native_vram: texture_storage_2d<r32uint, read>;
@group(0) @binding(1)
var scaled_vram_copy: texture_storage_2d<rgba8unorm, read>;

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
    return vec4f(factor);
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
    return vec4f(factor);
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