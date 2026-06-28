#version 450

layout(location = 0) in vec2 v_local;     // [-1,1] quad-local coords
layout(location = 1) in vec4 v_color;     // rgba (a = intensity)
layout(location = 2) in float v_softness; // edge softness
layout(location = 3) in float v_material; // 0 = glow blob, 1 = soccer ball

layout(set = 0, binding = 0) uniform sampler2D u_ball_texture;

layout(location = 0) out vec4 frag;

float line_mask(float value, float width) {
    float d = abs(fract(value) - 0.5);
    return 1.0 - smoothstep(width, width + 0.018, d);
}

float pent_mask(vec2 p, float radius) {
    float a = atan(p.y, p.x);
    float sector = 3.14159265 * 2.0 / 5.0;
    float d = cos(floor(0.5 + a / sector) * sector - a) * length(p);
    return 1.0 - smoothstep(radius, radius + 0.035, d);
}

float soccer_height(vec3 n) {
    vec2 uv = vec2(atan(n.z, n.x) / 6.2831853 + 0.5, asin(n.y) / 3.14159265 + 0.5);
    vec3 tex = texture(u_ball_texture, uv).rgb;
    float tex_edge = length(vec2(dFdx(dot(tex, vec3(0.299, 0.587, 0.114))), dFdy(dot(tex, vec3(0.299, 0.587, 0.114)))));
    float longitude = line_mask(uv.x * 10.0, 0.035);
    float latitude = line_mask(uv.y * 6.0 + 0.08 * sin(uv.x * 31.4159), 0.04);
    vec2 front = vec2(n.x, n.y) / max(0.25, 1.0 + n.z);
    vec2 side = vec2(n.z, n.y) / max(0.25, 1.0 + abs(n.x));
    float patch_mask = max(pent_mask(front, 0.28), 0.65 * pent_mask(side - vec2(0.18, 0.02), 0.24));
    return max(max(max(longitude, latitude) * 0.5, patch_mask), tex_edge * 8.0);
}

vec4 soccer_ball(vec2 local) {
    float r2 = dot(local, local);
    if (r2 > 1.0) {
        discard;
    }

    float z = sqrt(max(0.0, 1.0 - r2));
    vec3 n = normalize(vec3(local.x, -local.y, z));
    vec2 uv = vec2(atan(n.z, n.x) / 6.2831853 + 0.5, asin(n.y) / 3.14159265 + 0.5);
    vec3 tex = texture(u_ball_texture, uv).rgb;

    float h = soccer_height(n);
    vec2 grad = vec2(dFdx(h), dFdy(h));
    vec3 bumped = normalize(n + vec3(-grad.x, grad.y, h * 0.11));

    float seam = max(
        line_mask(uv.x * 10.0, 0.035),
        line_mask(uv.y * 6.0 + 0.08 * sin(uv.x * 31.4159), 0.04)
    );
    vec2 front = vec2(n.x, n.y) / max(0.25, 1.0 + n.z);
    vec2 side = vec2(n.z, n.y) / max(0.25, 1.0 + abs(n.x));
    float black_patch = max(pent_mask(front, 0.28), 0.72 * pent_mask(side - vec2(0.18, 0.02), 0.24));
    float black = max(black_patch, seam * 0.78);

    vec3 white_panel = vec3(0.92, 0.9, 0.84);
    vec3 black_panel = vec3(0.015, 0.017, 0.02);
    vec3 albedo = mix(tex, mix(white_panel, black_panel, black), 0.32);

    vec3 light_dir = normalize(vec3(-0.42, -0.55, 0.72));
    vec3 view_dir = vec3(0.0, 0.0, 1.0);
    float diffuse = max(dot(bumped, light_dir), 0.0);
    float rim = pow(1.0 - max(dot(n, view_dir), 0.0), 2.8);
    vec3 half_dir = normalize(light_dir + view_dir);
    float spec = pow(max(dot(bumped, half_dir), 0.0), 42.0) * (1.0 - black * 0.45);
    float relief_shadow = 1.0 - seam * 0.18 - black_patch * 0.08;

    vec3 color = albedo * (0.28 + diffuse * 0.78) * relief_shadow;
    color += vec3(1.0, 0.96, 0.84) * spec * 0.38;
    color += vec3(0.25, 0.55, 1.0) * rim * 0.22;

    float edge_alpha = 1.0 - smoothstep(0.965, 1.0, sqrt(r2));
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
