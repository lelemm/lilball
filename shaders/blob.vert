#version 450

// Per-vertex quad corner in [-1, 1].
layout(location = 0) in vec2 a_corner;

// Per-instance data (see Instance in renderer.rs).
layout(location = 1) in vec2 i_center;    // center in logical pixels
layout(location = 2) in vec2 i_half;      // half-extent (rx, ry) in pixels
layout(location = 3) in vec4 i_color;     // rgba, alpha used as intensity
layout(location = 4) in float i_softness; // 0 = hard edge, 1 = fully soft
layout(location = 5) in float i_material; // 0 = glow blob, 1 = soccer ball

layout(push_constant) uniform Push {
    vec2 resolution; // framebuffer size in pixels
} pc;

layout(location = 0) out vec2 v_local;
layout(location = 1) out vec4 v_color;
layout(location = 2) out float v_softness;
layout(location = 3) out float v_material;

void main() {
    vec2 world = i_center + a_corner * i_half;
    // Map logical pixels (origin top-left, y down) to Vulkan NDC (y down).
    vec2 ndc = (world / pc.resolution) * 2.0 - 1.0;
    gl_Position = vec4(ndc, 0.0, 1.0);
    v_local = a_corner;
    v_color = i_color;
    v_softness = i_softness;
    v_material = i_material;
}
