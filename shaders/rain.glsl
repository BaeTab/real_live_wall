// rain.glsl — moody rainfall; a bass hit flashes like distant lightning.

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9, 78.2))) * 43758.5);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec3 col = mix(vec3(0.03, 0.04, 0.07), vec3(0.09, 0.10, 0.16), uv.y);

    // three depth layers of rain streaks
    float rain = 0.0;
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        float cols = 80.0 + fk * 50.0;
        float x = floor(uv.x * cols);
        float speed = 1.5 + hash(vec2(x, fk)) * 1.8;
        float y = fract(uv.y + iTime * speed + hash(vec2(x, fk + 7.0)));
        float drop = smoothstep(0.0, 0.02, y) * smoothstep(0.28, 0.0, y);
        rain += drop * (0.35 + 0.65 * hash(vec2(x, fk + 3.0))) * (1.0 - fk * 0.25);
    }
    col += vec3(0.6, 0.7, 0.95) * rain * 0.25;

    // ground mist + lightning on bass
    col += vec3(0.1, 0.12, 0.18) * smoothstep(0.35, 0.0, uv.y);
    col += vec3(0.55, 0.6, 0.75) * iBass * 0.35;
    fragColor = vec4(col, 1.0);
}
