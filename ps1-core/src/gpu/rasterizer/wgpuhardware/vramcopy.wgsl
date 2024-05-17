struct VramCopyArgs {
    source: vec2u,
    destination: vec2u,
    size: vec2u,
    force_mask_bit: u32,
    check_mask_bit: u32,
}

@group(0) @binding(0)
var native_vram: texture_storage_2d<r32uint, read_write>;

var<push_constant> args: VramCopyArgs;

@compute
@workgroup_size(16, 1, 1)
fn vram_copy(@builtin(global_invocation_id) invocation: vec3u) {
    if invocation.x >= args.size.x {
        return;
    }

    let destination_x = (args.destination.x + invocation.x) & 1023;
    let destination_y = (args.destination.y + invocation.y) & 511;
    let destination = vec2u(destination_x, destination_y);

    if args.check_mask_bit != 0 {
        let existing_texel = textureLoad(native_vram, destination).r;
        if (existing_texel & 0x8000) != 0 {
            return;
        }
    }

    let source_x = (args.source.x + invocation.x) & 1023;
    let source_y = (args.source.y + invocation.y) & 511;
    let source = vec2u(source_x, source_y);

    let texel = textureLoad(native_vram, source).r | (args.force_mask_bit << 15);
    textureStore(native_vram, destination, vec4u(texel, 0, 0, 0));
}