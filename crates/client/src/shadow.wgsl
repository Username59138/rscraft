@group(1) @binding(0) var shadow_texture: texture_2d<f32>;
@group(1) @binding(1) var shadow_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) alpha: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.alpha = in.alpha;
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(shadow_texture, shadow_sampler, in.uv);
    // Fainter in the dark and in the fog, like the rest of the world.
    let alpha = texel.a * in.alpha * globals.zenith.w * (1.0 - fog_factor(in.world_pos));
    return vec4<f32>(texel.rgb, srgb_alpha(alpha));
}
