// ocean.glsl — golden-hour sea.
// A hazy, light-scattering sky with wind-stretched cirrus sits over layered,
// perspective-scaled swells. The water is shaded by a Fresnel blend of the
// reflected sky against a deep-water colour (grazing angles near the horizon
// mirror the sky, the foreground shows its own depth), and a sun pillar of
// perspective glitter runs from the horizon down toward the viewer. Swell
// amplitude breathes gently with iBass; the sun-path sparkle twinkles with
// iTreble. Fully procedural — no textures.
// Shadertoy-compatible (engine extensions: iBass, iTreble).

// sin-free hash — trig hashes collapse into visible blocks on some GPUs.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
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

mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

// Six-octave fbm; each octave is rotated so the lattice never lines up.
float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int k = 0; k < 6; k++) {
        s += a * noise(p);
        p = rot(0.55) * p * 2.02 + vec2(19.1, 7.7);
        a *= 0.5;
    }
    return s;
}

// Sky colour at a normalised screen point. Reused to shade water reflections,
// so it stays a pure function of position (plus time for the drifting cirrus).
vec3 skyColor(vec2 uv, float aspect, vec2 sunp, float horizon, float t) {
    float h = clamp((uv.y - horizon) / (1.0 - horizon), 0.0, 1.0);
    vec3 low  = vec3(1.00, 0.60, 0.33);   // warm band hugging the horizon
    vec3 mid  = vec3(0.76, 0.50, 0.51);   // peach
    vec3 high = vec3(0.15, 0.26, 0.55);   // cool blue overhead
    vec3 c = mix(mix(low, mid, smoothstep(0.0, 0.35, h)), high, smoothstep(0.16, 1.0, h));

    // atmospheric scattering: a restrained warm glow tightening toward the sun
    // (the ACES+bloom stage amplifies anything past ~0.75 — stay under budget)
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));
    c += vec3(1.10, 0.62, 0.34) * smoothstep(0.55, 0.0, dsun) * 0.26;
    c += vec3(1.15, 0.55, 0.30) * smoothstep(0.22, 0.0, dsun) * 0.38;

    // thin, wind-stretched cirrus, brighter where it is back-lit near the sun
    float cir = fbm(vec2(uv.x * aspect * 2.2 - t * 0.015, uv.y * 7.0 + 3.0));
    cir = smoothstep(0.55, 0.95, cir) * smoothstep(0.015, 0.45, h);
    vec3 cloudCol = mix(vec3(0.70, 0.46, 0.50), vec3(1.05, 0.76, 0.58), smoothstep(0.50, 0.0, dsun));
    c = mix(c, cloudCol, cir * 0.45);
    return c;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float t = iTime;
    float bass = iBass;
    float treble = iTreble;

    float horizon = 0.50;
    vec2 sunp = vec2(0.5, horizon + 0.10);
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));

    vec3 col;
    if (uv.y > horizon) {
        // --- sky -----------------------------------------------------------
        col = skyColor(uv, aspect, sunp, horizon, t);
    } else {
        // --- sea -----------------------------------------------------------
        float d = horizon - uv.y;
        float dn = clamp(d / horizon, 0.0, 1.0);     // 0 at horizon → 1 at the bottom

        // perspective water coordinates; wave detail fades to glass right at
        // the horizon so the perspective blow-up never aliases. No normals —
        // the relief field itself shades the water (artifact-free).
        float persp = 1.0 / (d + 0.06);
        vec2 wp = vec2((uv.x - 0.5) * aspect * persp * 1.5, persp * 1.35 + t * 0.35);
        float w1 = fbm(wp * 0.9 + vec2(t * 0.10, 0.0));
        float w2 = fbm(wp * 2.1 - vec2(t * 0.14, t * 0.05));
        float wave = mix(w1, w2, 0.35);
        float detail = smoothstep(0.0, 0.16, d);      // glassy band at the horizon
        float rel = (wave - 0.5) * detail;            // signed relief

        // Fresnel by grazing angle: the far water mirrors the sky
        float fres = 0.06 + 0.80 * pow(1.0 - dn, 3.0);
        vec2 refUV = vec2(uv.x + rel * 0.10, horizon + d * (1.0 + rel * 0.8));
        refUV.y = clamp(refUV.y, horizon, 1.0);
        vec3 reflCol = skyColor(refUV, aspect, sunp, horizon, t);

        vec3 shallow = vec3(0.05, 0.20, 0.28);
        vec3 deep    = vec3(0.012, 0.06, 0.115);
        vec3 water = mix(shallow, deep, dn) * (0.90 + 0.40 * rel * (0.9 + 0.4 * bass));
        col = mix(water, reflCol, fres);

        // warm light on wave shoulders facing the sun
        float crest = smoothstep(0.60, 0.95, wave) * detail;
        col += vec3(0.60, 0.38, 0.22) * crest * (1.0 - dn) * 0.30;

        // sun pillar: narrow at the horizon, widening toward the viewer
        float colW = 0.025 + d * 0.85;
        float colMask = smoothstep(colW, 0.0, abs(uv.x - sunp.x) * aspect);
        col += vec3(0.95, 0.55, 0.30) * colMask * (1.0 - dn * 0.8) * 0.30;

        // glitter: sparse sparkle gated to the column and crests; the only sea
        // element allowed into HDR, twinkling harder with the treble. The extra
        // (1 + dn) frequency term keeps sparkles small near the viewer, where
        // the perspective coordinates would otherwise turn coarse and blobby.
        float sp = noise(wp * vec2(5.0, 2.6) * (1.0 + dn * 2.5) + vec2(t * 1.1, -t * 1.7));
        float glint = pow(sp, 6.0) * colMask * smoothstep(0.45, 0.85, wave) * detail
                    * (1.0 - 0.55 * dn);
        col += vec3(1.6, 1.15, 0.65) * glint * (1.5 + 2.0 * treble) * (0.9 + 0.3 * bass);
    }

    // sun disk: a bright HDR core to drive the bloom, plus a tighter inner glow
    col += vec3(1.5, 1.05, 0.60) * smoothstep(0.040, 0.0, dsun) * 2.0;
    col += vec3(1.3, 0.80, 0.45) * smoothstep(0.100, 0.0, dsun) * 0.45;
    // a thin warm seam along the horizon
    col += vec3(1.0, 0.70, 0.45) * smoothstep(0.008, 0.0, abs(uv.y - horizon)) * 0.30;

    col += (hash(fragCoord + t) - 0.5) / 255.0;    // dither out banding
    fragColor = vec4(col, 1.0);
}
