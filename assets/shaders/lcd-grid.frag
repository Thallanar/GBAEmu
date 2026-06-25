// lcd-grid — grade de LCD suave: escurece nas bordas de cada pixel da fonte,
// em x e y (perfil cosseno), imitando a matriz de pontos do LCD do GBA. Cume
// 100% no centro do pixel, vale ~65% nos cantos. Independente da escala.
// Single-pass. Veja README.md para o contrato de uniforms/aliases.
vec4 effect(vec2 uv) {
    vec4 c = SAMPLE(uTex, uv);
    float gx = 0.5 + 0.5 * cos(6.2831853 * uv.x * uInputSize.x);
    float gy = 0.5 + 0.5 * cos(6.2831853 * uv.y * uInputSize.y);
    float k = mix(0.65, 1.0, gx * gy);
    return vec4(c.rgb * k, c.a);
}
