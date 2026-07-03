// plasma.glsl — "silk aurora": iridescent ink flowing in slow, meditative sheets.
// A pure Shadertoy demo — only iTime and iResolution, no engine extensions — so
// it proves that stock Shadertoy image shaders run unmodified on real_live_wall.
// Space is folded through itself twice (domain-warped fbm) to lay thin sheets of
// light over a deep field; a cosine palette tints them through an oil-on-water
// sweep (gold → magenta → azure → teal), and only the thinnest filaments cross
// into HDR for a soft bloom. Dithered to keep the deep gradients band-free.
//
//   real_live_wall --shader shaders/plasma.glsl

// sin-free hash — fract(sin(x)*43758.…) collapses into visible blocks on some
// GPUs (fp32 sin precision breaks down for large arguments); this one doesn't.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
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
    for (int k = 0; k < 5; k++) {
        s += a * noise(p);
        // rotate + scale each octave so the value-noise lattice never lines up
        p = mat2(1.6, 1.2, -1.2, 1.6) * p + vec2(3.1, 1.7);
        a *= 0.5;
    }
    return s;
}

vec2 rot(vec2 v, float a) {
    float s = sin(a);
    float c = cos(a);
    return vec2(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Curated ink ramp — deep indigo through teal and orchid into gold crests.
// A fixed ramp (not a rolling cosine palette) so the scene never drifts through
// muddy hue regions: a wallpaper is stared at all day.
vec3 inkRamp(float f, float g) {
    vec3 deep   = vec3(0.05, 0.12, 0.35);
    vec3 teal   = vec3(0.10, 0.55, 0.60);
    vec3 orchid = vec3(0.75, 0.30, 0.62);
    vec3 gold   = vec3(1.05, 0.78, 0.42);
    vec3 c = mix(deep, teal, smoothstep(0.30, 0.58, f));
    c = mix(c, orchid, smoothstep(0.58, 0.80, f));
    c = mix(c, gold, smoothstep(0.82, 0.98, f));
    // the sheen field pulls the mids toward orchid for iridescent variety
    return mix(c, orchid, 0.25 * smoothstep(0.55, 0.95, g));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    float t = iTime * 0.08;                 // slow and meditative

    // --- domain-warped flow field: fold space through itself twice ----------
    vec2 q = rot(p * 1.3, 0.20);
    vec2 w1 = vec2(fbm(q + vec2(0.0, t)),
                   fbm(q + vec2(5.2, -t) + 1.3));
    vec2 w2 = vec2(fbm(q * 1.7 + 2.0 * w1 + vec2(t, 1.7)),
                   fbm(q * 1.7 + 2.0 * w1 + vec2(-1.5, t * 0.8)));
    float f = fbm(q + 3.0 * w2);            // final silk field
    float g = fbm(q * 2.3 - 2.0 * w2 + 4.0); // second field for sheen / highlights

    // --- iridescent body: colour by the field, ink-dark in the troughs ------
    // Keep most of the frame in deep ink; only the folded sheets carry light.
    vec3 col = inkRamp(f, g);
    float body = smoothstep(0.35, 0.95, f);
    col *= mix(0.10, 0.85, body * body);
    col = mix(vec3(0.010, 0.016, 0.048), col, smoothstep(0.22, 0.80, f));

    // --- thin silk filaments where the folded sheets meet -------------------
    float fold = abs(w2.x - w2.y);
    float fil = smoothstep(0.08, 0.0, fold);              // fine ridges
    float crest = pow(smoothstep(0.55, 0.95, g), 3.0);    // sparse specular crests
    vec3 sheen = mix(vec3(0.35, 0.80, 0.85), vec3(1.10, 0.85, 0.50), smoothstep(0.4, 0.9, g));
    col += sheen * fil * 0.45 * (0.3 + 0.7 * body);
    col += sheen * crest * 1.4;                            // small HDR highlight

    // --- gentle vignette seats the flow in the dark -------------------------
    col *= 0.85 + 0.15 * (1.0 - dot(p, p) * 0.25);

    col += (hash(fragCoord + iTime) - 0.5) / 255.0;        // dither out banding
    fragColor = vec4(max(col, vec3(0.0)), 1.0);
}
