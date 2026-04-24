@group(0) @binding(0) @input_attachment_index(3) 
var gbuffer_uint: texture_2d<u32>;

@fragment 
fn main() -> @location(0) @interpolate(flat) vec4<u32> {
    let _e1 = subpassLoad(gbuffer_uint);
    return _e1;
}
