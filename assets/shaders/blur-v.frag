// blur-v — passe vertical do blur gaussiano separável (5 taps, kernel binomial
// 1-4-6-4-1). É o passe 1 (final) de `blur.mpass`: lê a saída do passe horizontal
// (uTex) e escreve na tela. Junto com o blur-h forma um gaussiano 2D barato.
// Veja README.md (seção multipass).
vec4 effect(vec2 uv) {
    vec2 t = vec2(0.0, 1.0 / uInputSize.y);
    vec4 c = SAMPLE(uTex, uv - 2.0 * t) * 1.0;
    c += SAMPLE(uTex, uv - 1.0 * t) * 4.0;
    c += SAMPLE(uTex, uv) * 6.0;
    c += SAMPLE(uTex, uv + 1.0 * t) * 4.0;
    c += SAMPLE(uTex, uv + 2.0 * t) * 1.0;
    return c / 16.0;
}
