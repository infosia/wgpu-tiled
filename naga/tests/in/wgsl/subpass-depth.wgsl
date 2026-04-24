@group(0) @binding(0) @input_attachment_index(1)
var gbuffer_depth: texture_depth_2d;

@fragment
fn main() -> @location(0) vec4<f32> {
    let depth = subpassLoad(gbuffer_depth);
    return vec4<f32>(depth, depth, depth, 1.0);
}
