// sunset_clouds.glsl — back-lit clouds over a dusk sky.
// Three parallax cloud layers, each built from domain-warped fbm for volume,
// drift slowly across a gradient that cools from a warm horizon into deep dusk
// blue. Thin cloud edges facing the sun glow with a silver-gold lining, and a
// short screen-space march toward the sun casts crepuscular god-ray shafts that
// are broken up by the clouds. A few early stars come out at the top. The sun
// halo brightens softly with iTreble; everything else drifts on its own slow
// clock so the wallpaper never gets busy. Fully procedural — no textures.
// Shadertoy-compatible (engine extension: iTreble).

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
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int k = 0; k < 6; k++) {
        s += a * noise(p);
        p = rot(0.5) * p * 2.03 + vec2(11.3, 5.7);
        a *= 0.5;
    }
    return s;
}

// Domain-warped cloud density in [0,1]-ish. One warp stage keeps it billowy
// without a second nested fbm — two fbm calls per layer.
float cloudDensity(vec2 p, float t) {
    float w = fbm(p * 0.6 + vec2(t * 0.02, t * 0.006));
    return fbm(p + vec2(w * 1.6, w * 1.1));
}

// Crepuscular shafts: march a handful of steps from the pixel toward the sun,
// accumulating how much of the path stays clear of cloud. Bright, streaky where
// the line to the sun threads a gap; dark where a cloud blocks it. The occluder
// is a cheap single-octave proxy — the rays are soft, so the mismatch with the
// drawn clouds never shows.
float godRays(vec2 uv, vec2 sun, float aspect, float t) {
    vec2 march = (sun - uv) / 12.0;
    vec2 sp = uv;
    float light = 0.0;
    for (int i = 0; i < 12; i++) {
        sp += march;
        float c = noise(vec2(sp.x * aspect * 3.4 - t * 0.03, sp.y * 3.4 + 4.0));
        float occ = smoothstep(0.52, 0.86, c);
        float w = 1.0 - float(i) / 12.0;       // weight light nearer the sun
        light += (1.0 - occ) * w;
    }
    return light / 12.0;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float t = iTime;
    float treble = iTreble;

    vec2 sunp = vec2(0.5, 0.26);
    float dsun = length(vec2((uv.x - sunp.x) * aspect, uv.y - sunp.y));

    // --- sky: warm horizon → cool dusk blue → dark zenith --------------------
    vec3 sky = mix(vec3(1.00, 0.52, 0.26), vec3(0.70, 0.38, 0.42), smoothstep(0.0, 0.32, uv.y));
    sky = mix(sky, vec3(0.12, 0.13, 0.34), smoothstep(0.30, 0.80, uv.y));
    sky = mix(sky, vec3(0.035, 0.045, 0.15), smoothstep(0.65, 1.0, uv.y));
    sky += vec3(1.05, 0.58, 0.30) * smoothstep(0.70, 0.0, dsun) * 0.26;   // sun scatter
    vec3 col = sky;

    // --- a few early stars, only in the dark upper sky -----------------------
    vec2 sg = floor(fragCoord / 3.0);
    float star = pow(hash(sg), 62.0);
    float tw = 0.5 + 0.5 * sin(t * 2.0 + hash(sg) * 40.0);
    col += vec3(star * tw) * smoothstep(0.55, 1.0, uv.y) * 0.8;

    // --- crepuscular god-ray shafts (behind the clouds) ----------------------
    float rays = godRays(uv, sunp, aspect, t);
    col += vec3(1.20, 0.78, 0.42) * rays * smoothstep(0.95, 0.0, dsun) * 0.35;

    // --- three parallax cloud layers, far/high → near/low --------------------
    for (int L = 0; L < 3; L++) {
        float fl = float(L);
        float sc = 1.7 + fl * 1.5;                       // big, chunky decks
        vec2 p = vec2(uv.x * aspect * sc - t * (0.010 + fl * 0.012),
                      uv.y * sc + fl * 8.0 + t * 0.004);
        // stretch the density range so dense cores actually form (raw fbm
        // hovers near 0.5 and otherwise never leaves the lining window)
        float c = cloudDensity(p, t) * 1.45 - 0.16;

        float lo = 0.55;
        float hi = 0.72;
        // each layer lives in its own altitude band so the upper dusk sky and
        // the horizon glow both stay visible between the cloud decks
        float band = smoothstep(0.03, 0.20, uv.y)
                   * (1.0 - smoothstep(0.62 - fl * 0.10, 0.95 - fl * 0.12, uv.y));
        float cover = smoothstep(lo, hi - 0.02, c) * band;
        // silver lining: thin cloud transmits back-light, strongest toward the sun
        float thin = smoothstep(hi + 0.06, lo, c);
        float lining = cover * thin * smoothstep(0.85, 0.0, dsun);

        vec3 body   = mix(vec3(0.16, 0.11, 0.21), vec3(0.06, 0.05, 0.13), fl * 0.5);
        vec3 silver = mix(vec3(1.35, 0.85, 0.52), vec3(1.70, 1.10, 0.72), smoothstep(0.5, 0.0, dsun));
        vec3 cloud  = mix(body, silver, clamp(lining * 2.2, 0.0, 1.0));

        col = mix(col, cloud, cover * (0.92 - fl * 0.16));
    }

    // --- sun disk: bright HDR core (bloom) + halo that lifts with treble ------
    col += vec3(1.60, 1.05, 0.60) * smoothstep(0.060, 0.0, dsun) * 2.2;
    col += vec3(1.25, 0.65, 0.38) * smoothstep(0.30, 0.0, dsun) * (0.50 + 0.50 * treble);

    col += (hash(fragCoord + t) - 0.5) / 255.0;    // dither out banding
    fragColor = vec4(col, 1.0);
}
