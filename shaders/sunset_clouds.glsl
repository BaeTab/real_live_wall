// sunset_clouds.glsl — drifting, back-lit clouds over a sunset gradient.
// The sun halo pulses with the treble; a warm rim lights the cloud tops.
// Domain-warped fbm gives the clouds volume. Shadertoy-compatible.

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
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
        p = p * 2.0 + vec2(11.3, 5.7);
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    vec2 sunp = vec2(0.5, 0.20);
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));

    // --- sky gradient, warm at the horizon into cool dusk overhead ---
    vec3 sky = mix(vec3(1.15, 0.55, 0.25), vec3(0.90, 0.45, 0.45), smoothstep(0.0, 0.35, uv.y));
    sky = mix(sky, vec3(0.22, 0.20, 0.50), smoothstep(0.3, 1.0, pow(uv.y, 0.85)));
    sky += vec3(1.0, 0.55, 0.3) * smoothstep(0.9, 0.0, dsun) * 0.5;   // glow around the sun
    vec3 col = sky;

    // --- two parallax cloud layers with back-lit rims ---
    for (int L = 0; L < 2; L++) {
        float fl = float(L);
        float sc = 3.0 + fl * 2.5;
        vec2 p = vec2(uv.x * sc - iTime * (0.02 + fl * 0.02),
                      uv.y * sc * 1.3 + fl * 10.0 + iTime * 0.008);
        float c = fbm(p + fbm(p));                                   // domain warp
        float mask = smoothstep(0.5, 0.95, c) * smoothstep(0.02, 0.5, uv.y);
        float rim = smoothstep(0.6, 0.5, c) * smoothstep(0.7, 0.0, dsun);
        vec3 body = mix(vec3(0.5, 0.35, 0.42), vec3(0.16, 0.12, 0.22), fl * 0.5);
        vec3 lit = mix(body, vec3(1.2, 0.75, 0.45), rim * 1.3);
        col = mix(col, lit, mask * (1.0 - fl * 0.25));
    }

    // --- sun disk (bright core → bloom), halo brightens with treble ---
    col += vec3(1.6, 1.0, 0.55) * smoothstep(0.05, 0.0, dsun) * 1.8;
    col += vec3(1.2, 0.6, 0.35) * smoothstep(0.28, 0.0, dsun) * (0.5 + 0.6 * iTreble);

    col += (hash(fragCoord + iTime) - 0.5) / 255.0;   // dither
    fragColor = vec4(col, 1.0);
}
