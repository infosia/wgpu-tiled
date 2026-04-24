@group(0) @binding(0) @input_attachment_index(3)
var gbuffer_uint: texture_2d<u32>;

@fragment
fn main() -> @location(0) vec4<u32> {
    return subpassLoad(gbuffer_uint);
}
