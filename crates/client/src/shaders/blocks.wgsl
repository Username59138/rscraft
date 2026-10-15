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

fn vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, ao: f32, sky_light: f32, shade: f32) -> VertexOut {
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.layer = layer;
    out.world_pos = position;
    // Sky light is scaled by the time of day; a little ambient keeps caves from pitch black.
    let sky = light_brightness(sky_light) * globals.zenith.w;
    out.light = gamma_light(shade * ao * max(sky, 0.05));
    return out;
}

/// Minecraft-style fixed face shading: top bright, bottom dark, sides in between.
fn face_shade(n: vec3<f32>) -> f32 {
    return 1.0 * max(n.y, 0.0) + 0.5 * max(-n.y, 0.0) + 0.6 * abs(n.x) + 0.8 * abs(n.z);
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    return vertex(in.position, in.uv, in.layer, in.ao, in.sky, face_shade(in.normal));
}

/// Blocks drawn as items (dropped, held): lit like entities, not like terrain.
@vertex
fn vs_item(in: VertexIn) -> VertexOut {
    return vertex(in.position, in.uv, in.layer, in.ao, in.sky, entity_shade(in.normal));
}

/// The compact vertices of terrain sections (`TerrainVertex`).
struct TerrainIn {
    /// Section-local position in 1/2048 blocks, then the texture layer.
    @location(0) position: vec4<u32>,
    @location(1) uv: vec2<f32>,
    /// Face shade, ambient occlusion, sky light.
    @location(2) light: vec4<f32>,
    /// Per instance: the section's origin in blocks.
    @location(3) origin: vec4<i32>,
};

@vertex
fn vs_terrain(in: TerrainIn) -> VertexOut {
    let position = vec3<f32>(in.origin.xyz) + vec3<f32>(in.position.xyz) / 2048.0;
    return vertex(position, in.uv, in.position.w, in.light.y, in.light.z, in.light.x);
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
fn fs_crack(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(block_textures, block_sampler, in.uv, in.layer);
    if color.a < 0.1 {
        discard;
    }
    // Vanilla multiplies the block by 2 × crack on sRGB values; the blend
    // here gives 2 × output × block in linear space, so convert the factor.
    let srgb = pow(color.rgb, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(pow(2.0 * srgb, vec3<f32>(2.2)) * 0.5, 1.0);
}

@fragment
fn fs_translucent(in: VertexOut) -> @location(0) vec4<f32> {
    return shaded(in);
}
