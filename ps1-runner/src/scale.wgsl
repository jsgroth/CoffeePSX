var<private> VERTICES: array<vec4f, 4> = array<vec4f, 4>(
    vec4f(-1.0, -1.0, 0.0, 1.0),
    vec4f(1.0, -1.0, 0.0, 1.0),
    vec4f(-1.0, 1.0, 0.0, 1.0),
    vec4f(1.0, 1.0, 0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4f {
    return VERTICES[vertex_index];
}

@group(0) @binding(0)
var texture_in: texture_2d<f32>;
@group(0) @binding(1)
var<uniform> width_scale: u32;
@group(0) @binding(2)
var<uniform> height_scale: u32;

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let grid_position = vec2u(floor(position.xy));
    let tex_position = vec2u(grid_position.x / width_scale, grid_position.y / height_scale);

    return textureLoad(texture_in, tex_position, 0);
}