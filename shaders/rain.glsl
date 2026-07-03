// rain.glsl — "Rain on the Window": raindrops running down glass at night.
//
// Beyond the pane, a defocused city glows in procedural bokeh — cool teal
// windows against warm sodium street lamps. On the glass, static condensation
// stipples the view while heavy drops slide down under gravity, each a little
// lens that refracts the city behind it: the background function is simply
// re-evaluated at the drop's displaced coordinate (kept cheap for exactly this
// reason). Distant rain streaks fall outside, and a hard bass hit throws an HDR
// lightning flash across the sky (a rare bolt still fires in silence).
// Leisurely, and complete without any audio. Shadertoy-compatible (ext: iBass).

// sin-free hash — trig hashes collapse into visible blocks on some GPUs.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Defocused city lights: a jittered grid of soft bokeh disks, warm vs. cool.
// Searches a 3x3 cell neighbourhood so glows spill naturally across cells.
vec3 bokeh(vec2 uv, float tm) {
    vec3 c = vec3(0.0);
    vec2 g = vec2(9.0, 6.0);
    vec2 gp = uv * g;
    vec2 id = floor(gp);
    vec2 f = fract(gp);
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            vec2 o = vec2(float(i), float(j));
            vec2 cid = id + o;
            float present = step(0.55, hash(cid + vec2(1.7, 9.3)));
            vec2 lp = o + vec2(0.20 + 0.60 * hash(cid + vec2(5.2, 1.3)),
                               0.20 + 0.60 * hash(cid + vec2(1.9, 8.7)));
            float dist = length(f - lp);
            float rad = 0.30 + 0.35 * hash(cid + vec2(7.1, 2.9));   // varied bokeh sizes
            float glow = smoothstep(rad, 0.0, dist);
            float warm = step(0.45, hash(cid + vec2(3.3, 6.6)));
            vec3 lc = mix(vec3(0.30, 0.62, 0.85), vec3(1.00, 0.62, 0.26), warm);
            float flick = 0.75 + 0.25 * sin(tm * 1.5 + hash(cid) * 30.0);
            c += lc * glow * glow * present * flick;
        }
    }
    return c;
}

// The night city behind the glass — cheap enough to re-evaluate for refraction.
vec3 cityBg(vec2 uv, float tm) {
    float y = clamp(uv.y, 0.0, 1.0);
    vec3 base = mix(vec3(0.030, 0.070, 0.095), vec3(0.040, 0.095, 0.130), y);
    base += vec3(0.05, 0.11, 0.12) * smoothstep(0.5, -0.1, uv.y) * 0.7;   // low ground glow
    vec3 lights = bokeh(uv, tm) * 1.5;
    lights *= 0.45 + 0.95 * smoothstep(0.8, 0.0, uv.y);                   // denser near street
    return base + lights;
}

// One layer of gravity-fed drops.
// Returns (refraction offset.xy in glass-space, body mask, specular glint).
vec4 dropLayer(vec2 q, float cols, float speed, float seed) {
    float colId = floor(q.x * cols + seed * 3.7);
    float fx = fract(q.x * cols) - 0.5;
    float ch = hash(vec2(colId, seed));
    // stream coordinate; +iTime => features slide downward
    float sy = q.y * cols * 0.6 + iTime * speed * (0.6 + 0.8 * ch) + ch * 20.0;
    float rowId = floor(sy);
    float fy = fract(sy) - 0.5;
    float rh = hash(vec2(colId * 1.7, rowId));
    // wobble + radius must stay inside the half-cell, or drops get sliced by
    // the column boundary and the refraction field tears visibly
    float wob = 0.10 * sin(sy * 1.3 + ch * 25.0);
    vec2 d = vec2(fx - wob, fy);
    float rad = 0.13 + 0.10 * rh;
    // sparse: most stream cells carry no drop at all
    float present = step(0.72, rh);
    float body = smoothstep(rad, rad * 0.55, length(vec2(d.x, d.y * 0.85))) * present;
    // short broken bead trail just above the head — it must die out quickly or
    // the stacked rows read as full-height vertical stripes
    float trail = smoothstep(0.035, 0.0, abs(d.x))
                * smoothstep(0.0, 0.08, fy) * smoothstep(0.50, 0.12, fy)
                * max(0.0, 0.35 + 0.65 * sin(sy * 26.0 + rh * 10.0))
                * step(0.35, ch);
    float m = max(body, trail * 0.30);
    vec2 off = d * m;                                       // lens displacement
    // a small off-centre sparkle, not a blob highlight
    float glint = body * smoothstep(0.06, 0.0, length(d - vec2(-0.06, 0.08)));
    return vec4(off, m, glint);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float bass = clamp(iBass, 0.0, 1.0);
    vec2 q = vec2(uv.x * aspect, uv.y);                     // aspect-corrected glass space

    // ---- glass: two drop layers + static micro condensation ----
    vec4 dA = dropLayer(q, 6.5, 0.10, 1.0);
    vec4 dB = dropLayer(q, 11.0, 0.15, 5.0);
    vec2 bigOff = dA.xy + dB.xy * 0.7;
    float dmask = max(dA.z, dB.z);
    float dglint = max(dA.w, dB.w);

    // static micro-droplets: sparse, jittered, size-varied — a regular lattice
    // of same-size dots reads as a halftone screen, so every parameter is
    // randomised per cell and only a few cells hold a droplet at all
    vec2 mg = q * 70.0;
    vec2 mcell = floor(mg);
    vec2 mjit = vec2(hash(mcell + 7.7), hash(mcell + 3.3)) - 0.5;
    vec2 md = fract(mg) - 0.5 - mjit * 0.6;
    float mh = hash(mcell);
    float msize = 0.10 + 0.12 * hash(mcell + 9.1);
    float mdrop = smoothstep(msize, msize * 0.35, length(md)) * step(0.90, mh);
    vec2 microOff = md * mdrop * 0.35;

    // total refraction offset (glass-space) → uv-space; lens pulls the far side in
    vec2 offQ = bigOff * 0.85 + microOff;
    vec2 refr = vec2(offQ.x / aspect, offQ.y) * 0.22;
    vec2 suv = uv - refr;

    // ---- refracted view of the city behind the glass ----
    vec3 col = cityBg(suv, iTime);

    // ---- distant rain streaks (outside the glass, faint, wind-slanted) ----
    float streaks = 0.0;
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        float cols = 120.0 + fk * 90.0;
        vec2 rp = vec2(uv.x * aspect + uv.y * 0.14, uv.y);
        float x = floor(rp.x * cols);
        float sp = 1.4 + hash(vec2(x, fk)) * 1.6;
        float yy = fract(rp.y * 1.5 + iTime * sp + hash(vec2(x, fk + 5.0)) * 10.0);
        float st = smoothstep(0.0, 0.02, yy) * smoothstep(0.30, 0.0, yy);
        streaks += st * (0.25 + 0.5 * hash(vec2(x, fk + 2.0))) * (1.0 - fk * 0.25);
    }
    col += vec3(0.45, 0.60, 0.85) * streaks * 0.07 * (1.0 - dmask * 0.7);

    // ---- glass shading: wet body, drop glints, condensation sparkle ----
    col *= 1.0 - 0.05 * dmask;                              // wet body reads slightly darker
    // glints are reflections of the city lights — bright over bokeh, nearly
    // invisible against dark sky (free-floating white specks read as snow)
    float bgLum = dot(col, vec3(0.299, 0.587, 0.114));
    col += vec3(1.00, 1.00, 1.05) * dglint * (0.15 + 2.2 * bgLum);
    float microSpark = mdrop * smoothstep(0.06, 0.0, length(md - vec2(-0.04, 0.05)));
    col += vec3(0.80, 0.90, 1.00) * microSpark * 0.3;

    // faint condensation fog toward the top of the pane
    float fog = smoothstep(0.55, 1.0, uv.y) * 0.10;
    col = mix(col, vec3(0.12, 0.18, 0.22), fog * (1.0 - dmask));

    // ---- lightning: a hard bass hit flashes the sky; a rare auto-bolt in silence ----
    float autoT = mod(iTime, 14.0);
    float autoBolt = smoothstep(0.0, 0.04, autoT) * smoothstep(0.40, 0.06, autoT) * 0.35;
    float bassBolt = smoothstep(0.55, 1.0, bass) * (0.6 + 0.4 * sin(iTime * 47.0));
    float bolt = max(autoBolt, bassBolt);
    col += vec3(0.70, 0.80, 1.10) * bolt * (0.45 + 0.70 * smoothstep(1.0, 0.15, uv.y));

    col += (hash(fragCoord + iTime) - 0.5) / 255.0;         // dither out banding
    fragColor = vec4(col, 1.0);
}
