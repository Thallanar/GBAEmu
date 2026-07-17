# Shaders do AuroraGBA — formato próprio (single-pass)

Cada efeito é **um fragment shader single-pass** num dialeto neutro. O arquivo
`.frag` contém **só o corpo do efeito**: uma função

```glsl
vec4 effect(vec2 uv) { ... }
```

onde `uv` é a coordenada de textura (0..1, com `y=0` no topo). Cada frontend
**prepende um preâmbulo** que resolve as diferenças de dialeto entre OpenGL ES
(Android) e OpenGL desktop (glow) e chama `effect(vTex)` no `main()`. Por isso o
mesmo `.frag` roda nas duas frentes sem alteração.

Esta é a fonte **canônica única**: o desktop embute via `include_str!` e o
Android lê via `AssetManager` (o `assets/` do app aponta para esta pasta).

## Contrato (o que o preâmbulo garante)

Aliases/funções disponíveis dentro de `effect`:

| nome | o que é |
| --- | --- |
| `SAMPLE(tex, uv)` | amostra a textura (`texture2D` no GLES, `texture` no GL desktop) |

Uniforms disponíveis:

| uniform | tipo | significado |
| --- | --- | --- |
| `uTex` | `sampler2D` | framebuffer do GBA (240×160 RGBA8, filtro NEAREST) |
| `uInputSize` | `vec2` | resolução de entrada em pixels — sempre `(240, 160)` |
| `uOutputSize` | `vec2` | tamanho do retângulo de saída em pixels (pós-escala) |
| `uFrameCount` | `int` | contador de frames, para efeitos animados |

## Regras para escrever um efeito

- Não declare `precision`, `#version`, `varying/in/out`, nem `main()` — o
  preâmbulo cuida disso. Escreva apenas a função `effect`.
- Use `SAMPLE(uTex, uv)` para ler a imagem; não chame `texture2D`/`texture`
  direto (muda entre as duas frentes).
- Mantenha precisão `mediump`-amigável (o Android roda em `mediump`).
- Um `.frag` é sempre **single-pass**. Efeitos de **múltiplos passes** existem
  via um manifesto `.mpass` que encadeia vários `.frag` (ver abaixo); presets
  `.glslp` do RetroArch continuam fora deste formato.

## Importar um `.frag` próprio

As duas frentes deixam carregar um efeito de fora dos embutidos, seguindo este
mesmo contrato (só a função `effect`, sem `#version`/`precision`/`main`):

- **Desktop:** janela _🎨 Vídeo_ → **Importar .frag…**. O caminho é lembrado e o
  shader é recompilado no próximo boot (some se o arquivo for movido/apagado).
- **Android:** menu ☰ → _🎨 Shader_ → **Importar .frag…**. O texto do shader é
  copiado para as preferências (sobrevive a restart mesmo sem acesso ao arquivo).

Um shader importado aparece como **Custom** no seletor, ao lado dos embutidos.

## Efeitos multipass (`.mpass`)

Um efeito de múltiplos passes é um **manifesto** de texto `.mpass` que encadeia
vários `.frag` deste mesmo formato. Cada passe renderiza numa textura intermediária
(ping-pong de framebuffer); o **último passe desenha na tela**. Fonte canônica
única, igual aos `.frag` (desktop embute; Android lê dos assets).

Formato do manifesto — `key = value` por linha, `#` é comentário:

```
passes = 2
pass0 = blur-h.frag ; scale = 1 ; filter = linear
pass1 = blur-v.frag ; scale = 1 ; filter = linear
```

- `passes` — quantidade de passes.
- `passN` — `<arquivo.frag> ; scale = <int> ; filter = nearest|linear ; float =
  true ; prev = <int>` (só o `.frag` é obrigatório; o resto tem default).
  - `scale` — fator inteiro da textura de **saída** daquele passe (× tamanho da
    fonte). O passe final ignora `scale` (desenha direto no retângulo da tela).
    Default `1`.
  - `filter` — filtro (min/mag) da textura de saída daquele passe, isto é, como o
    passe seguinte a amostra. Default `nearest`.
  - `float` — se `true`, a textura de saída é meio-float (RGBA16F) em vez de
    RGBA8; para passes que guardam **dados** (não cor) sem perder precisão (ex.:
    métricas do ScaleFX). No Android exige `EXT_color_buffer_half_float` +
    `OES_texture_half_float`; sem eles o efeito é escondido do menu. Default `false`.
  - `prev` — índice de um passe **anterior** cuja saída este passe lê em `uPrevTex`
    (ver ABI abaixo). Default: sem `uPrevTex`.

**ABI de um passe** — superset da de um `.frag` single-pass:

- `uTex` — no **passe 0** é o framebuffer da fonte; no **passe k** é a saída do
  passe `k-1`. `uInputSize` = tamanho dessa entrada.
- `uOrigTex`/`uOrigSize` — a textura **original** (a fonte da cadeia), disponível
  em **todos** os passes. Útil pra passes finais que combinam a classificação com a
  imagem de origem (ex.: reverse-AA do ScaleFX).
- `uPrevTex`/`uPrevSize` — a saída do passe declarado em `prev = N`. Sem `prev`,
  `uPrevTex` fica apontando pra fonte (inofensivo; só quem declara deve amostrá-lo).
- `uOutputSize` = tamanho da saída daquele passe. `SAMPLE`/`uFrameCount` inalterados.

## Efeitos atuais

- `none.frag` — passthrough (sem efeito).
- `scanlines.frag` — scanlines suaves (perfil cosseno por linha da fonte), com
  vale ~55% de brilho; sensação CRT/"dark".
- `lcd-grid.frag` — grade de LCD (escurece nas bordas de cada pixel da fonte, em
  x e y); imita a matriz de pontos do LCD do GBA.
- `lcd3x.frag` — porte single-pass do `lcd3x` do libretro: faixas de subpixel RGB
  por coluna (senoide defasada por canal) + leve modulação por linha.
- `crt.frag` — scanlines + leve aperture grille nas colunas + vinheta nas bordas.

Multipass:

- `blur.mpass` — blur gaussiano separável de 2 passes (`blur-h.frag` +
  `blur-v.frag`); efeito de prova do motor multipass.
- `scalefx.mpass` — ScaleFX (Sp00kyFox, MIT), upscaler de pixel-art de 5 passes
  (`scalefx-pass0..4.frag`). Preserva texto/curvas melhor que o xBRZ. Usa `float`
  (pass0/pass1), `prev` (pass2 lê a métrica do pass0) e `uOrigTex` (pass4 mistura
  com um reverse-AA da imagem original). No Android depende de meio-float.
