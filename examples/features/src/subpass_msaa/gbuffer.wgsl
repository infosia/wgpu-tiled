struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct Uniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(in: VertexInput, @builtin(instance_index) instance: u32) -> VertexOutput {
    let spacing = 3.0;
    let half_grid = f32(GRID_SIZE - 1u) * spacing * 0.5;
    let col = instance % GRID_SIZE;
    let row = instance / GRID_SIZE;
    let offset = vec3<f32>(
        f32(col) * spacing - half_grid,
        0.0,
        f32(row) * spacing - half_grid,
    );

    let world_pos = in.position + offset;

    var out: VertexOutput;
    out.clip_position = uniforms.view_proj * vec4<f32>(world_pos, 1.0);
    out.color = in.color;
    out.normal = in.normal;
    return out;
}

const GRID_SIZE: u32 = 5u;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let ndotl = max(dot(n, normalize(vec3<f32>(0.4, 0.9, 0.1))), 0.0);
    let base = in.color * (0.2 + 0.8 * ndotl);
    return vec4<f32>(base, 1.0);
}
