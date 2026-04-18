#version 310 es

precision highp float;
precision highp int;

layout(input_attachment_index = 3) uniform usubpassInput _group_0_binding_0_fs;

layout(location = 0) out uvec4 _fs2p_location0;

void main() {
    uvec4 _e4 = subpassLoad(_group_0_binding_0_fs);
    _fs2p_location0 = _e4;
    return;
}

