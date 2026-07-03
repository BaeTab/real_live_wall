// audio_bars.glsl — "Neon Spectrum Ring": a commercial-grade music visualizer.
//
// A mirror-symmetric circular spectrum ring floats above a wet, reflective
// floor. Frequency drives a neon cyan -> purple -> pink gradient and the bar
// tips bloom in HDR. A pulsing bass core sits inside the ring and sheds soft
// shockwaves on heavy bass. In silence the ring keeps a slow breathing/idle
// motion so the frame always reads as a finished picture, never empty.
//
// Engine reactive uniforms: iBass, iMid, iTreble, iVolume, iSpectrum(x).
// Tuned for the HDR bloom pipeline (bright tips/core exceed 1.0 on purpose:
// bright-pass threshold 0.75). Shadertoy-compatible: origin bottom-left
// (the engine flips Y), and the ring stays circular at any aspect ratio.
//
//   real_live_wall --shader shaders/audio_bars.glsl --watch

const float PI = 3.14159265;

// ---- helpers ---------------------------------------------------------------

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 345.45));
    p += dot(p, p + 34.345);
    return fract(p.x * p.y);
}

// frequency position (0..1) -> harmonious neon gradient
vec3 palette(float t) {
    vec3 cyan   = vec3(0.10, 0.90, 1.00);
    vec3 purple = vec3(0.55, 0.24, 1.00);
    vec3 pink   = vec3(1.00, 0.22, 0.62);
    vec3 c = mix(cyan, purple, smoothstep(0.0, 0.55, t));
    c = mix(c, pink, smoothstep(0.5, 1.0, t));
    return c;
}

// Spectrum height for one bar, with an animated idle floor so a bar never
// collapses to zero. In silence this becomes a gentle breathing shimmer.
float barHeight(float binf, float seed) {
    float s = pow(max(iSpectrum(binf), 0.0), 0.72);   // perceptual lift
    float idle = 0.11
               + 0.05 * sin(iTime * 1.10 + seed * 6.2831)
               + 0.03 * sin(iTime * 2.70 - seed * 11.0);
    return clamp(max(s, idle), 0.0, 1.25);
}

// Additive HDR light of the visualizer (ring + baseline + core + shockwaves +
// sparkle). `uv` is screen 0..1; the result is added over the dark backdrop and
// is also sampled through a mirror for the reflective floor.
vec3 sceneEmissive(vec2 uv, float aspect, float aa) {
    vec2 center = vec2(0.5, 0.58);
    vec2 p = uv - center;
    p.x *= aspect;                              // aspect-correct -> true circle

    // slow idle sway so the ring is alive even without audio
    float phi = 0.05 * sin(iTime * 0.20);
    float cs = cos(phi);
    float sn = sin(phi);
    p = mat2(cs, -sn, sn, cs) * p;

    float r = length(p);
    float ang = atan(p.x, p.y);                 // 0 at top, +/- to the sides
    float t = abs(ang) / PI;                    // 0..1, mirrored across vertical

    // --- ring bars (round-capped capsules in a radial frame) -------------
    float N = 40.0;                             // bars per half -> 80 around
    float seg = t * N;
    float idx = floor(seg);
    float binf = (idx + 0.5) / N;               // bass at top -> treble at base
    float amp = barHeight(binf, idx / N);

    float R0 = 0.170;                           // ring baseline radius
    float maxLen = 0.140;
    float Rtop = R0 + amp * maxLen;

    float segAng = PI / N;
    float u = fract(seg) - 0.5;                 // -0.5..0.5 within the segment
    float lateral = r * u * segAng;             // ~perp distance to centerline
    float halfW = 0.0052;                       // constant world half-width

    float alongR = clamp(r, R0, Rtop);
    float d = length(vec2(r - alongR, lateral)) - halfW;   // capsule SDF
    float bar = smoothstep(aa, -aa, d);

    vec3 barCol = palette(t);
    float body = bar * (0.35 + 0.65 * amp);                      // solid core
    float tip  = bar * smoothstep(Rtop - 0.035, Rtop, r)
                     * (1.35 + 1.5 * amp);                       // HDR tip -> bloom
    float halo = exp(-max(d, 0.0) * 95.0) * (0.28 + 0.5 * amp);  // soft neon bleed

    vec3 col = barCol * (body + halo + tip);

    // --- glowing baseline ring -------------------------------------------
    float base = smoothstep(0.0035, 0.0, abs(r - R0));
    col += palette(0.5) * base * (0.6 + 0.8 * iVolume) * 1.30;

    // --- pulsing bass core -----------------------------------------------
    float coreR = 0.086;
    float pulse = 0.42 + 0.80 * iBass + 0.12 * sin(iTime * 1.6);
    float core = smoothstep(coreR, 0.0, r);
    vec3 coreTint = mix(vec3(0.35, 0.75, 1.0), vec3(0.90, 0.35, 0.95),
                        0.5 + 0.5 * sin(iTime * 0.5));
    col += coreTint * core * pulse * 1.55;
    col += vec3(1.0, 0.92, 1.0) * exp(-r * r / 0.0016) * (0.45 + 0.9 * iBass);  // hot center

    // --- shockwave rings on bass (short-lived, subtle) -------------------
    float shock = 0.0;
    for (int i = 0; i < 2; i++) {
        float ph = fract(iTime * 0.55 + float(i) * 0.5);
        float rr = R0 + ph * 0.34;
        shock += smoothstep(0.010, 0.0, abs(r - rr)) * (1.0 - ph);
    }
    col += palette(0.35) * shock * iBass * 1.15;

    // --- sparse drifting sparkle (twinkles with treble, reflects too) ----
    // round points, not cell blocks: mask by distance inside the grid cell
    vec2 sgp = uv * vec2(aspect, 1.0) * 90.0;
    vec2 gp = floor(sgp);
    float spk = pow(hash21(gp), 60.0);
    spk *= smoothstep(0.45, 0.10, length(fract(sgp) - 0.5));
    float tw = 0.5 + 0.5 * sin(iTime * 3.0 + hash21(gp) * 30.0);
    col += vec3(0.6, 0.8, 1.0) * spk * tw * (0.35 + 0.7 * iTreble);

    return col;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float aa = 1.6 / iResolution.y;             // ~1px, in height units

    // --- dark restrained backdrop: radial gradient + faint center haze ----
    vec2 pc = uv - vec2(0.5, 0.58);
    pc.x *= aspect;
    float rad = length(pc);
    vec3 bg = mix(vec3(0.035, 0.030, 0.065), vec3(0.006, 0.006, 0.020),
                  smoothstep(0.05, 0.95, rad));
    bg += vec3(0.10, 0.06, 0.16) * (0.14 + 0.24 * iVolume) * exp(-rad * 3.0);

    float horizon = 0.24;
    vec3 col;
    if (uv.y > horizon) {
        col = bg + sceneEmissive(uv, aspect, aa);
    } else {
        // wet reflective floor: mirror the scene across the horizon, then
        // ripple it and fade with depth for a polished-glass look.
        float depth = horizon - uv.y;
        vec2 muv = vec2(uv.x, 2.0 * horizon - uv.y);
        muv.x += 0.010 * sin(uv.y * 55.0 + iTime * 2.0) * (0.4 + 0.8 * iBass);
        vec3 refl = sceneEmissive(muv, aspect, aa);
        float fade = exp(-depth * 5.5);
        vec3 floorBase = mix(vec3(0.020, 0.020, 0.045), vec3(0.0),
                             smoothstep(0.0, 0.25, depth));
        col = floorBase + refl * fade * vec3(0.55, 0.70, 1.0);
    }

    // soft glow seam where the scene meets its reflection — concentrated under
    // the ring, not a hard laser line across the full frame
    float seamX = exp(-abs(uv.x - 0.5) * aspect * 2.2);
    col += palette(0.5) * exp(-abs(uv.y - horizon) * 110.0)
                        * seamX * (0.30 + 0.55 * iVolume);

    // gentle compositional vignette (post adds its own on top)
    float vig = smoothstep(1.15, 0.35, length((uv - 0.5) * vec2(aspect, 1.0)));
    col *= 0.90 + 0.10 * vig;

    // dither to kill banding (+/-0.5/255)
    col += (hash21(fragCoord + iTime) - 0.5) / 255.0;

    fragColor = vec4(col, 1.0);
}
