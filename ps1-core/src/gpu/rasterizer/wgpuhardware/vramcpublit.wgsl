struct VramCpuBlitArgs {
    position: vec2u,
    size: vec2u,
}

@group(0) @binding(0)
var native_vram: texture_storage_2d<r32uint, read>;
@group(0) @binding(1)
var<storage, read_write> blit_buffer: array<u32>;

var<push_constant> args: VramCpuBlitArgs;

@compute
@workgroup_size(16, 16, 1)
fn vram_cpu_blit(@builtin(global_invocation_id) invocation: vec3u) {
    if invocation.x >= args.size.x || invocation.y >= args.size.y {
        return;
    }

    let position = (args.position + invocation.xy) & vec2u(1023, 511);
    let pixel = textureLoad(native_vram, position).r;

    let buffer_idx = invocation.y * args.size.x + invocation.x;
    blit_buffer[buffer_idx] = pixel;
}