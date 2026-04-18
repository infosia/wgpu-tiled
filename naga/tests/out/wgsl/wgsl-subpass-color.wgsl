@group(0) @binding(0) 
var gbuffer_color: texture_2d<f32>;

@fragment 
fn main() -> @location(0) vec4<f32> {
    let _e4 = textureLoad(gbuffer_color, vec2<i32>(1i, 2i));
    return _e4;
}
