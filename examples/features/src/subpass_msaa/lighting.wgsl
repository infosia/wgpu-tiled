@group(0) @binding(0) var albedo_ms: subpass_input_multisampled<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index & 2u) * 2 - 1);
    out.position = vec4<f32>(x, -y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(sample_index) sid: u32) -> @location(0) vec4<f32> {
    let albedo = subpassLoad(albedo_ms).rgb;
    let shade = 0.85 + 0.15 * albedo.g;
    let sample_bias = 1.0 - f32(sid) * 0.08;
    return vec4<f32>(albedo * shade * sample_bias, 1.0);
}
