// mountains.glsl — layered mountain ridges at dusk.

float hash(float x) {
    return fract(sin(x * 127.1) * 43758.5453);
}
float vnoise(float x) {
    float i = floor(x);
    float f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(hash(i), hash(i + 1.0), f);
}
float ridge(float x) {
    float s = 0.0;
    float a = 0.5;
    float fq = 1.0;
    for (int k = 0; k < 5; k++) {
        s += a * vnoise(x * fq);
        fq *= 2.0;
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    // dusk sky
    vec3 col = mix(vec3(0.95, 0.5, 0.4), vec3(0.12, 0.1, 0.3), uv.y);
    col += vec3(1.0, 0.7, 0.4) * smoothstep(0.28, 0.0, distance(uv, vec2(0.5, 0.34))) * 0.6;
    // stars in the upper sky
    float star = pow(hash(floor(fragCoord.x * 0.7) + floor(fragCoord.y * 0.7) * 91.0), 60.0);
    col += vec3(star) * smoothstep(0.55, 1.0, uv.y);

    // four parallax ridges, near ones darker
    for (int L = 0; L < 4; L++) {
        float fl = float(L);
        float base = 0.16 + fl * 0.12;
        float h = base + 0.13 * ridge(uv.x * (2.0 + fl * 2.5) + fl * 10.0 + iTime * 0.02);
        float m = step(uv.y, h);
        vec3 mc = mix(vec3(0.08, 0.08, 0.18), vec3(0.26, 0.19, 0.36), fl / 3.0);
        col = mix(col, mc, m);
    }
    fragColor = vec4(col, 1.0);
}
