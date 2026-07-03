// mountains.glsl — "Twilight Ridges": a layered mountain range at dusk.
//
// Six parallax ridgelines recede toward the horizon, each paler than the last
// (aerial perspective), with exponential mist pooling in the valleys between
// them — depth is carried by atmosphere, not detail. A low HDR sun sinks behind
// the range and blooms; a multi-band dusk sky scatters warm light along the
// horizon, stars wheel overhead, and once every half-minute or so a faint
// meteor streaks past. The bass gently swells the afterglow. Fully composed in
// silence. Shadertoy-compatible (engine extension: iBass).

// sin-free hashes — trig hashes collapse into visible blocks on some GPUs.
float hash(float x) {
    x = fract(x * 0.1031);
    x *= x + 33.33;
    x *= x + x;
    return fract(x);
}
float hash2(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// 1D value noise, smoothstep-interpolated.
float vnoise(float x) {
    float i = floor(x);
    float f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(hash(i), hash(i + 1.0), f);
}

// 2D value noise for soft sky haze.
float vnoise2(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2(1.0, 0.0)), f.x),
               mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), f.x), f.y);
}

// Ridged multifractal: sharp, mountain-like crests normalized to 0..1.
float ridged(float x) {
    float s = 0.0;
    float a = 0.5;
    float fq = 1.0;
    float norm = 0.0;
    // 5 octaves, capped growth: the top octaves of a 6x1.97 stack under-sample
    // at 1080p and smear the silhouettes into vertical streaks
    for (int k = 0; k < 5; k++) {
        float n = vnoise(x * fq);
        n = 1.0 - abs(2.0 * n - 1.0);
        s += a * n;
        norm += a;
        fq *= 1.90;
        a *= 0.52;
    }
    return s / norm;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float e = 1.4 / iResolution.y;                    // ~1px silhouette AA band

    float horizon = 0.30;
    vec2 sunp = vec2(0.60, 0.435);   // the disk floats just above the far crest
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));
    float sunAz = exp(-abs(uv.x - sunp.x) * aspect * 0.9);   // warm bias toward the sun column
    float bass = clamp(iBass, 0.0, 1.0);

    // ---- multi-band dusk sky ----
    vec3 sky = mix(vec3(1.10, 0.52, 0.30), vec3(0.66, 0.34, 0.46),
                   smoothstep(0.0, 0.34, uv.y));
    sky = mix(sky, vec3(0.20, 0.17, 0.42), smoothstep(0.30, 0.62, uv.y));
    sky = mix(sky, vec3(0.04, 0.05, 0.15), smoothstep(0.58, 1.0, uv.y));

    // horizon atmospheric scattering: a warm band, brightest toward the sun,
    // lifted a touch by the bass.
    float scatter = exp(-abs(uv.y - horizon) * 6.5);
    sky += vec3(1.05, 0.50, 0.28) * scatter * (0.22 + 0.50 * sunAz) * (0.85 + 0.45 * bass);

    // faint high cirrus for texture in the upper sky
    float cir = vnoise2(vec2(uv.x * 3.0 - iTime * 0.01, uv.y * 6.0));
    sky = mix(sky, sky + vec3(0.10, 0.06, 0.10),
              smoothstep(0.55, 0.90, cir) * smoothstep(0.35, 0.90, uv.y) * 0.5);

    vec3 col = sky;

    // ---- low sun: broad afterglow (bass-swelled) + mid glow + HDR core (→ bloom) ----
    col += vec3(1.15, 0.50, 0.30) * exp(-dsun * 3.0) * (0.45 + 0.55 * bass);
    col += vec3(1.50, 0.80, 0.45) * smoothstep(0.17, 0.0, dsun) * 0.9;
    col += vec3(2.30, 1.55, 0.95) * smoothstep(0.05, 0.0, dsun) * 1.5;

    // ---- stars (upper sky, twinkling; round points, not cell blocks) ----
    vec2 sgp = fragCoord / 3.0;
    vec2 sg = floor(sgp);
    float sh = hash2(sg);
    float star = pow(sh, 42.0);
    star *= smoothstep(0.5, 0.15, length(fract(sgp) - 0.5));
    float tw = 0.55 + 0.45 * sin(iTime * 3.0 + sh * 40.0);
    col += vec3(0.85, 0.92, 1.10) * star * tw * smoothstep(0.40, 0.80, uv.y) * 1.5;

    // ---- occasional meteor (~every 26s, brief and subtle) ----
    float period = 26.0;
    float cyc = floor(iTime / period);
    float mt = mod(iTime, period);
    float mActive = smoothstep(0.0, 0.06, mt) * smoothstep(1.5, 0.9, mt);
    vec2 mDir = vec2(0.864, -0.504);                  // normalized heading, down-right
    vec2 mPerp = vec2(0.504, 0.864);
    vec2 mStart = vec2(0.10 + 0.70 * hash(cyc * 3.13), 0.96);
    vec2 mHead = mStart + mDir * (mt * 0.50);
    vec2 mrel = (uv - mHead) * vec2(aspect, 1.0);
    float mAlong = dot(mrel, -mDir);                  // >0 = behind the head (tail)
    float mPerpD = dot(mrel, mPerp);
    float meteor = smoothstep(0.02, 0.0, abs(mPerpD)) *
                   smoothstep(0.0, 0.015, mAlong) * smoothstep(0.22, 0.0, mAlong);
    col += vec3(0.90, 0.95, 1.15) * meteor * mActive * smoothstep(0.35, 0.70, uv.y) * 1.3;

    // ---- six parallax ridges (far → near) with valley mist between them ----
    for (int L = 0; L < 6; L++) {
        float fl = float(L);
        float t = fl / 5.0;                           // 0 far → 1 near
        float base = 0.40 - fl * 0.060;               // far high (at horizon) → near low
        float amp  = 0.030 + fl * 0.032;
        float fq   = 1.6 + fl * 2.0;
        float drift = iTime * (0.004 + fl * 0.0025);
        float h = base + amp * ridged(uv.x * fq + fl * 23.3 + drift);

        // conifer micro-roughness hint on the nearest ridges (kept low-frequency
        // enough to stay well-sampled — aliasing here reads as vertical noise)
        float nearF = smoothstep(3.0, 5.0, fl);
        h += nearF * 0.008 * (ridged(uv.x * fq * 3.0 + fl * 5.0) - 0.5);

        // exponential valley mist hugging this crest (far layers softer & taller)
        float above = max(uv.y - h, 0.0);
        float fogK = 34.0 - 18.0 * t;
        float fog = exp(-above * fogK) * (0.25 + 0.55 * (1.0 - t));
        vec3 mist = mix(vec3(0.44, 0.39, 0.52), vec3(0.95, 0.60, 0.42),
                        sunAz * (0.5 + 0.5 * (1.0 - t)));
        col = mix(col, mist, clamp(fog, 0.0, 0.85));

        // silhouette with aerial perspective + warm, sun-facing crest rim
        float fill = smoothstep(h + e, h - e, uv.y);
        vec3 mc = mix(vec3(0.50, 0.42, 0.52), vec3(0.03, 0.035, 0.08), t);
        mc = mix(mc, vec3(1.05, 0.58, 0.38),
                 smoothstep(h - 0.03, h, uv.y) * (0.55 - 0.35 * t) * sunAz);
        col = mix(col, mc, fill);
    }

    // gentle foreground settle so the base doesn't read flat
    col *= 1.0 - 0.12 * smoothstep(0.25, 0.0, uv.y);

    col += (hash2(fragCoord + iTime) - 0.5) / 255.0;   // dither out banding
    fragColor = vec4(col, 1.0);
}
