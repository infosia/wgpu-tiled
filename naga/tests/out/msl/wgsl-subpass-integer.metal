// language: metal1.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;


struct main_Output {
    metal::uint4 member [[color(0)]];
};
fragment main_Output main_(
  metal::uint4 gbuffer_uint [[color(3)]]
) {
    metal::uint4 _e1 = gbuffer_uint;
    return main_Output { _e1 };
}
