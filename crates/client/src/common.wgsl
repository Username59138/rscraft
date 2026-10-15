// Shared by every shader: per-frame globals and the sky colour function, so
// that fog on the terrain blends seamlessly into the horizon.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // w = seconds since start
    camera_pos: vec4<f32>,
    // w = daylight, 0 at night .. 1 at day
    sun_dir: vec4<f32>,
    // w = ambient light multiplier
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    // rgb = colour, w = intensity
    sunset: vec4<f32>,
    // x = fog start, y = fog end, z = cloud height, w = cloud extent
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var color = mix(globals.horizon.rgb, globals.zenith.rgb, smoothstep(0.0, 0.45, up));
    // Sunrise/sunset glow around the sun, strongest at the horizon.
    let toward_sun = max(dot(dir, globals.sun_dir.xyz), 0.0);
    let glow = pow(toward_sun, 3.0) * (1.0 - up) * (1.0 - up) * globals.sunset.w;
    color = mix(color, globals.sunset.rgb, glow * 0.85);
    return color;
}

// Minecraft does its lighting maths on sRGB colours, but we blend in
// linear space. These convert so brightness and overlays look the same.

/// A brightness multiplier meant for sRGB colours, for use on linear ones.
fn gamma_light(light: f32) -> f32 {
    return pow(max(light, 0.0), 2.2);
}

/// Alpha for a dark overlay that darkens as much as `alpha` would in sRGB.
fn srgb_alpha(alpha: f32) -> f32 {
    return 1.0 - pow(1.0 - clamp(alpha, 0.0, 1.0), 2.2);
}

/// Minecraft's light curve at the default 50% brightness setting, for a sky
/// light level 0..1, scaled by the time of day.
fn light_brightness(level: f32) -> f32 {
    let dim = level / (4.0 - 3.0 * level);
    let bright = 1.0 - pow(1.0 - dim, 4.0);
    return mix(dim, bright, 0.5);
}

/// Cylindrical fog like Minecraft: horizontal distance only.
fn fog_factor(world_pos: vec3<f32>) -> f32 {
    let dist = distance(world_pos.xz, globals.camera_pos.xz);
    return smoothstep(globals.params.x, globals.params.y, dist);
}

fn fog_color(world_pos: vec3<f32>) -> vec3<f32> {
    return sky_color(normalize(world_pos - globals.camera_pos.xyz));
}
