// none — passthrough, sem efeito.
// Veja README.md para o contrato de uniforms/aliases que cada frontend injeta.
vec4 effect(vec2 uv) {
    return SAMPLE(uTex, uv);
}
