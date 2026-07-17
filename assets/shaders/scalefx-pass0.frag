// ScaleFX — pass 0 (porte do shader de Sp00kyFox, MIT; ver README/multipass).
// Prepara dados de métrica (distâncias perceptuais entre vizinhos) para o pass 1.
// Saída = dados, não cor → o manifesto marca este passe como `float = true`.
// Corpo do algoritmo preservado verbatim do original; só a amostragem foi
// adaptada da ABI RetroArch (textureOffset/Source) para a nossa (SAMPLE/uTex).

// Reference: http://www.compuphase.com/cmetric.htm
float sfx_dist(vec3 A, vec3 B) {
    float r = 0.5 * (A.r + B.r);
    vec3 d = A - B;
    vec3 c = vec3(2.0 + r, 4.0, 3.0 - r);
    return sqrt(dot(c * d, d)) / 3.0;
}

vec4 effect(vec2 uv) {
    vec2 ts = 1.0 / uInputSize; // 1 texel
    // grid:  A B C / . E F   (E = centro)
    vec3 A = SAMPLE(uTex, uv + vec2(-1.0, -1.0) * ts).rgb;
    vec3 B = SAMPLE(uTex, uv + vec2(0.0, -1.0) * ts).rgb;
    vec3 C = SAMPLE(uTex, uv + vec2(1.0, -1.0) * ts).rgb;
    vec3 E = SAMPLE(uTex, uv).rgb;
    vec3 F = SAMPLE(uTex, uv + vec2(1.0, 0.0) * ts).rgb;
    return vec4(sfx_dist(E, A), sfx_dist(E, B), sfx_dist(E, C), sfx_dist(E, F));
}
