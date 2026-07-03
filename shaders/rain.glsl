// rain.glsl — moody, layered rainfall with a bass-triggered lightning flash.
// Four depth layers of slanted streaks over a slow, stormy cloud backdrop;
// drifting ground mist; the whole sky flashes on a bass hit. Shadertoy-compatible.

float hash(vec2 p) { return fract(sin(dot(p, vec2(12.9, 78.2))) * 43758.5); }
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
    for (int k = 0; k < 5; k++) {
        s += a * noise(p);
        p *= 2.0;
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;

    // --- stormy sky with slow, roiling clouds ---
    vec3 col = mix(vec3(0.03, 0.04, 0.07), vec3(0.10, 0.12, 0.18), uv.y);
    float cl = fbm(vec2(uv.x * 3.0 - iTime * 0.02, uv.y * 3.0));
    col = mix(col, vec3(0.13, 0.15, 0.22), smoothstep(0.5, 0.9, cl) * smoothstep(0.2, 1.0, uv.y));

    // --- rain: several depth layers, slightly wind-slanted ---
    float rain = 0.0;
    for (int k = 0; k < 4; k++) {
        float fk = float(k);
        float cols = 90.0 + fk * 60.0;
        vec2 rp = vec2(uv.x * aspect + uv.y * 0.15, uv.y);   // shear = slant
        float x = floor(rp.x * cols);
        float speed = 1.3 + hash(vec2(x, fk)) * 1.6;
        float y = fract(rp.y * 2.0 + iTime * speed + hash(vec2(x, fk + 7.0)) * 10.0);
        float streak = smoothstep(0.0, 0.015, y) * smoothstep(0.22, 0.0, y);
        float bright = 0.3 + 0.7 * hash(vec2(x, fk + 3.0));
        rain += streak * bright * (1.0 - fk * 0.2);
    }
    col += vec3(0.55, 0.65, 0.9) * rain * 0.22;

    // --- drifting ground mist ---
    float mist = fbm(vec2(uv.x * 4.0 + iTime * 0.3, iTime * 0.2));
    col += vec3(0.10, 0.12, 0.18) * smoothstep(0.32, 0.0, uv.y) * (0.6 + 0.4 * mist);

    // --- lightning: a bass hit flashes the sky bright (drives the bloom) ---
    float bolt = smoothstep(0.55, 1.0, iBass);
    col += vec3(0.8, 0.85, 1.1) * bolt * (0.6 + 0.8 * smoothstep(1.0, 0.3, uv.y));

    col += (hash(fragCoord + iTime) - 0.5) / 255.0;   // dither
    fragColor = vec4(col, 1.0);
}
