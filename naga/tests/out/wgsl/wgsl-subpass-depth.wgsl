@group(0) @binding(0) 
var gbuffer_depth: texture_depth_2d;

@fragment 
fn main() -> @location(0) vec4<f32> {
    let depth = textureLoad(gbuffer_depth, vec2<i32>(4i, 8i));
    return vec4<f32>(depth, depth, depth, 1f);
}
