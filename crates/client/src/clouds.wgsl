struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) shade: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) shade: f32,
};

// Must match CLOUD_SPEED in clouds.rs.
const CLOUD_SPEED: f32 = 0.6;

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let world = in.position + vec3<f32>(globals.camera_pos.w * CLOUD_SPEED, 0.0, 0.0);
    var out: VertexOut;
    out.clip_position = globals.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.shade = in.shade;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let daylight = globals.sun_dir.w;
    var color = mix(vec3<f32>(0.08, 0.08, 0.11), vec3<f32>(1.0), daylight);
    color = color * in.shade + globals.sunset.rgb * globals.sunset.w * 0.4;
    // Clouds fade out towards the edge of the cloud range.
    let dist = distance(in.world_pos.xz, globals.camera_pos.xz);
    let fade = 1.0 - smoothstep(globals.params.w * 0.4, globals.params.w, dist);
    // Keep invisible cloud edges out of the depth buffer.
    if fade <= 0.01 {
        discard;
    }
    return vec4<f32>(color, 0.8 * fade);
}
