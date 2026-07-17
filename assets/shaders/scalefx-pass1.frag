// ScaleFX — pass 1 (porte de Sp00kyFox, MIT). Força dos cantos a partir da
// métrica do pass 0. Saída = dados → manifesto marca `float = true`.
// Corpo verbatim do original; só a amostragem foi adaptada (Source → uTex).
// Params do preset baked como const: SFX_CLR=0.5, SFX_SAA=1.0.

// corner strength
float sfx_str(float d, vec2 a, vec2 b) {
    const float SFX_CLR = 0.5;
    float diff = a.x - a.y;
    float wght1 = max(SFX_CLR - d, 0.0) / SFX_CLR;
    float wght2 = clamp((1.0 - d) + (min(a.x, b.x) + a.x > min(a.y, b.y) + a.y ? diff : -diff), 0.0, 1.0);
    // SFX_SAA == 1.0 → a condição (SFX_SAA==1. || ...) é sempre verdadeira.
    return (wght1 * wght2) * (a.x * a.y);
}

vec4 effect(vec2 uv) {
    vec2 ts = 1.0 / uInputSize;
    // grid 3×3: A B / D E F / G H I (metric data do pass 0)
    vec4 A = SAMPLE(uTex, uv + vec2(-1.0, -1.0) * ts), B = SAMPLE(uTex, uv + vec2(0.0, -1.0) * ts);
    vec4 D = SAMPLE(uTex, uv + vec2(-1.0, 0.0) * ts), E = SAMPLE(uTex, uv), F = SAMPLE(uTex, uv + vec2(1.0, 0.0) * ts);
    vec4 G = SAMPLE(uTex, uv + vec2(-1.0, 1.0) * ts), H = SAMPLE(uTex, uv + vec2(0.0, 1.0) * ts), I = SAMPLE(uTex, uv + vec2(1.0, 1.0) * ts);

    vec4 res;
    res.x = sfx_str(D.z, vec2(D.w, E.y), vec2(A.w, D.y));
    res.y = sfx_str(F.x, vec2(E.w, E.y), vec2(B.w, F.y));
    res.z = sfx_str(H.z, vec2(E.w, H.y), vec2(H.w, I.y));
    res.w = sfx_str(H.x, vec2(D.w, H.y), vec2(G.w, G.y));
    return res;
}
