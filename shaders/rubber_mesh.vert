#version 450

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec4 a_rubber;

layout(push_constant) uniform Push {
    vec4 viewport; // resolution.xy, unused.zw
} pc;

layout(location = 0) out vec3 v_normal;
layout(location = 1) out vec4 v_color;
layout(location = 2) out vec4 v_rubber;

void main() {
    vec2 ndc = (a_position.xy / pc.viewport.xy) * 2.0 - 1.0;
    gl_Position = vec4(ndc, 0.0, 1.0);

    v_normal = normalize(a_normal);
    v_color = a_color;
    v_rubber = a_rubber;
}
