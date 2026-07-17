// ScaleFX — pass 4 híbrido (porte de Sp00kyFox, MIT). Passe FINAL: escala 3× e
// combina a classificação de subpixel (Source → uTex) com um reverse-AA sobre a
// imagem original (Original → uOrigTex). Corpo preservado verbatim, exceto:
//  - amostragem adaptada da ABI RetroArch para a nossa (SAMPLE/uTex/uOrigTex);
//  - `res2x` reescrito com 4 vec3 no lugar de `mat4x3` (tipo inexistente em
//    GLSL ES 1.00) — matemática idêntica, portável nas duas frentes.
// Param do preset baked: SFX_RAA=2.0.

// extract corners
vec4 loadCrn(vec4 x) { return floor(mod(x * 80. + 0.5, 9.)); }
// extract mids
vec4 loadMid(vec4 x) { return floor(mod(x * 8.888888 + 0.055555, 9.)); }

vec3 res2x(vec3 pre2, vec3 pre1, vec3 px, vec3 pos1, vec3 pos2) {
    const float SFX_RAA = 2.0;
    vec3 t, m;
    // Colunas de `df = pos - pre`, com pre=(pre2,pre1,px,pos1), pos=(pre1,px,pos1,pos2).
    vec3 df0 = pre1 - pre2;
    vec3 df1 = px - pre1;
    vec3 df2 = pos1 - px;
    vec3 df3 = pos2 - pos1;

    m = mix(px, 1. - px, step(px, vec3(0.5)));
    m = SFX_RAA * min(m, min(abs(df1), abs(df2)));
    t = (7. * (df1 + df2) - 3. * (df0 + df3)) / 16.;
    t = clamp(t, -m, m);
    return t;
}

vec4 effect(vec2 uv) {
    vec2 ss = uInputSize;        // SourceSize.xy (tamanho da fonte deste passe)
    vec2 tso = 1.0 / uOrigSize;  // texel da imagem original

    // read data
    vec4 E = SAMPLE(uTex, uv);

    // determine subpixel
    vec2 fc = fract(uv * ss);
    vec2 fp = floor(3.0 * fc);

    // check adjacent pixels to prevent artifacts
    vec4 hn = SAMPLE(uTex, uv + vec2(fp.x - 1., 0.) / ss);
    vec4 vn = SAMPLE(uTex, uv + vec2(0., fp.y - 1.) / ss);

    // extract data
    vec4 crn = loadCrn(E), hc = loadCrn(hn), vc = loadCrn(vn);
    vec4 mid = loadMid(E), hm = loadMid(hn), vm = loadMid(vn);

    vec3 res = fp.y == 0. ? (fp.x == 0. ? vec3(crn.x, hc.y, vc.w) : fp.x == 1. ? vec3(mid.x, 0., vm.z) : vec3(crn.y, hc.x, vc.z)) : (fp.y == 1. ? (fp.x == 0. ? vec3(mid.w, hm.y, 0.) : fp.x == 1. ? vec3(0.) : vec3(mid.y, hm.w, 0.)) : (fp.x == 0. ? vec3(crn.w, hc.z, vc.x) : fp.x == 1. ? vec3(mid.z, 0., vm.x) : vec3(crn.z, hc.w, vc.y)));

#define TEX(X, Y) SAMPLE(uOrigTex, uv + vec2(X, Y) * tso).rgb

    // reverseAA
    vec3 E0 = TEX(0., 0.);
    vec3 B0 = TEX(0., -1.), B1 = TEX(0., -2.), H0 = TEX(0., 1.), H1 = TEX(0., 2.);
    vec3 D0 = TEX(-1., 0.), D1 = TEX(-2., 0.), F0 = TEX(1., 0.), F1 = TEX(2., 0.);

    // output coordinate - 0 = E0, 1 = D0, 2 = D1, 3 = F0, 4 = F1, 5 = B0, 6 = B1, 7 = H0, 8 = H1
    vec3 sfx = res.x == 1. ? D0 : res.x == 2. ? D1 : res.x == 3. ? F0 : res.x == 4. ? F1 : res.x == 5. ? B0 : res.x == 6. ? B1 : res.x == 7. ? H0 : H1;

    // rAA weight
    vec2 w = 2. * fc - 1.;
    w.x = res.y == 0. ? w.x : 0.;
    w.y = res.z == 0. ? w.y : 0.;

    // rAA filter
    vec3 t1 = res2x(D1, D0, E0, F0, F1);
    vec3 t2 = res2x(B1, B0, E0, H0, H1);

    vec3 a = min(min(min(min(B0, D0), E0), F0), H0);
    vec3 b = max(max(max(max(B0, D0), E0), F0), H0);
    vec3 raa = clamp(E0 + w.x * t1 + w.y * t2, a, b);

    // hybrid output
    return vec4((res.x != 0.) ? sfx : raa, 1.0);
}
