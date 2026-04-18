#version 310 es
#extension GL_EXT_shader_framebuffer_fetch : require

precision highp float;
precision highp int;

inout vec4 _group_0_binding_0_fs;

layout(location = 0) out vec4 _fs2p_location0;

void main() {
    vec4 _e4 = _group_0_binding_0_fs;
    _fs2p_location0 = _e4;
    return;
}

