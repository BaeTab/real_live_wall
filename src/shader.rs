//! Shader sources and the Shadertoy → engine GLSL wrapper.
//!
//! Every scene is drawn as a full-screen triangle: one WGSL vertex shader that
//! emits three oversized clip-space vertices, plus a fragment shader that is
//! either the built-in WGSL scene or a user-supplied Shadertoy GLSL shader.

/// Full-screen triangle vertex shader (shared by every scene, WGSL or GLSL).
pub const FULLSCREEN_VS: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(p[vid], 0.0, 1.0);
}
"#;

/// Built-in default scene: "Aurora Borealis" — a night-sky aurora over a
/// mountain lake with a 64-band spectrum equalizer on the near shore.
/// Rendered to an Rgba16Float HDR target; emitters push past 1.0 to drive the
/// bright-pass + bloom stage, while the average luminance stays low (night).
/// Guaranteed to compile so the engine always shows something on first run.
pub const DEFAULT_WGSL_FS: &str = r#"
struct Uniforms {
    resolution: vec4<f32>,
    mouse: vec4<f32>,
    time: vec4<f32>,
    audio: vec4<f32>,
    sys: vec4<f32>,
    date: vec4<f32>,
    beat: vec4<f32>,                     // x=pulse, y=bpm, z=confidence, w=onset
    media: vec4<f32>,                    // x=hasMusic, y=isPlaying, z=trackPulse
    palette: array<vec4<f32>, 4>,        // album-art colours (xyz=rgb, w=weight)
    spectrum: array<vec4<f32>, 16>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

// Dynamic index into the uniform spectrum array (this split form is what naga
// accepts for a uniform array<vec4> lookup).
fn spec(x: f32) -> f32 {
    let i = i32(clamp(x, 0.0, 1.0) * 63.0);
    return u.spectrum[i / 4][i % 4];
}

// sin-free hashes (Hoskins) — trig-based hashes collapse into visible blocks
// on GPUs whose fp32 sin loses precision for large arguments.
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    let w = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var v = 0.0;
    var a = 0.5;
    for (var k = 0; k < 5; k = k + 1) {
        v = v + a * vnoise(p);
        // rotate + scale each octave so the value-noise lattice never lines up
        p = vec2<f32>(1.6 * p.x + 1.2 * p.y, -1.2 * p.x + 1.6 * p.y) + vec2<f32>(3.7, 1.7);
        a = a * 0.5;
    }
    return v;
}

// Unsigned distance from point p to the segment a-b (used for equalizer bars).
fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / (dot(ba, ba) + 1e-4), 0.0, 1.0);
    return length(pa - ba * h);
}

// One aurora curtain: emission intensity at aspect-corrected x = cx0 and
// height sky (0 at the horizon, 1 at the top of the frame). The curtain hangs
// from a meandering foot line; its rays lean sideways as they climb.
fn curtain(cx0: f32, sky: f32, t: f32, seed: f32, foot_h: f32, warp: f32, lean: f32) -> f32 {
    let above = sky - (foot_h
        + sin(cx0 * 1.05 + t * 0.10 + seed) * 0.090
        + sin(cx0 * 0.43 - t * 0.063 + seed * 1.7) * 0.060
        + warp * 0.20);
    // bright glowing foot, exponential fade upward, soft cut below the foot
    let env = exp(-max(above, 0.0) * 3.6) * smoothstep(-0.06, 0.02, above);
    let cx = cx0 + max(above, 0.0) * lean;
    // near-vertical ray columns: broad brightness variation x fine streaks.
    // Multiplying keeps the envelope smooth (no triangular tips); deep contrast
    // leaves genuinely dark gaps so the sky shows through the curtain.
    let r1 = fbm(vec2<f32>(cx * 2.2 + seed * 3.0 - t * 0.030, sky * 0.35 + t * 0.020));
    let r2 = vnoise(vec2<f32>(cx * 9.0 + seed * 9.0 + t * 0.045, sky * 0.8));
    let rays = (0.22 + 0.90 * smoothstep(0.35, 0.85, r1)) * (0.55 + 0.60 * r2);
    return env * rays;
}

// Accumulated HDR aurora colour from three curtains. Each curtain runs its own
// green foot -> teal body -> violet crown ramp so every band reads distinctly.
// Reacts subtly to bass.
fn aurora_color(cx: f32, sky: f32, t: f32, bass: f32) -> vec3<f32> {
    // the whole system drifts very slowly across the sky
    let dx = cx + t * 0.008;
    let warp = fbm(vec2<f32>(dx * 0.6 + 4.0, sky * 0.5 + t * 0.05)) - 0.5;
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    for (var k = 0; k < 3; k = k + 1) {
        let fk = f32(k);
        let foot = 0.36 + fk * 0.17;
        let ity = curtain(dx, sky, t, fk * 3.7, foot, warp, 0.30 - fk * 0.25)
                * (1.0 - fk * 0.24);
        let hgt = clamp((sky - foot) * 2.4, 0.0, 1.0);
        var hue = mix(vec3<f32>(0.15, 1.05, 0.40), vec3<f32>(0.10, 0.75, 0.90),
                      smoothstep(0.10, 0.55, hgt));
        hue = mix(hue, vec3<f32>(0.60, 0.25, 1.00), smoothstep(0.50, 1.0, hgt));
        acc = acc + hue * ity;
    }
    return acc * (0.90 + 0.45 * bass) * 1.05;
}

// One grid layer of procedural stars (additive, slightly tinted).
fn star_layer(uv: vec2<f32>, aspect: f32, t: f32, density: f32, size: f32,
              thresh: f32, twinkle: f32, spikes: f32) -> vec3<f32> {
    let p = vec2<f32>(uv.x * aspect, uv.y) * density;
    let id = floor(p);
    let gv = fract(p) - 0.5;
    let rnd = hash22(id + 3.1) - 0.5;
    let bright = hash(id + vec2<f32>(11.7, 4.3));
    let present = smoothstep(thresh, thresh + 0.02, bright);   // sparse cells only
    let lv = gv - rnd * 0.6;
    let d = length(lv);
    var s = pow(smoothstep(size, 0.0, d), 2.0);
    let tw = 0.60 + twinkle * sin(t * 2.2 + bright * 40.0);
    s = s * tw;
    // thin diffraction spikes, reserved for the very brightest stars
    let big = smoothstep(0.985, 1.0, bright) * spikes;
    let sp = smoothstep(size * 5.0, 0.0, abs(lv.x)) * smoothstep(size * 0.7, 0.0, abs(lv.y))
           + smoothstep(size * 5.0, 0.0, abs(lv.y)) * smoothstep(size * 0.7, 0.0, abs(lv.x));
    s = (s + sp * big * 0.5) * present;
    let tint = mix(vec3<f32>(0.75, 0.85, 1.0), vec3<f32>(1.0, 0.92, 0.82),
                   hash(id + vec2<f32>(1.9, 7.7)));
    return tint * s * (0.30 + 0.9 * bright * bright);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.resolution.xy;
    let uv = frag.xy / res;                 // 0..1, origin top-left
    let t = u.time.x;
    let bass = u.audio.x;
    let treble = u.audio.z;
    let vol = u.audio.w;
    let beat = u.beat.x;                     // decaying pulse on each detected onset
    // how strongly the album-art palette colours the scene (0 when no music)
    let music = u.media.x * (0.35 + 0.65 * u.media.y);
    let aspect = res.x / max(res.y, 1.0);

    let sky = 1.0 - uv.y;                    // 0 at the water line's floor, 1 at the top
    let cx = (uv.x - 0.5) * aspect;          // aspect-corrected horizontal
    let horizon = 0.24;                      // sky value of the water line

    // --- night-sky vertical gradient ---------------------------------------
    var col = mix(vec3<f32>(0.030, 0.045, 0.090),
                  vec3<f32>(0.012, 0.016, 0.045), smoothstep(0.0, 1.0, sky));
    // faint green airglow hugging the horizon
    col = col + vec3<f32>(0.02, 0.07, 0.06)
              * exp(-max(sky - horizon, 0.0) * 5.0) * 0.6;

    // --- milky way band -----------------------------------------------------
    let mband = cx * 0.55 + (uv.y - 0.30);
    let mmask = exp(-mband * mband * 7.0);
    let mcloud = fbm(vec2<f32>(cx * 2.2 + 10.0, uv.y * 3.0 - t * 0.01));
    let milky = mmask * smoothstep(0.25, 0.85, mcloud);
    col = col + vec3<f32>(0.10, 0.11, 0.17) * milky * smoothstep(horizon, 0.60, sky);

    // --- stars (denser inside the milky way, fade toward the horizon) ------
    let star_fade = smoothstep(horizon, horizon + 0.25, sky);
    var starcol = star_layer(uv, aspect, t, 150.0, 0.14, 0.80, 0.10, 0.0) * 0.40;
    starcol = starcol + star_layer(uv, aspect, t, 55.0, 0.10, 0.93, 0.30, 0.0) * 0.85;
    starcol = starcol + star_layer(uv, aspect, t, 20.0, 0.080, 0.96, 0.45, 1.0)
                        * (1.25 + 0.5 * treble);
    col = col + starcol * star_fade * (0.7 + 0.7 * milky);

    // --- aurora curtains (beat gives a gentle brightness kick) --------------
    var aurora = aurora_color(cx, sky, t, bass) * (1.0 + 0.22 * beat);
    // when music is playing, drift the aurora hue toward the album palette
    let apal = mix(u.palette[0].xyz, u.palette[2].xyz, smoothstep(0.3, 0.9, sky));
    let alum = max(aurora.x, max(aurora.y, aurora.z));
    aurora = mix(aurora, apal * alum * 1.4, music * 0.5);
    col = col + aurora;

    // --- distant mountains (atmospheric haze) ------------------------------
    let rf = horizon + 0.050 + 0.085 * fbm(vec2<f32>(cx * 0.8 + 20.0, 3.0));
    let m_far = smoothstep(rf + 0.006, rf - 0.006, sky);
    col = mix(col, vec3<f32>(0.050, 0.068, 0.115), m_far * 0.92);
    col = col + vec3<f32>(0.10, 0.14, 0.20)
              * smoothstep(0.010, 0.0, abs(sky - rf)) * 0.25;     // snow rim on the crest

    // --- near mountains with a conifer-spiked ridge ------------------------
    let trees = pow(vnoise(vec2<f32>(cx * 40.0, 1.0)), 3.0);
    let rn = horizon + 0.016 + 0.070 * fbm(vec2<f32>(cx * 1.5 + 5.0, 0.0)) + 0.014 * trees;
    let m_near = smoothstep(rn + 0.006, rn - 0.006, sky);
    col = mix(col, vec3<f32>(0.010, 0.014, 0.026), m_near);
    // aurora rim light kissing the near crest
    col = col + vec3<f32>(0.10, 0.50, 0.40)
              * smoothstep(0.012, 0.0, abs(sky - rn)) * 0.18;

    // --- lake reflection below the water line ------------------------------
    if (sky < horizon) {
        let depth = horizon - sky;
        let ripple = sin(cx * 30.0 + t * 0.6) * 0.004
                   + sin(cx * 13.0 - t * 0.4) * 0.007 * (1.0 + depth * 4.0);
        // compressed mirror: sample the bright curtain band so the water
        // actually carries aurora light (a flat mirror would only see the
        // empty sky below the curtain feet)
        let refl_sky = horizon + 0.14 + depth * 1.8;
        var lake = vec3<f32>(0.008, 0.016, 0.038);
        lake = lake + aurora_color(cx + ripple * 2.0, refl_sky, t, bass)
                    * exp(-depth * 5.5) * 0.34 * vec3<f32>(0.70, 0.85, 1.0);
        lake = lake + vec3<f32>(0.02, 0.07, 0.06) * exp(-depth * 10.0) * 0.5;
        col = lake;
    }

    // --- 64-band spectrum equalizer on the near shore ----------------------
    let bars = 64.0;
    let bw = res.x / bars;
    let idx = floor(frag.x / bw);
    let sx = (idx + 0.5) / bars;
    let by = res.y * 0.85;                                        // shore baseline (px)
    let bcx = (idx + 0.5) * bw;
    let hw = bw * 0.32;                                           // bar half-width (px)
    // idle shimmer keeps the bars looking alive even in silence
    let idle = 0.020 + 0.012 * sin(t * 1.5 + idx * 0.40)
                     + 0.008 * sin(t * 3.3 - idx * 0.70);
    let amp = max(spec(sx), 0.0);
    let h01 = clamp(pow(max(amp, idle), 0.60), 0.0, 1.25);
    let bh = h01 * res.y * 0.30;
    let top = vec2<f32>(bcx, by - bh);
    let base = vec2<f32>(bcx, by);
    // colour climbs the bar through the aurora palette
    let hy = clamp((by - frag.y) / (res.y * 0.30), 0.0, 1.0);
    var bar_col = mix(vec3<f32>(0.10, 1.0, 0.50), vec3<f32>(0.20, 0.80, 1.0),
                      smoothstep(0.0, 0.45, hy));
    bar_col = mix(bar_col, vec3<f32>(0.75, 0.35, 1.0), smoothstep(0.40, 0.85, hy));
    // with music, recolour the bars from the album palette (base→tip = col0→col2)
    let palBar = mix(u.palette[0].xyz, u.palette[2].xyz, smoothstep(0.0, 0.9, hy));
    bar_col = mix(bar_col, palBar, music * 0.6);
    let emis = (0.55 + 1.0 * h01) * (1.0 + 0.35 * beat);
    // upright rounded bar (capsule) with anti-aliased edge
    let sd_up = sd_segment(frag.xy, base, top) - hw;
    let up_mask = smoothstep(1.5, -1.0, sd_up);
    col = mix(col, bar_col * emis, up_mask);
    // HDR tip glow -> bloom
    let tipd = length(frag.xy - top);
    col = col + bar_col * exp(-tipd / (hw * 1.5)) * (0.6 + 1.3 * h01) * 0.7;
    // mirror reflection into the lake, faded with depth
    let mp = vec2<f32>(frag.x, 2.0 * by - frag.y);
    let sd_rf = sd_segment(mp, base, top) - hw;
    let rf_mask = smoothstep(1.5, -1.0, sd_rf);
    let rdepth = max(frag.y - by, 0.0) / res.y;
    col = col + bar_col * emis * rf_mask * exp(-rdepth * 9.0) * 0.45;

    // --- subtle vignette + volume lift -------------------------------------
    let vig = smoothstep(1.35, 0.35, length((uv - 0.5) * vec2<f32>(aspect, 1.0)));
    col = col * (0.82 + 0.18 * vig);
    col = col * (1.0 + 0.10 * vol);

    // --- dithering to kill 8-bit banding -----------------------------------
    let dither = (hash(frag.xy + t) - 0.5) / 255.0;
    col = col + vec3<f32>(dither);

    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
"#;

/// GLSL preamble injected before user Shadertoy code. Declares the shared
/// uniform block and re-creates the Shadertoy uniform names via `#define`,
/// plus engine extensions (`iBass`, `iMid`, `iTreble`, `iVolume`, `iSpectrum`).
const GLSL_PREAMBLE: &str = r#"#version 450
layout(location = 0) out vec4 rlw_fragColor;

layout(std140, set = 0, binding = 0) uniform Uniforms {
    vec4 resolution;
    vec4 mouse;
    vec4 time;
    vec4 audio;
    vec4 sys;
    vec4 date;
    vec4 beat;
    vec4 media;
    vec4 palette[4];
    vec4 spectrum[16];
} rlw;

#define iResolution   rlw.resolution.xyz
#define iTime         rlw.time.x
#define iTimeDelta    rlw.time.y
#define iFrame        int(rlw.time.z)
#define iFrameRate    rlw.sys.w
#define iSampleRate   rlw.time.w
#define iMouse        rlw.mouse
#define iDate         rlw.date
// --- engine extensions (not on Shadertoy, but handy for reactive wallpapers)
#define iBass         rlw.audio.x
#define iMid          rlw.audio.y
#define iTreble       rlw.audio.z
#define iVolume       rlw.audio.w
#define iCpu          rlw.sys.x
#define iMem          rlw.sys.y
// --- beat / tempo (onset detection + BPM estimate) ---
#define iBeat         rlw.beat.x
#define iBpm          rlw.beat.y
#define iBeatConf     rlw.beat.z
#define iOnset        rlw.beat.w
// --- now-playing (SMTC): music state + album-art palette ---
#define iHasMusic     rlw.media.x
#define iPlaying      rlw.media.y
#define iTrackChange  rlw.media.z

float iSpectrum(float x) {
    int idx = int(clamp(x, 0.0, 1.0) * 63.0);
    return rlw.spectrum[idx >> 2][idx & 3];
}

// Dominant album-art colour i (0..3); i=0 is the strongest. rgb 0..1.
vec3 iPalette(int i) {
    return rlw.palette[clamp(i, 0, 3)].xyz;
}
"#;

const GLSL_EPILOGUE: &str = r#"
void main() {
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
    // Shadertoy's fragCoord has its origin at the bottom-left; flip Y.
    vec2 fragCoord = vec2(gl_FragCoord.x, rlw.resolution.y - gl_FragCoord.y);
    mainImage(color, fragCoord);
    rlw_fragColor = vec4(color.rgb, 1.0);
}
"#;

/// Wrap a Shadertoy-style GLSL image shader (defining `mainImage`) into a full
/// GLSL 450 fragment shader compatible with wgpu/naga and our uniform layout.
pub fn wrap_shadertoy_glsl(user_source: &str) -> String {
    format!("{GLSL_PREAMBLE}\n// ---- user shader ----\n{user_source}\n// ---- end user shader ----\n{GLSL_EPILOGUE}")
}
