// ocean.glsl — calm sea at golden hour with a low sun.
// Shimmer breathes with the bass (iBass). Shadertoy-compatible.

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(41.3, 289.7))) * 43758.5453);
}
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float horizon = 0.55;
    vec3 col;

    if (uv.y > horizon) {
        // sky
        float t = (uv.y - horizon) / (1.0 - horizon);
        col = mix(vec3(1.0, 0.6, 0.35), vec3(0.2, 0.35, 0.7), t);
        vec2 sunp = vec2(0.5, horizon + 0.12);
        float sun = smoothstep(0.16, 0.0, distance(uv, sunp));
        col += vec3(1.0, 0.8, 0.45) * sun * 1.3;
    } else {
        // sea
        float d = horizon - uv.y;
        float w = 0.5 * noise(vec2(uv.x * 8.0, iTime * 0.6 + d * 20.0));
        w += 0.25 * noise(vec2(uv.x * 16.0 - iTime * 0.3, d * 40.0));
        float shimmer = smoothstep(0.4, 0.9, w);
        col = mix(vec3(0.05, 0.16, 0.32), vec3(0.30, 0.5, 0.72), d * 1.5);
        col += vec3(1.0, 0.8, 0.5) * shimmer * smoothstep(0.0, 0.25, d) * (0.4 + 0.7 * iBass);
        float glitter = smoothstep(0.09, 0.0, abs(uv.x - 0.5)) * shimmer;
        col += vec3(1.0, 0.85, 0.5) * glitter * 0.5;
    }
    fragColor = vec4(col, 1.0);
}
