struct LightUniform {
    direction: vec4<f32>,
    color_intensity: vec4<f32>,
};

@group(0) @binding(0) @input_attachment_index(0)
var gbuffer_albedo: texture_2d<f32>;
@group(0) @binding(1) @input_attachment_index(1)
var gbuffer_normal: texture_2d<f32>;
@group(0) @binding(2)
var<uniform> light: LightUniform;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(i32(position.x), i32(position.y));
    let albedo = textureLoad(gbuffer_albedo, coord).rgb;
    let encoded_normal = textureLoad(gbuffer_normal, coord).rgb;
    let normal = normalize(encoded_normal * 2.0 - 1.0);
    let light_dir = normalize(light.direction.xyz);
    let diffuse = max(dot(normal, -light_dir), 0.0);
    let ambient = 0.12;
    let radiance = light.color_intensity.xyz * light.color_intensity.w;
    let lit = albedo * (ambient + diffuse) * radiance;
    return vec4<f32>(lit, 1.0);
}
