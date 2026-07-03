// sunset_clouds.glsl — drifting fbm clouds over a sunset gradient.

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}
float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int k = 0; k < 6; k++) {
        s += a * noise(p);
        p *= 2.0;
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec3 sky = mix(vec3(1.0, 0.55, 0.25), vec3(0.25, 0.2, 0.5), pow(uv.y, 0.8));
    float c = fbm(vec2(uv.x * 3.0 - iTime * 0.03, uv.y * 3.0 + iTime * 0.01));
    float cloud = smoothstep(0.5, 0.95, c) * smoothstep(0.05, 0.55, uv.y);
    vec3 cloudCol = mix(vec3(0.95, 0.65, 0.5), vec3(0.4, 0.3, 0.45), uv.y);
    vec3 col = mix(sky, cloudCol, cloud);
    // low sun glow, brightening with treble
    float sun = smoothstep(0.35, 0.0, distance(uv, vec2(0.5, 0.22)));
    col += vec3(1.0, 0.7, 0.4) * sun * (0.45 + 0.4 * iTreble);
    fragColor = vec4(col, 1.0);
}
