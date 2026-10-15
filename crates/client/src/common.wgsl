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
    // x = 1 when the camera is under water, y = water fog end,
    // zw = underwater overlay texture offset
    water: vec4<f32>,
    // rgb = water fog colour (sRGB), w = sky light at the eyes (0..1)
    water_color: vec4<f32>,
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

/// Minecraft's lighting for entities and items: two fixed directional
/// lights from above (its minecraft_mix_light), so sides and bottoms are
/// clearly darker than tops.
fn entity_shade(normal: vec3<f32>) -> f32 {
    let light0 = normalize(vec3<f32>(0.2, 1.0, -0.7));
    let light1 = normalize(vec3<f32>(-0.2, 1.0, 0.7));
    let n = normalize(normal);
    return min(1.0, (max(dot(light0, n), 0.0) + max(dot(light1, n), 0.0)) * 0.6 + 0.4);
}

/// Under water the fog is a dark blue, dimmed at night.
fn water_fog_color() -> vec3<f32> {
    return pow(globals.water_color.rgb, vec3<f32>(2.2)) * globals.zenith.w;
}

/// Cylindrical fog like Minecraft: horizontal distance only. Under water it
/// is spherical and much closer, running linearly from -8 to the fog end.
fn fog_factor(world_pos: vec3<f32>) -> f32 {
    if globals.water.x > 0.5 {
        let dist = distance(world_pos, globals.camera_pos.xyz);
        return clamp((dist + 8.0) / (globals.water.y + 8.0), 0.0, 1.0);
    }
    let dist = distance(world_pos.xz, globals.camera_pos.xz);
    return smoothstep(globals.params.x, globals.params.y, dist);
}

fn fog_color(world_pos: vec3<f32>) -> vec3<f32> {
    if globals.water.x > 0.5 {
        return water_fog_color();
    }
    return sky_color(normalize(world_pos - globals.camera_pos.xyz));
}
