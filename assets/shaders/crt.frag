// crt — visual de tubo: scanlines horizontais suaves + leve aperture grille nas
// colunas + vinheta escurecendo as bordas. Single-pass (sem curvatura de tela,
// que exigiria distorcer a amostragem). Veja README.md para o contrato.
vec4 effect(vec2 uv) {
    vec4 c = SAMPLE(uTex, uv);
    // Scanlines (perfil cosseno por linha da fonte).
    float scan = 0.5 + 0.5 * cos(6.2831853 * uv.y * uInputSize.y);
    float scanK = mix(0.5, 1.0, scan);
    // Aperture grille leve nas colunas (uma modulação por pixel da fonte).
    float ap = 0.85 + 0.15 * (0.5 + 0.5 * cos(6.2831853 * uv.x * uInputSize.x));
    // Vinheta: escurece em direção aos cantos.
    vec2 d = uv - 0.5;
    float vig = 1.0 - dot(d, d) * 0.7;
    return vec4(c.rgb * scanK * ap * vig, c.a);
}
