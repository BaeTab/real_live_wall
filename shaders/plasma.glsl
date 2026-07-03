// plasma.glsl — a classic Shadertoy-compatible image shader.
// Uses only standard Shadertoy uniforms (iTime, iResolution), so it demonstrates
// that stock Shadertoy shaders run unmodified on real_live_wall.
//
//   real_live_wall --shader shaders/plasma.glsl

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    vec2 uv = (2.0 * fragCoord - iResolution.xy) / iResolution.y;
    float t = iTime;

    float v = 0.0;
    v += sin((uv.x + t) * 3.0);
    v += sin((uv.y - t) * 4.0);
    v += sin((uv.x + uv.y + t) * 3.0);
    float cx = uv.x + 0.5 * sin(t * 0.3);
    float cy = uv.y + 0.5 * cos(t * 0.4);
    v += sin(sqrt(cx * cx + cy * cy + 1.0) * 6.0 - t);

    vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.0, 4.0) + v + t * 0.5);
    fragColor = vec4(col, 1.0);
}
