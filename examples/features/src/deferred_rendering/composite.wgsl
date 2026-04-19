@group(0) @binding(0) @input_attachment_index(0)
var lit_hdr: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(i32(position.x), i32(position.y));
    let hdr = textureLoad(lit_hdr, coord).rgb;
    let tone_mapped = hdr / (hdr + vec3<f32>(1.0));
    let gamma_corrected = pow(tone_mapped, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(gamma_corrected, 1.0);
}
