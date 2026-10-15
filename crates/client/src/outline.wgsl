struct Outline {
    // xy = screen size in pixels, z = line width in pixels
    screen: vec4<f32>,
};

@group(1) @binding(0) var<uniform> outline: Outline;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) other: vec3<f32>,
    @location(2) side: f32,
};

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4<f32> {
    // Pull the line 1% towards the camera so the half of it lying over the
    // block's own faces isn't hidden by them.
    let eye = globals.camera_pos.xyz;
    let position = eye + (in.position - eye) * 0.99;
    var clip = globals.view_proj * vec4<f32>(position, 1.0);
    var other = globals.view_proj * vec4<f32>(eye + (in.other - eye) * 0.99, 1.0);
    // Keep ends behind the camera from flipping the direction.
    clip.w = max(clip.w, 0.01);
    other.w = max(other.w, 0.01);

    // Work in pixels so the line has the same width at any distance.
    let half_screen = outline.screen.xy * 0.5;
    let a = clip.xy / clip.w * half_screen;
    let b = other.xy / other.w * half_screen;
    let dir = normalize(b - a + vec2<f32>(1e-6, 0.0));
    let normal = vec2<f32>(-dir.y, dir.x);
    // Out to the side, and past the end so corners close up.
    let offset_px = (normal * in.side - dir) * outline.screen.z * 0.5;
    clip = vec4<f32>(clip.xy + offset_px / half_screen * clip.w, clip.zw);
    return clip;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Vanilla's block outline is black at 40% opacity in sRGB; blending here
    // happens in linear space, where that darkening takes about 68%.
    return vec4<f32>(0.0, 0.0, 0.0, 0.68);
}
