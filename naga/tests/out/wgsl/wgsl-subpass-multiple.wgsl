@group(0) @binding(0) @input_attachment_index(0) 
var gbuffer_albedo: texture_2d<f32>;
@group(0) @binding(1) @input_attachment_index(1) 
var gbuffer_normal: texture_2d<f32>;
@group(0) @binding(2) @input_attachment_index(2) 
var gbuffer_material: texture_2d<f32>;

@fragment 
fn main() -> @location(0) vec4<f32> {
    let albedo = textureLoad(gbuffer_albedo, vec2<i32>(0i, 0i));
    let normal = textureLoad(gbuffer_normal, vec2<i32>(0i, 0i));
    let material = textureLoad(gbuffer_material, vec2<i32>(0i, 0i));
    return (((albedo + normal) + material) / vec4(3f));
}
