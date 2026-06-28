#version 450

layout(location = 0) in vec2 v_local;     // [-1,1] quad-local coords
layout(location = 1) in vec4 v_color;     // rgba (a = intensity)
layout(location = 2) in float v_softness; // edge softness

layout(location = 0) out vec4 frag;

void main() {
    // Distance from the center of the quad (1.0 at the inscribed circle edge).
    float d = length(v_local);

    // Soft circular falloff. `edge` is where the falloff begins.
    float edge = clamp(1.0 - v_softness, 0.0, 0.999);
    float a = 1.0 - smoothstep(edge, 1.0, d);

    // A little extra core brightness for a glowing look.
    a = a * a;

    float intensity = a * v_color.a;
    // Premultiplied output so we can use additive blending for glow.
    frag = vec4(v_color.rgb * intensity, intensity);
}
