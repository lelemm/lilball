#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec3 v_normal;

layout(set = 0, binding = 0) uniform sampler2D u_ball_texture;

layout(location = 0) out vec4 frag;

void main() {
    if (v_normal.z <= 0.0) {
        discard;
    }

    vec3 tex = texture(u_ball_texture, v_uv).rgb;
    vec3 light_dir = normalize(vec3(-0.35, -0.45, 0.82));
    float diffuse = max(dot(normalize(v_normal), light_dir), 0.0);
    float matte = 0.70 + diffuse * 0.30;
    float rim = mix(0.76, 1.0, smoothstep(0.0, 0.35, v_normal.z));
    vec3 color = tex * matte * rim;

    frag = vec4(color, 1.0);
}
