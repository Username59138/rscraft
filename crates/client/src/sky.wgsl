// Layer 0 = sun, layer 1 = moon.
@group(1) @binding(0) var celestial: texture_2d_array<f32>;
@group(1) @binding(1) var celestial_sampler: sampler;

struct SkyOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    // One triangle covering the whole screen.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    let ndc = uv * 2.0 - 1.0;
    var out: SkyOut;
    out.position = vec4<f32>(ndc, 1.0, 1.0);
    out.ndc = ndc;
    return out;
}

/// Additive textured quad around `center` with angular half-size `size`.
fn celestial_body(dir: vec3<f32>, center: vec3<f32>, size: f32, layer: i32) -> vec3<f32> {
    let facing = dot(dir, center);
    if facing <= 0.0 {
        return vec3<f32>(0.0);
    }
    // The sun and moon orbit in the XY plane, so Z is a stable "right" axis.
    let right = vec3<f32>(0.0, 0.0, 1.0);
    let up = cross(center, right);
    let local = vec2<f32>(dot(dir, right), dot(dir, up)) / facing / size;
    if any(abs(local) > vec2<f32>(1.0)) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(celestial, celestial_sampler, local * 0.5 + 0.5, layer, 0.0).rgb;
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - globals.camera_pos.xyz);
    let daylight = globals.sun_dir.w;
    let sun = globals.sun_dir.xyz;

    var color = sky_color(dir);
    // Hide what's below the horizon, like the terrain would.
    let above = smoothstep(-0.02, 0.02, dir.y);

    // Stars come out only once the sun is well below the horizon.
    let cell = floor(dir * 300.0);
    if hash3(cell) > 0.9975 {
        color += vec3<f32>(0.8) * (1.0 - smoothstep(0.0, 0.4, daylight)) * above * hash3(cell + 1.0);
    }

    color += celestial_body(dir, sun, 0.3, 0) * above;
    color += celestial_body(dir, -sun, 0.2, 1) * above * mix(1.0, 0.4, daylight);
    return vec4<f32>(color, 1.0);
}
