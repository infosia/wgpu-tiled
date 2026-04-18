@group(0) @binding(0) 
var gbuffer_uint: texture_2d<u32>;

@fragment 
fn main() -> @location(0) @interpolate(flat) vec4<u32> {
    let _e4 = textureLoad(gbuffer_uint, vec2<i32>(3i, 4i));
    return _e4;
}
