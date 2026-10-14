@group(1) @binding(0) var block_textures: texture_2d_array<f32>;
@group(1) @binding(1) var block_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) layer: u32,
    @location(4) ao: f32,
    @location(5) sky: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) light: f32,
    @location(3) world_pos: vec3<f32>,
};

/// Minecraft's light curve at the default 50% brightness setting.
fn light_brightness(level: f32) -> f32 {
    let dim = level / (4.0 - 3.0 * level);
    let bright = 1.0 - pow(1.0 - dim, 4.0);
    return mix(dim, bright, 0.5);
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.layer = in.layer;
    out.world_pos = in.position;
    // Minecraft-style fixed face shading: top bright, bottom dark, sides in between.
    let n = in.normal;
    let shade = 1.0 * max(n.y, 0.0) + 0.5 * max(-n.y, 0.0) + 0.6 * abs(n.x) + 0.8 * abs(n.z);
    // Sky light is scaled by the time of day; a little ambient keeps caves from pitch black.
    let sky = light_brightness(in.sky) * globals.zenith.w;
    out.light = shade * in.ao * max(sky, 0.03);
    return out;
}

fn shaded(in: VertexOut) -> vec4<f32> {
    let color = textureSample(block_textures, block_sampler, in.uv, in.layer);
    let lit = color.rgb * in.light;
    return vec4<f32>(mix(lit, fog_color(in.world_pos), fog_factor(in.world_pos)), color.a);
}

@fragment
fn fs_solid(in: VertexOut) -> @location(0) vec4<f32> {
    let color = shaded(in);
    // Cut-out transparency (leaves).
    if color.a < 0.5 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_translucent(in: VertexOut) -> @location(0) vec4<f32> {
    return shaded(in);
}
