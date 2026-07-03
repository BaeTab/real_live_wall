// forest_fireflies.glsl — misty forest night with drifting, twinkling fireflies.
// Firefly glow swells with the overall volume (iVolume).

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(27.6, 57.4))) * 43758.5);
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;

    // deep forest gradient + faint mist bands
    vec3 col = mix(vec3(0.02, 0.06, 0.05), vec3(0.05, 0.13, 0.10), uv.y);
    col += vec3(0.04, 0.09, 0.07) * smoothstep(0.3, 0.75, uv.y);

    // fireflies
    float glowSum = 0.0;
    for (int k = 0; k < 42; k++) {
        float fk = float(k);
        vec2 p = vec2(hash(vec2(fk, 1.0)), hash(vec2(fk, 2.0)));
        p.x = fract(p.x + iTime * 0.02 * (0.5 + hash(vec2(fk, 3.0))));
        p.y = fract(p.y + iTime * 0.015 * (0.3 + hash(vec2(fk, 4.0))));
        vec2 dv = (uv - p) * vec2(aspect, 1.0);
        float d = length(dv);
        float glow = 0.0016 / (d * d + 0.0004);
        float twinkle = 0.5 + 0.5 * sin(iTime * 3.0 + fk * 1.7);
        glowSum += glow * twinkle;
    }
    col += vec3(0.85, 1.0, 0.45) * glowSum * (0.35 + 0.7 * iVolume);
    fragColor = vec4(col, 1.0);
}
