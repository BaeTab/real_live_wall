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

/// Built-in default scene: an audio-reactive aurora with a spectrum equalizer.
/// Guaranteed to compile so the engine always shows something on first run.
pub const DEFAULT_WGSL_FS: &str = r#"
struct Uniforms {
    resolution: vec4<f32>,
    mouse: vec4<f32>,
    time: vec4<f32>,
    audio: vec4<f32>,
    sys: vec4<f32>,
    date: vec4<f32>,
    spectrum: array<vec4<f32>, 16>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

fn spec(x: f32) -> f32 {
    let i = i32(clamp(x, 0.0, 1.0) * 63.0);
    return u.spectrum[i / 4][i % 4];
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 345.45));
    let r = q + dot(q, q + 34.345);
    return fract(r.x * r.y);
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
        p = p * 2.0 + vec2<f32>(37.0, 17.0);
        a = a * 0.5;
    }
    return v;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = u.resolution.xy;
    let uv = frag.xy / res;              // 0..1, origin top-left
    let t = u.time.x;
    let bass = u.audio.x;
    let vol = u.audio.w;

    // --- night-sky gradient -------------------------------------------------
    var col = mix(vec3<f32>(0.02, 0.03, 0.09), vec3<f32>(0.06, 0.02, 0.16), uv.y);

    // --- aurora ribbons (flow noise, react to bass) -------------------------
    let q = vec2<f32>(uv.x * 2.0, uv.y * 1.2);
    let flow = fbm(q * 3.0 + vec2<f32>(0.0, t * 0.15));
    let f2 = fbm(q * 5.0 - vec2<f32>(t * 0.10, flow * 1.5));
    let ribbon = smoothstep(0.35, 0.9, f2) * (0.55 + 1.5 * bass);
    let aurora = mix(vec3<f32>(0.10, 0.95, 0.60), vec3<f32>(0.30, 0.45, 1.0), flow);
    col = col + ribbon * aurora * (1.0 - uv.y);

    // --- stars --------------------------------------------------------------
    let star = pow(hash(floor(frag.xy * 0.5)), 42.0);
    col = col + vec3<f32>(star) * (0.4 + 0.6 * sin(t * 3.0 + uv.x * 60.0)) * (1.0 - uv.y * 0.6);

    // --- bass bloom at the horizon -----------------------------------------
    let d = distance(uv, vec2<f32>(0.5, 0.28));
    col = col + vec3<f32>(0.45, 0.22, 0.85) * bass * 0.5 / (d * 6.0 + 1.0);

    // --- spectrum equalizer along the bottom -------------------------------
    let bars = 64.0;
    let bx = floor(uv.x * bars);
    let sx = bx / bars;
    let yb = 1.0 - uv.y;                 // 0 at the bottom of the screen
    // idle shimmer so the bars always look alive, even in silence
    let idle = 0.045 + 0.035 * sin(t * 2.0 + bx * 0.5) + 0.025 * sin(t * 5.0 + bx);
    var h = max(spec(sx), idle * 0.6);
    h = pow(h, 0.6) * 0.42;
    let cell = fract(uv.x * bars);
    let gap = smoothstep(0.04, 0.12, cell) * smoothstep(0.04, 0.12, 1.0 - cell);
    let barCol = mix(vec3<f32>(0.15, 0.55, 1.0), vec3<f32>(1.0, 0.25, 0.6), clamp(h * 2.0 + sx * 0.3, 0.0, 1.0));
    let barMask = step(yb, h);
    let tipGlow = exp(-40.0 * max(0.0, yb - h));
    col = col + barCol * gap * (barMask * (0.5 + 0.6 * h) + tipGlow * 0.6);

    // --- vignette + subtle volume lift -------------------------------------
    let vig = smoothstep(1.25, 0.35, length(uv - vec2<f32>(0.5)));
    col = col * vig * (1.0 + 0.15 * vol);

    return vec4<f32>(col, 1.0);
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

float iSpectrum(float x) {
    int idx = int(clamp(x, 0.0, 1.0) * 63.0);
    return rlw.spectrum[idx >> 2][idx & 3];
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
