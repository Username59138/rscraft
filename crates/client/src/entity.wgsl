@group(1) @binding(0) var skin: texture_2d<f32>;
@group(1) @binding(1) var skin_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) light: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: f32,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.world_pos = in.position;
    // Entities get soft directional shading instead of fixed per-face values.
    let n = normalize(in.normal);
    let shade = 0.6 + 0.4 * max(n.y, 0.0) + 0.2 * abs(n.z) - 0.1 * max(-n.y, 0.0);
    // Lit by the sky light where the entity is, like blocks.
    let sky = light_brightness(in.light) * globals.zenith.w;
    out.light = gamma_light(clamp(shade, 0.4, 1.0) * max(sky, 0.05));
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(skin, skin_sampler, in.uv);
    // The outer skin layer is mostly transparent.
    if color.a < 0.5 {
        discard;
    }
    let lit = color.rgb * in.light;
    return vec4<f32>(mix(lit, fog_color(in.world_pos), fog_factor(in.world_pos)), 1.0);
}
