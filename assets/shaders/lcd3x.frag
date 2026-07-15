// lcd3x — grade de LCD estilo handheld (porte do lcd3x do libretro): faixas de
// subpixel RGB por coluna da fonte (perfil senoidal defasado por canal) + leve
// modulação por linha, imitando a matriz de pontos do LCD. Os `brighten_*`
// controlam o contraste da grade (maior = mais suave). Independente da escala.
// Single-pass. Veja README.md para o contrato de uniforms/aliases.
vec4 effect(vec2 uv) {
    // Defasagem por canal (R/G/B) que separa as três faixas de subpixel na coluna.
    const vec3 offsets = 3.141592654 * vec3(0.5, 0.5 - 2.0 / 3.0, 0.5 - 4.0 / 3.0);
    const float brighten_scanlines = 16.0; // maior = linhas mais suaves
    const float brighten_lcd = 4.0;        // maior = colunas mais suaves
    vec4 c = SAMPLE(uTex, uv);
    // Um ciclo senoidal por pixel da fonte, em x e y.
    vec2 angle = 6.2831853 * uv * uInputSize;
    float yf = (brighten_scanlines + sin(angle.y)) / (brighten_scanlines + 1.0);
    vec3 xf = (brighten_lcd + sin(angle.x + offsets)) / (brighten_lcd + 1.0);
    return vec4(c.rgb * yf * xf, c.a);
}
