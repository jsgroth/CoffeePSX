struct UntexturedVertex {
    @location(0) position: vec2i,
    @location(1) color: vec3u,
}

struct UntexturedVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) color: vec3f,
}

@vertex
fn vs_untextured(input: UntexturedVertex) -> UntexturedVertexOutput {
    let x = f32(input.position.x - 512) / 512.0;
    let y = -f32(input.position.y - 256) / 256.0;
    let position = vec4f(x, y, 0.0, 1.0);

    let color = vec3f(input.color) / 255.0;

    return UntexturedVertexOutput(position, color);
}

struct DrawSettings {
    draw_area_top_left: vec2i,
    draw_area_bottom_right: vec2i,
    force_mask_bit: u32,
}

var<push_constant> draw_settings: DrawSettings;

@fragment
fn fs_untextured_opaque(input: UntexturedVertexOutput) -> @location(0) vec4f {
    if input.position.x < f32(draw_settings.draw_area_top_left.x)
        || input.position.x > f32(draw_settings.draw_area_bottom_right.x)
        || input.position.y < f32(draw_settings.draw_area_top_left.y)
        || input.position.y > f32(draw_settings.draw_area_bottom_right.y)
    {
        discard;
    }

    return vec4f(input.color, f32(draw_settings.force_mask_bit));
}