@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return globals.view_proj * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Vanilla's block outline: black at 40% opacity.
    return vec4<f32>(0.0, 0.0, 0.0, 0.4);
}
