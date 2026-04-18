// language: metal1.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;


struct main_Output {
    metal::float4 member [[color(0)]];
};
fragment main_Output main_(
  float gbuffer_depth [[color(1)]]
) {
    float depth = gbuffer_depth;
    return main_Output { metal::float4(depth, depth, depth, 1.0) };
}
