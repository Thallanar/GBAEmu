// scanlines — escurece as linhas ímpares na resolução de entrada do GBA,
// imitando o gap entre scanlines de um CRT/LCD. Single-pass.
// Veja README.md para o contrato de uniforms/aliases.
vec4 effect(vec2 uv) {
    vec4 c = SAMPLE(uTex, uv);
    // Linha física do GBA sob este pixel (0..159).
    float onLine = step(1.0, mod(uv.y * uInputSize.y, 2.0)); // 0 nas pares, 1 nas ímpares
    float k = mix(1.0, 0.72, onLine);
    return vec4(c.rgb * k, c.a);
}
