// ocean.glsl — calm golden-hour sea with a glittering sun path.
// The swell and glitter breathe with the bass; sparkle picks up the treble.
// Shadertoy-compatible (engine extensions: iBass, iTreble).

float hash(vec2 p) { return fract(sin(dot(p, vec2(41.3, 289.7))) * 43758.5453); }
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
float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int k = 0; k < 6; k++) {
        s += a * noise(p);
        p = p * 2.0 + vec2(19.1, 7.7);
        a *= 0.5;
    }
    return s;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float horizon = 0.52;
    vec2 sunp = vec2(0.5, horizon + 0.11);
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));
    vec3 col;

    if (uv.y > horizon) {
        // --- sky: warm low band fading to cool blue, with soft haze cloud ---
        float t = (uv.y - horizon) / (1.0 - horizon);
        vec3 low = vec3(1.05, 0.62, 0.34);
        vec3 midc = vec3(0.85, 0.55, 0.55);
        vec3 high = vec3(0.24, 0.34, 0.62);
        col = mix(mix(low, midc, smoothstep(0.0, 0.4, t)), high, smoothstep(0.25, 1.0, t));
        float cl = fbm(vec2(uv.x * 3.0 - iTime * 0.02, uv.y * 6.0));
        col = mix(col, vec3(1.0, 0.8, 0.72), smoothstep(0.55, 0.9, cl) * 0.30 * (1.0 - t));
    } else {
        // --- sea: perspective-scaled, domain-warped ripples ---
        float d = horizon - uv.y;                  // depth below the horizon
        vec2 wp = vec2(uv.x * aspect * 3.0, 1.0 / (d + 0.04));
        float w = fbm(wp + vec2(iTime * 0.25, iTime * 0.6));
        w = mix(w, fbm(wp * 2.0 - vec2(iTime * 0.4, 0.0)), 0.5);
        float swell = (0.5 + 0.5 * sin(w * 6.0 + d * 40.0)) * (0.6 + 0.8 * iBass);

        vec3 deep = vec3(0.03, 0.12, 0.24);
        vec3 near = vec3(0.16, 0.34, 0.52);
        col = mix(near, deep, smoothstep(0.0, 0.5, d));
        col = mix(col, vec3(0.9, 0.55, 0.4), smoothstep(0.12, 0.0, d) * 0.6);  // reflected warmth

        // sun glitter column — sparkles gathered near x=0.5, fading with depth
        float colMask = smoothstep(0.28, 0.0, abs(uv.x - 0.5) / (0.15 + d * 0.8));
        float spark = pow(swell, 3.0) * colMask * smoothstep(0.0, 0.05, d);
        col += vec3(1.3, 1.0, 0.65) * spark * (1.2 + 1.5 * iTreble);
        col += vec3(1.0, 0.75, 0.5) * smoothstep(0.6, 1.0, w) * colMask * 0.4;
    }

    // sun disk + glow (bright values drive the bloom), and a horizon glow line
    col += vec3(1.4, 1.0, 0.6) * smoothstep(0.05, 0.0, dsun) * 1.6;
    col += vec3(1.2, 0.7, 0.4) * smoothstep(0.35, 0.0, dsun) * 0.5;
    col += vec3(1.0, 0.7, 0.45) * smoothstep(0.02, 0.0, abs(uv.y - horizon)) * 0.5;

    col += (hash(fragCoord + iTime) - 0.5) / 255.0;   // dither out banding
    fragColor = vec4(col, 1.0);
}
