// Must match VramSyncVertex in Rust
struct Vertex {
    @location(0) position: vec2i,
}

@vertex
fn vs_main(input: Vertex) -> @builtin(position) vec4f {
    let x = f32(input.position.x - 512) / 512.0;
    let y = -f32(input.position.y - 256) / 256.0;
    return vec4f(x, y, 0.0, 1.0);
}

@group(0) @binding(0)
var native_vram: texture_storage_2d<r32uint, read>;
@group(0) @binding(1)
var<uniform> resolution_scale: u32;

@fragment
fn native_to_scaled(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let native_position = vec2u(position.xy) / resolution_scale;
    let texel = textureLoad(native_vram, native_position).r;
    let r = f32(texel & 0x1F) / 31.0;
    let g = f32((texel >> 5) & 0x1F) / 31.0;
    let b = f32((texel >> 10) & 0x1F) / 31.0;
    let a = f32((texel >> 15) & 1);

    return vec4f(r, g, b, a);
}