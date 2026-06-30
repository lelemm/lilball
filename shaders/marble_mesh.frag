#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec3 v_local;
layout(location = 3) in vec2 v_screen_uv;

layout(set = 0, binding = 0) uniform sampler2D u_desktop_snapshot;

layout(push_constant) uniform Push {
    vec4 viewport;
    vec4 shape;
    vec4 state;     // roll angle, health, crack, seed
    vec4 primary;
    vec4 secondary;
    vec4 accent;
    vec4 ribbons;   // curve twist, brush width, curve amount, bubble density
    vec4 glass;     // bubble scale, tint, specular, refraction
} pc;

layout(location = 0) out vec4 frag;

float hash13(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return fract((p.x + p.y) * p.z);
}

float value_noise(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float n000 = hash13(i + vec3(0.0, 0.0, 0.0));
    float n100 = hash13(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash13(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash13(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash13(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash13(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash13(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash13(i + vec3(1.0, 1.0, 1.0));
    float nx00 = mix(n000, n100, f.x);
    float nx10 = mix(n010, n110, f.x);
    float nx01 = mix(n001, n101, f.x);
    float nx11 = mix(n011, n111, f.x);
    float nxy0 = mix(nx00, nx10, f.y);
    float nxy1 = mix(nx01, nx11, f.y);
    return mix(nxy0, nxy1, f.z);
}

mat2 rot2(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

vec3 to_material_space(vec3 p) {
    float seed_angle = hash13(vec3(pc.state.w * 0.000091, 0.23, 4.7)) * 6.28318;
    float roll = pc.state.x;
    float c = cos(roll);
    float s = sin(roll);

    vec3 q = p;
    q = vec3(q.x * c + q.z * s, q.y, -q.x * s + q.z * c);
    q.xy = rot2(seed_angle) * q.xy;
    q.yz = rot2(seed_angle * 0.47) * q.yz;
    q.xz = rot2(seed_angle * 0.23) * q.xz;
    return q;
}

vec2 curved_stack_space(vec3 p) {
    float seed = pc.state.w * 0.000043;
    float y = p.y;
    float twist = pc.ribbons.x;
    float curve = pc.ribbons.z;
    float wave = sin(y * (2.9 + twist * 1.15) + seed);
    wave += 0.42 * sin(y * (6.8 + twist * 0.72) + seed * 1.71);
    float spiral = y * (0.72 + twist * 0.46) + wave * curve * 0.46;
    vec2 stack = rot2(spiral) * p.xz;
    vec2 bend = vec2(
        sin(y * (3.3 + twist) + seed) + 0.35 * sin(y * 8.4 + seed * 1.9),
        cos(y * (2.7 + twist * 0.62) + seed * 1.37)
    ) * curve * vec2(0.20, 0.16);
    return stack - bend;
}

vec3 brush_color(float lane) {
    if (lane < 0.5) {
        return pc.primary.rgb;
    }
    if (lane < 1.5) {
        return pc.secondary.rgb;
    }
    if (lane < 2.5) {
        return pc.accent.rgb;
    }
    return mix(pc.accent.rgb, pc.primary.rgb, 0.42);
}

float brush_sheet(vec3 p, float lane, out float axial) {
    vec2 stack = curved_stack_space(p);
    float seed = pc.state.w * 0.000071 + lane * 8.37;
    axial = p.y;

    float lane_center = mix(-0.56, 0.56, lane / 3.0);
    lane_center += (hash13(vec3(seed, lane, 1.0)) - 0.5) * 0.08;
    float plane_depth = (hash13(vec3(seed, lane, 2.0)) - 0.5) * 0.52;

    float edge_noise = value_noise(vec3(axial * 1.7 + seed, lane * 4.1, 0.0));
    float width = pc.ribbons.y * mix(0.96, 1.42, edge_noise);
    float thickness = width * mix(0.64, 0.96, hash13(vec3(seed, lane, 3.0)));

    float across = abs(stack.x - lane_center) / width;
    float depth = abs(stack.y - plane_depth) / thickness;
    float slab = 1.0 - smoothstep(0.84, 1.04, max(across, depth));
    float soft_glass_edge = 1.0 - smoothstep(1.00, 1.20, max(across * 0.82, depth));

    float length_mask = 1.0 - smoothstep(0.72, 0.99, abs(axial));
    float pigment = 0.92 + 0.08 * value_noise(vec3(axial * 2.6, lane * 2.9, seed));
    float feather = (1.0 - smoothstep(0.90, 1.08, across)) *
        (1.0 - smoothstep(0.86, 1.04, depth));

    float body = max(slab, soft_glass_edge * 0.28) * feather;
    float inside_shell = 1.0 - smoothstep(0.92, 1.0, length(p));
    return clamp(body * length_mask * pigment, 0.0, 1.0) * inside_shell;
}

vec4 internal_paint(vec3 p) {
    vec3 color = vec3(0.0);
    float alpha = 0.0;
    for (int i = 0; i < 4; ++i) {
        float lane = float(i);
        float axial = 0.0;
        float mask = brush_sheet(p, lane, axial);
        vec3 lane_color = brush_color(lane);
        float pigment_shift = value_noise(vec3(axial * 3.1, lane * 2.2, pc.state.w * 0.000017));
        lane_color *= 0.88 + pigment_shift * 0.16;
        lane_color = mix(lane_color, brush_color(mod(lane + 1.0, 4.0)), 0.045 * pigment_shift);
        float stroke_alpha = smoothstep(0.08, 0.32, mask);
        color += lane_color * stroke_alpha * (1.0 - alpha);
        alpha += stroke_alpha * (1.0 - alpha);
    }
    return vec4(color, clamp(alpha, 0.0, 1.0));
}

float bubble_mask(vec3 p) {
    float density = pc.ribbons.w;
    float scale = 7.0 + pc.glass.x * 7.0;
    vec3 q = p * scale + pc.state.w * 0.00007;
    vec3 cell = floor(q);
    vec3 local = fract(q) - 0.5;
    float h = hash13(cell);
    float radius = mix(0.04, 0.16, hash13(cell + 7.31)) * density;
    float d = length(local);
    float outer = 1.0 - smoothstep(radius * 0.78, radius * 1.05, d);
    float inner = smoothstep(radius * 0.32, radius * 0.60, d);
    float shell = outer * inner;
    float air = 1.0 - smoothstep(radius * 0.18, radius * 0.50, d);
    float inside_shell = 1.0 - smoothstep(0.94, 1.0, length(p));
    return (shell + air * 0.24) * step(1.0 - density * 0.46, h) * inside_shell;
}

float crack_mask(vec3 p) {
    float crack = clamp(pc.state.z, 0.0, 1.0);
    if (crack <= 0.01) {
        return 0.0;
    }
    float seed = pc.state.w * 0.00019;
    vec3 q = p * (8.0 + crack * 10.0);
    float a = abs(sin(q.x * 2.7 + q.y * 1.4 + seed));
    float b = abs(sin(q.y * 3.1 - q.z * 2.2 + seed * 1.7));
    float line = smoothstep(0.035 + crack * 0.025, 0.0, min(a, b));
    return line * crack;
}

void main() {
    if (v_normal.z <= 0.0) {
        discard;
    }

    vec3 n = normalize(v_normal);
    float health = clamp(pc.state.y, 0.0, 1.0);
    float crack = clamp(pc.state.z, 0.0, 1.0);
    float fresnel = pow(1.0 - clamp(n.z, 0.0, 1.0), 2.65);

    vec2 plane = v_local.xy;
    float plane_len = dot(plane, plane);
    if (plane_len > 1.0001) {
        discard;
    }
    float depth = sqrt(max(1.0 - plane_len, 0.0));

    vec2 refract_offset = n.xy * pc.glass.w * (0.52 + fresnel * 0.72);
    vec2 refract_uv = clamp(v_screen_uv + refract_offset, vec2(0.0), vec2(1.0));
    vec3 desktop;
    desktop.r = texture(u_desktop_snapshot, clamp(v_screen_uv + refract_offset * 1.05, vec2(0.0), vec2(1.0))).r;
    desktop.g = texture(u_desktop_snapshot, refract_uv).g;
    desktop.b = texture(u_desktop_snapshot, clamp(v_screen_uv + refract_offset * 0.94, vec2(0.0), vec2(1.0))).b;

    float body_mix = hash13(vec3(pc.state.w * 0.000037, 4.1, 8.7));
    vec3 body_tint = mix(pc.primary.rgb, pc.secondary.rgb, body_mix);
    body_tint = mix(body_tint, pc.accent.rgb, 0.14 + 0.10 * hash13(vec3(pc.state.w, 2.0, 5.0)));

    vec3 volume_energy = vec3(0.0);
    float volume_alpha = 0.0;
    float bubble = 0.0;
    const int VOLUME_STEPS = 9;
    for (int i = 0; i < VOLUME_STEPS; ++i) {
        float t = (float(i) + 0.5) / float(VOLUME_STEPS);
        float z = mix(depth * 0.78, -depth * 0.88, t);
        vec3 sample_view = vec3(plane, z);
        vec3 sample_material = to_material_space(sample_view);
        vec4 paint = internal_paint(sample_material);
        float segment = max(depth * 2.0 / float(VOLUME_STEPS), 0.03);
        float front_light = 0.84 + 0.34 * smoothstep(-depth, depth, z);
        float core_light = 0.94 + 0.22 * (1.0 - plane_len);
        float environment_light = 0.30 + 0.18 * (1.0 - plane_len);
        float self_emit = 0.08 + 0.05 * smoothstep(0.02, 0.36, paint.a);
        float paint_light = environment_light + front_light * core_light * 0.70 + self_emit;
        float a = 1.0 - exp(-paint.a * segment * 7.2);
        volume_energy += paint.rgb * paint_light * a * (1.0 - volume_alpha);
        volume_alpha += a * (1.0 - volume_alpha);
        bubble += bubble_mask(sample_material) * segment;
    }
    volume_alpha = clamp(volume_alpha, 0.0, 0.92);
    vec3 volume_color = volume_energy / max(volume_alpha, 0.001);
    bubble = clamp(bubble * 0.48, 0.0, 1.0);

    vec3 view_dir = vec3(0.0, 0.0, 1.0);
    vec3 light_dir = normalize(vec3(-0.06, -0.09, 1.0));
    vec3 half_dir = normalize(light_dir + view_dir);
    float facing_light = max(dot(n, light_dir), 0.0);
    float specular = pow(max(dot(n, half_dir), 0.0), 220.0) * pc.glass.z * 0.16;
    float sheen = pow(max(dot(n, half_dir), 0.0), 48.0) * pc.glass.z * 0.026;

    vec3 reflect_desktop = texture(
        u_desktop_snapshot,
        clamp(v_screen_uv - n.xy * (0.035 + fresnel * 0.025), vec2(0.0), vec2(1.0))
    ).rgb;
    vec3 glass_tint = mix(vec3(1.0), body_tint, pc.glass.y * 0.58);
    vec3 color = desktop * glass_tint * (0.84 + fresnel * 0.10);
    color += body_tint * pc.glass.y * (0.055 + (1.0 - fresnel) * 0.045);
    vec3 lit_volume = volume_color * (1.10 + facing_light * 0.22);
    float paint_coverage = smoothstep(0.24, 0.78, volume_alpha);
    color = mix(color, lit_volume, paint_coverage);
    color += volume_color * volume_alpha * (0.16 + health * 0.06);
    color = mix(color, reflect_desktop, fresnel * 0.13);
    color += mix(pc.accent.rgb, body_tint, 0.38) * fresnel * 0.10;
    float internal_rim = fresnel * volume_alpha * (0.26 + pc.glass.y * 0.14);
    color += volume_color * internal_rim * 0.86;
    color = mix(color, vec3(0.90, 0.97, 1.0), bubble * (0.22 + fresnel * 0.16));
    color += vec3(specular + sheen);

    float crack_line = crack_mask(to_material_space(v_local));
    color = mix(color, vec3(0.045, 0.055, 0.065), crack_line * 0.72);
    color *= 1.0 - crack * 0.13;

    float edge = smoothstep(0.0, 0.16, n.z);
    float alpha = edge * (0.34 + fresnel * 0.34 + volume_alpha * 0.34 + bubble * 0.08);
    alpha *= 0.90 + health * 0.10;
    alpha = max(alpha, edge * paint_coverage * 0.985);
    alpha = clamp(alpha + specular * 0.35 + internal_rim * 0.12, 0.0, 0.99);
    frag = vec4(color * alpha, alpha);
}
