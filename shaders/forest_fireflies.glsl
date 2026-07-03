// forest_fireflies.glsl — a misty pine forest at night, alive with fireflies.
// Three parallax tree-lines and moonlit ground fog build real depth; a bank of
// volumetric moonlight shafts slants down through the canopy and slowly shimmers.
// Three depth-sorted firefly swarms drift on curved flight paths, each blinking
// on its own slow rhythm; their cores are pushed into HDR so the engine's bloom
// haloes them into soft bokeh. The swarm quietly gains life with iVolume, but the
// frame is complete in silence. Palette: deep teal night vs. warm chartreuse glow.
// Engine extension used: iVolume. Shadertoy-compatible otherwise.

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

float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int k = 0; k < 5; k++) {
        s += a * noise(p);
        p = p * 2.02 + vec2(7.3, 4.1);
        a *= 0.5;
    }
    return s;
}

// A silhouetted tree-line. Returns the tree colour in .xyz and its coverage in
// .w. `depthY` is the canopy base height, `amp`/`freq` shape the jagged top, and
// `body` is the layer's silhouette colour (darker toward the front).
vec4 treeLayer(vec2 uv, float dmoon, float depthY, float amp, float freq,
               float seed, vec3 body) {
    // fbm canopy line + high-frequency conifer spikes; trees fill from the
    // bottom of the frame up to `edge`
    float edge = depthY + amp * (fbm(vec2(uv.x * freq + seed, seed * 0.3)) - 0.5) * 2.0;
    edge += pow(noise(vec2(uv.x * 55.0 + seed * 2.0, seed)), 2.0) * amp * 0.55;
    float cov = 1.0 - smoothstep(edge - 0.010, edge + 0.010, uv.y);
    // faint vertical trunk / foliage grain inside the mass
    float grain = 0.82 + 0.18 * noise(vec2(uv.x * 55.0 + seed, uv.y * 7.0));
    vec3 tcol = body * grain;
    // cool moonlit rim skimming the canopy top, strongest toward the moon
    float rim = smoothstep(0.020, 0.0, abs(uv.y - edge));
    tcol += vec3(0.10, 0.17, 0.22) * rim * smoothstep(0.9, 0.0, dmoon);
    return vec4(tcol, cov);
}

// One firefly's additive HDR glow. `id` seeds its home, drift and blink; `depth`
// (0 far … 1 near) sets its size, softness and how hard the core blooms.
vec3 fireflyGlow(vec2 uv, float aspect, float id, float depth) {
    float t = iTime;
    // home spread a little beyond the frame horizontally, but kept to the
    // forest band vertically — fireflies at the zenith read as UFOs
    vec2 home = vec2(hash(vec2(id, 1.7)) * 1.2 - 0.1,
                     0.04 + hash(vec2(id, 9.1)) * 0.60);
    // curved flight: two out-of-phase sines per axis trace a slow looping drift
    float sp = 0.35 + 0.5 * hash(vec2(id, 4.3));
    vec2 drift = vec2(
        sin(t * sp * 0.31 + id * 2.0) + 0.6 * sin(t * sp * 0.17 + id * 5.0),
        sin(t * sp * 0.27 + id * 3.1) + 0.6 * cos(t * sp * 0.13 + id * 1.7));
    vec2 pos = home + drift * mix(0.04, 0.12, depth);

    float d = length((uv - pos) * vec2(aspect, 1.0));
    float size = mix(0.010, 0.055, depth);          // near = large soft bokeh
    float coreR = mix(0.0016, 0.010, depth);
    float core = coreR * coreR / (d * d + coreR * coreR * 0.30);
    float halo = smoothstep(size, 0.0, d);

    // blink: a slow charge-and-fade, amplitude-modulated so some pulses stay dim
    float rate = 0.6 + 0.9 * hash(vec2(id, 2.2));
    float ph = id * 6.2831;
    float b = 0.5 + 0.5 * sin(t * rate + ph);
    b *= 0.55 + 0.45 * sin(t * rate * 0.37 + ph * 1.7);
    b = pow(clamp(b, 0.0, 1.0), 1.7);

    float warm = hash(vec2(id, 5.5));
    vec3 fcol = mix(vec3(0.55, 1.00, 0.32), vec3(0.95, 0.92, 0.38), warm);
    float peak = mix(2.7, 1.5, depth);              // distant cores bloom hardest
    return fcol * (core * peak + halo * mix(0.20, 0.60, depth)) * b;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float t = iTime;
    float volLift = 0.78 + 0.55 * iVolume;          // swarm quietly breathes with volume

    // ---- night sky: deep teal, a touch brighter toward the moon ------------
    vec2 moon = vec2(0.70, 0.82);
    float dmoon = length((uv - moon) * vec2(aspect, 1.0));
    vec3 col = mix(vec3(0.014, 0.048, 0.052), vec3(0.028, 0.090, 0.120),
                   smoothstep(0.0, 1.0, uv.y));
    col += vec3(0.06, 0.12, 0.16) * smoothstep(0.9, 0.0, dmoon);   // soft moon wash

    // ---- stars: sparse round points, high in the sky, gently twinkling -----
    vec2 sg = uv * vec2(aspect, 1.0) * 210.0;
    vec2 sc = floor(sg);
    float star = pow(hash(sc), 220.0);
    star *= smoothstep(0.5, 0.15, length(fract(sg) - 0.5));   // round, not a cell block
    star *= smoothstep(0.45, 0.95, uv.y);
    star *= 0.5 + 0.5 * sin(t * 2.5 + hash(sc) * 40.0);
    col += vec3(0.55, 0.72, 0.95) * star * 1.1;

    // ---- moon: layered soft halo and an HDR disk to drive the bloom --------
    // (no hard ring — a crisp circle reads as a lens-flare bug, not a halo)
    col += vec3(0.30, 0.48, 0.75) * smoothstep(0.30, 0.0, dmoon) * 0.35;
    col += vec3(0.40, 0.60, 0.90) * smoothstep(0.12, 0.0, dmoon) * 0.55;
    col += vec3(1.35, 1.45, 1.55) * smoothstep(0.052, 0.038, dmoon) * 1.6;

    // ---- volumetric moonlight shafts: slanting, slowly shimmering ----------
    vec2 ld = normalize(vec2(-0.55, -1.0));         // light travels down-left
    vec2 rel = uv - moon;
    float along = dot(rel, ld);
    float across = dot(rel, vec2(-ld.y, ld.x));
    float beams = 0.5 + 0.5 * sin(across * 26.0 + sin(t * 0.20) * 1.5);
    beams *= 0.5 + 0.5 * sin(across * 11.0 - t * 0.13);
    beams *= smoothstep(-0.05, 0.5, along);         // begin just below the moon
    beams *= smoothstep(1.2, 0.1, along);           // dissolve toward the floor
    beams *= 0.55 + 0.45 * fbm(vec2(across * 3.0, along * 2.0 - t * 0.10));
    col += vec3(0.16, 0.28, 0.34) * beams * 0.85;

    // ---- far fireflies: they sit behind the trees --------------------------
    vec3 ff = vec3(0.0);
    for (int i = 0; i < 22; i++) {
        float id = float(i) + 3.0;
        ff += fireflyGlow(uv, aspect, id, 0.10 + 0.18 * hash(vec2(id, 7.0)));
    }
    col += ff * volLift;

    // ---- back tree-line ----------------------------------------------------
    vec4 tb = treeLayer(uv, dmoon, 0.40, 0.06, 2.5, 11.0, vec3(0.030, 0.080, 0.088));
    col = mix(col, tb.xyz, tb.w);

    // ---- mid fireflies -----------------------------------------------------
    ff = vec3(0.0);
    for (int i = 0; i < 14; i++) {
        float id = float(i) + 40.0;
        ff += fireflyGlow(uv, aspect, id, 0.38 + 0.22 * hash(vec2(id, 7.0)));
    }
    col += ff * volLift;

    // ---- mid tree-line -----------------------------------------------------
    vec4 tm = treeLayer(uv, dmoon, 0.31, 0.08, 3.5, 27.0, vec3(0.018, 0.052, 0.058));
    col = mix(col, tm.xyz, tm.w);

    // ---- moonlit ground fog drifting between the trunks --------------------
    float fog = smoothstep(0.42, 0.90, fbm(vec2(uv.x * 3.0 - t * 0.02, uv.y * 5.0 + t * 0.03)));
    float band = smoothstep(0.55, 0.25, uv.y) * smoothstep(0.02, 0.22, uv.y);
    col = mix(col, vec3(0.11, 0.23, 0.25), fog * band * 0.65);

    // ---- front tree-line ---------------------------------------------------
    vec4 tf = treeLayer(uv, dmoon, 0.22, 0.10, 4.5, 41.0, vec3(0.010, 0.030, 0.032));
    col = mix(col, tf.xyz, tf.w);

    // ---- near fireflies: big soft bokeh floating in front ------------------
    ff = vec3(0.0);
    for (int i = 0; i < 8; i++) {
        float id = float(i) + 70.0;
        ff += fireflyGlow(uv, aspect, id, 0.74 + 0.22 * hash(vec2(id, 7.0)));
    }
    col += ff * volLift;

    col += (hash(fragCoord + t) - 0.5) / 255.0;      // dither out banding
    fragColor = vec4(max(col, vec3(0.0)), 1.0);
}
