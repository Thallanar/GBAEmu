// scanlines — uma linha escura por scanline da fonte (160 linhas do GBA), com
// perfil suave estilo CRT: vale ~55% de brilho entre as linhas e cume 100% no
// centro de cada linha. Dá uma sensação mais "dark" e descansa a vista.
// Single-pass. Veja README.md para o contrato de uniforms/aliases.
vec4 effect(vec2 uv) {
    vec4 c = SAMPLE(uTex, uv);
    // Um ciclo de cosseno por linha da fonte: cume (1.0) no centro da linha,
    // vale (0.0) no meio do caminho entre duas linhas.
    float scan = 0.5 + 0.5 * cos(6.2831853 * uv.y * uInputSize.y);
    float k = mix(0.55, 1.0, scan);
    return vec4(c.rgb * k, c.a);
}
