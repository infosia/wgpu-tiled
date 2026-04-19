struct SceneUniform {
    view_proj: mat4x4<f32>,
    model_0: mat4x4<f32>,
    model_1: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniform;
@group(0) @binding(1)
var base_color_texture: texture_2d<f32>;
@group(0) @binding(2)
var base_color_sampler: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

fn instance_model(instance_index: u32) -> mat4x4<f32> {
    if (instance_index == 0u) {
        return scene.model_0;
    }
    return scene.model_1;
}

@vertex
fn vs_main(input: VsIn, @builtin(instance_index) instance_index: u32) -> VsOut {
    let model = instance_model(instance_index);
    let world_position = model * vec4<f32>(input.position, 1.0);

    var out: VsOut;
    out.clip_position = scene.view_proj * world_position;
    out.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    out.uv = input.uv;
    return out;
}

struct FsOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
};

@fragment
fn fs_main(input: VsOut) -> FsOut {
    let tiled_uv = input.uv * 2.0;
    let albedo = textureSample(base_color_texture, base_color_sampler, tiled_uv).rgb;

    var out: FsOut;
    out.albedo = vec4<f32>(albedo, 1.0);
    out.normal = vec4<f32>(input.world_normal * 0.5 + 0.5, 1.0);
    return out;
}
