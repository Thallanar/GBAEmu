// blur-h — passe horizontal de um blur gaussiano separável (5 taps, kernel
// binomial 1-4-6-4-1). É o passe 0 do efeito multipass `blur.mpass`: lê a fonte
// e entrega pro passe vertical. Offsets inteiros de texel (via uInputSize), então
// funciona mesmo com a fonte em NEAREST. Veja README.md (seção multipass).
vec4 effect(vec2 uv) {
    vec2 t = vec2(1.0 / uInputSize.x, 0.0);
    vec4 c = SAMPLE(uTex, uv - 2.0 * t) * 1.0;
    c += SAMPLE(uTex, uv - 1.0 * t) * 4.0;
    c += SAMPLE(uTex, uv) * 6.0;
    c += SAMPLE(uTex, uv + 1.0 * t) * 4.0;
    c += SAMPLE(uTex, uv + 2.0 * t) * 1.0;
    return c / 16.0;
}
