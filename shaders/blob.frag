#version 450

layout(location = 0) in vec2 v_local;     // [-1,1] quad-local coords
layout(location = 1) in vec4 v_color;     // rgba (a = intensity)
layout(location = 2) in float v_softness; // edge softness
layout(location = 3) in float v_material; // 0 = glow blob, 1 = soccer ball
layout(location = 4) in vec4 v_roll;      // xy = movement axis, z = roll angle

layout(set = 0, binding = 0) uniform sampler2D u_ball_texture;

layout(location = 0) out vec4 frag;

vec4 soccer_ball(vec2 local) {
    float r2 = dot(local, local);
    if (r2 > 1.0) {
        discard;
    }

    float z = sqrt(max(0.0, 1.0 - r2));
    vec2 axis = normalize(v_roll.xy);
    vec2 tangent = vec2(-axis.y, axis.x);
    float along = dot(local, axis);
    float across = dot(local, tangent);
    float spin = v_roll.z;
    float rolled_along = along * cos(spin) - z * sin(spin);
    vec2 material_local = axis * rolled_along + tangent * across;
    vec2 uv = material_local * 0.5 + 0.5;
    vec3 tex = texture(u_ball_texture, uv).rgb;

    vec3 sphere_normal = normalize(vec3(local.x, -local.y, z));

    // Use texture luminance changes as a tiny height field. This keeps seams
    // tactile without turning the ball glossy or procedurally repainting it.
    float h = dot(tex, vec3(0.299, 0.587, 0.114));
    vec2 grad = vec2(dFdx(h), dFdy(h));
    vec3 leather_normal = normalize(sphere_normal + vec3(-grad.x, grad.y, 0.0) * 0.38);

    vec3 light_dir = normalize(vec3(-0.35, -0.45, 0.82));
    float diffuse = max(dot(leather_normal, light_dir), 0.0);
    float matte = 0.78 + diffuse * 0.22;
    float edge_vignette = mix(0.76, 1.0, z);
    vec3 color = tex * matte * edge_vignette;

    float edge_alpha = 1.0 - smoothstep(0.975, 1.0, sqrt(r2));
    float alpha = edge_alpha * v_color.a;
    return vec4(color * alpha, alpha);
}

void main() {
    if (v_material > 0.5) {
        frag = soccer_ball(v_local);
        return;
    }

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
