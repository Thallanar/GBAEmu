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
- Single-pass apenas. Multipass / presets `.glslp` do RetroArch são um stretch
  goal futuro e não fazem parte deste formato.

## Efeitos atuais

- `none.frag` — passthrough (sem efeito).
- `scanlines.frag` — scanlines suaves (perfil cosseno por linha da fonte), com
  vale ~55% de brilho; sensação CRT/"dark".
