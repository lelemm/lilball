#version 450

layout(location = 0) in vec3 v_normal;
layout(location = 1) in vec4 v_color;
layout(location = 2) in vec4 v_rubber;

layout(push_constant) uniform Push {
    vec4 viewport; // resolution.xy, unused.zw
} pc;

layout(location = 0) out vec4 frag;

void main() {
    vec3 n = normalize(v_normal);
    if (n.z < -0.10) {
        discard;
    }

    vec3 light_dir = normalize(vec3(-0.35, -0.55, 0.76));
    float diffuse = max(dot(n, light_dir), 0.0);
    float front = smoothstep(-0.05, 0.80, n.z);
    float fresnel = pow(1.0 - clamp(n.z, 0.0, 1.0), 2.2);
    float spec = pow(max(dot(reflect(-light_dir, n), vec3(0.0, 0.0, 1.0)), 0.0), 28.0);

    float strand = sin(v_rubber.x * 95.0) * 0.5 + 0.5;
    float joint = clamp(v_rubber.y, 0.0, 1.0);
    vec3 base = v_color.rgb * (0.48 + diffuse * 0.38 + front * 0.18);
    base *= 0.92 + strand * 0.08;
    base = mix(base, v_color.rgb * 1.25, joint * 0.34);

    vec3 sheen = vec3(0.75, 0.90, 1.0) * (fresnel * 0.18 + spec * 0.22);
    float alpha = v_color.a * (0.76 + front * 0.20 + joint * 0.04);
    vec3 color = base + sheen;

    frag = vec4(color * alpha, alpha);
}
