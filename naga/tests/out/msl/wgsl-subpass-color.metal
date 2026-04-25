// language: metal1.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;


struct main_Output {
    metal::float4 member [[color(0)]];
};
fragment main_Output main_(
  metal::float4 gbuffer_color [[user(fake0)]]
) {
    metal::float4 _e1 = gbuffer_color;
    return main_Output { _e1 };
}
