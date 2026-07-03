// audio_bars.glsl — Shadertoy-style shader using real_live_wall's reactive
// extensions: iSpectrum(x) samples the 64-bin FFT, and iBass/iMid/iTreble/iVolume
// give band energy. Play music (with --audio auto/loopback) to see it move.
//
//   real_live_wall --shader shaders/audio_bars.glsl --watch

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 uv = fragCoord / iResolution.xy;

    // deep background tinted by treble
    vec3 col = mix(vec3(0.02, 0.02, 0.06), vec3(0.10, 0.04, 0.14), uv.y);
    col += iTreble * 0.15 * vec3(0.2, 0.5, 1.0);

    // 64 spectrum bars
    const float BARS = 64.0;
    float bx = floor(uv.x * BARS);
    float sx = bx / BARS;
    float h = pow(iSpectrum(sx), 0.6);
    float yb = 1.0 - uv.y;

    float cell = fract(uv.x * BARS);
    float gap = smoothstep(0.05, 0.15, cell) * smoothstep(0.05, 0.15, 1.0 - cell);
    vec3 barCol = mix(vec3(0.15, 0.6, 1.0), vec3(1.0, 0.2, 0.55), h);
    float mask = step(yb, h * 0.85);
    float tip = exp(-50.0 * max(0.0, yb - h * 0.85));
    col += barCol * gap * (mask * (0.4 + 0.6 * h) + tip * 0.7);

    // bass-driven radial pulse
    float d = distance(uv, vec2(0.5, 0.5));
    col += iBass * 0.4 * vec3(0.6, 0.3, 1.0) / (d * 7.0 + 1.0);

    fragColor = vec4(col, 1.0);
}
