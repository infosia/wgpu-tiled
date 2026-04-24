@group(0) @binding(0) @input_attachment_index(0)
var gbuffer_albedo: texture_2d<f32>;

@group(0) @binding(1) @input_attachment_index(1)
var gbuffer_normal: texture_2d<f32>;

@group(0) @binding(2) @input_attachment_index(2)
var gbuffer_material: texture_2d<f32>;

@fragment
fn main() -> @location(0) vec4<f32> {
    let albedo = subpassLoad(gbuffer_albedo);
    let normal = subpassLoad(gbuffer_normal);
    let material = subpassLoad(gbuffer_material);
    return (albedo + normal + material) / 3.0;
}
