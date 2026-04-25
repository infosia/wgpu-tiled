// language: metal1.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;


struct main_Output {
    metal::float4 member [[color(0)]];
};
fragment main_Output main_(
  metal::float4 gbuffer_albedo [[user(fake0)]]
, metal::float4 gbuffer_normal [[user(fake0)]]
, metal::float4 gbuffer_material [[user(fake0)]]
) {
    metal::float4 albedo = gbuffer_albedo;
    metal::float4 normal = gbuffer_normal;
    metal::float4 material = gbuffer_material;
    return main_Output { ((albedo + normal) + material) / metal::float4(3.0) };
}
