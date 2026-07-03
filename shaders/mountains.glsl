// mountains.glsl — layered ridges under a dusk sky with a glowing sun.
// Distant ridges fade into haze; a bass pulse lifts the afterglow. The ridge
// silhouettes are anti-aliased (no stair-stepping). Shadertoy-compatible.

float hash(float x) { return fract(sin(x * 127.1) * 43758.5453); }
float hash2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
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
    for (int k = 0; k < 6; k++) {
        s += a * vnoise(x * fq);
        fq *= 2.0;
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float e = 1.5 / iResolution.y;             // ~1px anti-alias band

    // --- dusk sky, multi-stop gradient ---
    vec3 sky = mix(vec3(0.98, 0.52, 0.36), vec3(0.55, 0.30, 0.42), smoothstep(0.0, 0.45, uv.y));
    sky = mix(sky, vec3(0.10, 0.10, 0.28), smoothstep(0.4, 1.0, uv.y));
    vec3 col = sky;

    // low sun: bright core (bloom) + broad afterglow that swells on the bass
    vec2 sunp = vec2(0.5, 0.36);
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));
    col += vec3(1.5, 0.8, 0.45) * smoothstep(0.06, 0.0, dsun) * 1.7;
    col += vec3(1.1, 0.55, 0.35) * smoothstep(0.5, 0.0, dsun) * (0.35 + 0.5 * iBass);

    // twinkling stars in the upper sky
    vec2 g = floor(fragCoord / 2.5);
    float st = pow(hash2(g), 40.0);
    float tw = 0.5 + 0.5 * sin(iTime * 3.0 + hash2(g) * 30.0);
    col += vec3(st * tw) * smoothstep(0.5, 1.0, uv.y) * 1.2;

    // --- five parallax ridges, back → front, with atmospheric perspective ---
    for (int L = 0; L < 5; L++) {
        float fl = float(L);
        float base = 0.15 + fl * 0.11;
        float amp = 0.10 + fl * 0.02;
        float h = base + amp * ridge(uv.x * (2.0 + fl * 2.2) + fl * 13.0 + iTime * 0.015);
        float fill = smoothstep(h + e, h - e, uv.y);
        vec3 mc = mix(vec3(0.42, 0.30, 0.44), vec3(0.05, 0.05, 0.12), fl / 4.0);
        // warm rim where the ridge crest meets the sky
        mc = mix(mc, sky * 0.7, smoothstep(h - 0.05, h, uv.y) * (1.0 - fl / 4.0) * 0.5);
        col = mix(col, mc, fill);
    }

    col += (hash2(fragCoord + iTime) - 0.5) / 255.0;   // dither
    fragColor = vec4(col, 1.0);
}
