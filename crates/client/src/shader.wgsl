struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    // x = start, y = end
    fog_range: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var block_textures: texture_2d_array<f32>;
@group(1) @binding(1) var block_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) layer: u32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) shade: f32,
    @location(3) fog: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.layer = in.layer;
    // Minecraft-style fixed face shading: top bright, bottom dark, sides in between.
    let n = in.normal;
    out.shade = 1.0 * max(n.y, 0.0) + 0.5 * max(-n.y, 0.0) + 0.6 * abs(n.x) + 0.8 * abs(n.z);
    // Cylindrical fog like Minecraft: horizontal distance only.
    let dist = distance(in.position.xz, globals.camera_pos.xz);
    out.fog = smoothstep(globals.fog_range.x, globals.fog_range.y, dist);
    return out;
}

fn shaded(in: VertexOut) -> vec4<f32> {
    let color = textureSample(block_textures, block_sampler, in.uv, in.layer);
    return vec4<f32>(mix(color.rgb * in.shade, globals.fog_color.rgb, in.fog), color.a);
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
