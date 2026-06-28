#version 450

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;

layout(push_constant) uniform Push {
    vec4 viewport; // resolution.xy, center.xy
    vec4 shape;    // half.xy, movement axis.xy
    vec4 roll;     // angle, unused
} pc;

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec3 v_normal;

void main() {
    vec2 axis = pc.shape.zw;
    if (dot(axis, axis) < 0.001) {
        axis = vec2(1.0, 0.0);
    } else {
        axis = normalize(axis);
    }
    vec2 tangent = vec2(-axis.y, axis.x);

    float c = cos(pc.roll.x);
    float s = sin(pc.roll.x);

    vec3 p = a_position;
    vec3 n = normalize(a_normal);

    // Roll around the axis perpendicular to movement, then project the front
    // hemisphere into the 2D transparent overlay.
    float rolled_x = p.x * c - p.z * s;
    float rolled_z = p.x * s + p.z * c;
    float normal_x = n.x * c - n.z * s;
    float normal_z = n.x * s + n.z * c;

    vec2 world = pc.viewport.zw
        + axis * (rolled_x * pc.shape.x)
        + tangent * (p.y * pc.shape.y);
    vec2 ndc = (world / pc.viewport.xy) * 2.0 - 1.0;

    gl_Position = vec4(ndc, 0.0, 1.0);
    v_uv = a_uv;
    v_normal = normalize(vec3(normal_x, n.y, normal_z));
}
