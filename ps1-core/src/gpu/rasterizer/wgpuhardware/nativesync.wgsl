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
var scaled_vram: texture_2d<f32>;
@group(0) @binding(1)
var scaled_vram_sampler: sampler;
@group(0) @binding(2)
var<uniform> resolution_scale: u32;

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4u {
    let texel = textureSample(scaled_vram, scaled_vram_sampler, position.xy / vec2f(1024.0, 512.0));

    let r = u32(round(texel.r * 255.0)) >> 3;
    let g = u32(round(texel.g * 255.0)) >> 3;
    let b = u32(round(texel.b * 255.0)) >> 3;
    let a = u32(round(texel.a));

    let texel_16bpp = r | (g << 5) | (b << 10) | (a << 15);
    return vec4u(texel_16bpp, 0, 0, 0);
}