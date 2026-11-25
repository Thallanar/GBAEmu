# AuroraGBA — Roadmap

> Emulador de Game Boy Advance multiplataforma (Windows, Linux, Android) com modo diferencial **Shiny Hunter** para automação de caça a Pokémon shiny.

_Última atualização: 2026-11-23_

> O estado atual do projeto vive na seção **8. Status atual**; o trabalho não
> segue ordem linear — pegue a fase/etapa que fizer sentido. Mantenha este
> arquivo (e a data acima) atualizado conforme as fases avançam.

---

## 1. Identidade do projeto

- **Nome:** AuroraGBA
- **Linguagem principal:** Rust
- **Plataformas-alvo:** Windows, Linux, Android
- **Licença sugerida:** GPLv3 (padrão em emuladores)
- **Diferencial:** Modo Shiny Hunter — automação de soft reset / encontros aleatórios com detecção via leitura de RAM.

---

## 2. Stack técnica

| Camada           | Tecnologia                                           |
| ---------------- | ---------------------------------------------------- |
| Core do emulador | Rust (puro, sem deps pesadas)                        |
| Renderização   | `wgpu` ou `pixels`                               |
| Áudio           | `cpal`                                             |
| UI desktop       | `egui` / `eframe`                                |
| UI Android       | Kotlin + Jetpack Compose (via JNI sobre o core Rust) |
| Gamepad desktop  | `gilrs`                                            |
| Build            | `cargo workspace`                                  |
| CI               | GitHub Actions (Linux / Windows / Android)           |

### Estrutura de workspace proposta

```
auroragba/
├── crates/
│   ├── core/        # CPU, PPU, APU, memória — sem I/O
│   ├── desktop/     # frontend egui
│   ├── android/     # bindings JNI
│   ├── shiny/       # módulo Shiny Hunter
│   ├── scale/       # filtros de upscale de pixel-art (HQx)
│   └── link/        # protocolo de link cable portátil (transporte + sync)
├── assets/
└── tests/
```

---

## 3. Arquitetura geral

```
┌─────────────────────────────────────────┐
│           Frontend (UI/Input)           │
│  Desktop (egui)  │  Android (Compose)   │
├─────────────────────────────────────────┤
│         Bindings / FFI Layer            │
├─────────────────────────────────────────┤
│          Core Emulator (Rust)           │
│  ┌─────────┬─────────┬──────────────┐   │
│  │ CPU     │ PPU     │ APU          │   │
│  │ ARM7TDMI│ (video) │ (audio)      │   │
│  ├─────────┼─────────┼──────────────┤   │
│  │ Memory  │ Timers  │ DMA / IRQ    │   │
│  ├─────────┴─────────┴──────────────┤   │
│  │ Cartridge / SRAM / Flash         │   │
│  └──────────────────────────────────┘   │
├─────────────────────────────────────────┤
│      Módulo Shiny Hunter (plugin)       │
│  - Save state automation                │
│  - Frame/RAM inspection per game        │
│  - Reset loop + detector                │
└─────────────────────────────────────────┘
```

---

## 4. Roadmap por fases

### Fase 0 — Preparação (1–2 semanas)

- [X] Criar repositório e licença
- [X] Setup do `cargo workspace` (crates: `core`, `desktop`, `android`, `shiny`)
- [X] CI básico no GitHub Actions
- [X] Estudo: manual do ARM7TDMI + [GBATEK](https://problemkaputt.de/gbatek.htm)

### Fase 1 — CPU ARM7TDMI (4–6 semanas)

- [X] Modo ARM (32-bit)
- [X] Modo THUMB (16-bit)
- [X] Memory bus básico (BIOS, WRAM, IWRAM, ROM)
- [X] Testes: `armwrestler`, `FuzzARM`, `gba-tests`

### Fase 2 — PPU / Vídeo (3–5 semanas) ✅

- [X] Modos de vídeo 0–5
- [X] Backgrounds (texto + afim), sprites (normal/afim, 1D/2D)
- [X] Janelas (WIN0/1/OBJ), blending (BLDCNT/BLDALPHA/BLDY), mosaic, OBJ
      semitransparente — composição com resolução top-1/top-2 por pixel
- [X] Renderização por scanline
- [X] Output via textura `egui` (escalada 240×160) — `wgpu` direto fica p/ depois

### Fase 3 — APU + Timers + DMA + IRQ (3–4 semanas) ✅

- [X] 4 canais PSG (quadrada×2 / wave / ruído, envelope/length/sweep)
- [X] 2 canais Direct Sound (FIFO + Timer 0/1 + DMA special)
- [X] Timers 0–3 (cascade + prescalers)
- [X] Sistema de interrupções
- [X] 4 canais DMA gerais (imediato/VBlank/HBlank/special)

### Fase 4 — Cartridge & saves (1–2 semanas)

- [X] Detecção de tipo: SRAM, Flash 64K/128K, EEPROM
- [X] SRAM (32 KB) — leitura/escrita direta
- [X] Flash 64K/128K — máquina de comandos (chip ID, apagar chip/setor, gravar
      byte, troca de banco)
- [X] Persistência `.sav` (carrega no boot, grava throttled + ao fechar)
- [X] EEPROM (protocolo serial via DMA na região 0x0D; 512 B / 8 KB, detecção
      automática do tamanho pelo comprimento do comando)
- [X] Save states (snapshot completo do emulador via serde+bincode, atrás da
      feature `save-states`; ROM/BIOS/framebuffer ficam fora e são restaurados)

### Fase 5 — Frontend Desktop (3–4 semanas)

- [X] Biblioteca de ROMs (grid com capas: box art do libretro + fallback de
      screenshot; varredura de pasta, cache em disco, worker em background)
- [X] Configuração de teclas e gamepad (gilrs; remapeável na UI, persistido)
- [X] Save states UI (8 slots em disco, F5/F9, menu "Estado")
- [X] Fast-forward (Espaço), rewind (R, anel em RAM), screenshots (F12, PNG)
- [X] Ícone do app (o mesmo do launcher Android, gerado por
      `scripts/gen_desktop_icon.py`): ícone da janela via `with_icon`, ícone do
      `.exe` no Windows via `build.rs` (`winresource`) e `.desktop` no Linux
      (`scripts/install_linux_desktop.sh` — no Wayland a dock só usa o `.desktop`)

### Fase 6 — 🌟 Shiny Hunter Mode (3–5 semanas)

- [X] Banco de dados por jogo (data-driven, casado pelo game code do header;
      Emerald semeado, fácil estender)
- [X] Leitura/descriptografia de Pokémon Gen 3 (PID/OTID/espécie + validação por
      checksum)
- [X] Detector shiny: `(PID_hi ^ PID_lo ^ TID ^ SID) < 8`
- [X] Loop de soft reset hunting (power-cycle preservando Flash + amassar
      A/Start até o encontro) — `Hunter::tick` não-bloqueante
- [X] UI: seletor de alvo, contador, último PID + valor, banner ao encontrar
- [X] **Injeção de entropia no RNG do jogo** (`gRngValue` do Emerald em
      0x03005D80): sem isso o emulador é determinístico e todo reset dá o MESMO
      PID. Validado: Torchic 10/10 PIDs distintos. Cada perfil novo precisa do
      endereço de `gRngValue` daquele jogo.
- [X] Endereços de RAM do Emerald confirmados contra ROM real (player/enemy
      party, espécie Torchic=280)
- [X] Método: starter — os 3 iniciais de Hoenn (Torchic/Treecko/Mudkip). A-mash
      chega na bag e a seleção é forçada em **malha fechada**: escreve o byte do
      cursor `gTasks[0].data[0]` (0=esq/1=centro/2=dir) pro alvo, só quando a task
      de input do menu está ativa (`func == 0x0813425D`). Robusto a timing, sem
      mover o personagem. Reveal na batalha. **Validado na ROM real.**
- [X] Detector de RAM (debug) no desktop — acha endereços por versão (estilo
      Cheat Engine: o byte que passou por 0/1/2 e nunca excedeu 2). Oculto atrás
      da flag `SHOW_RAM_FINDER`; reutilizável pra mapear jogos novos.
- [X] Ruby/Sapphire (AXVE/AXPE): perfis completos — endereços dos símbolos do
      decomp pokeruby (iguais nas revs 0/1/2; só o func do menu do inicial muda,
      coberto por `input_funcs` em lista). `gRngValue=0x03004818` **confirmado
      empiricamente** nas duas ROMs com a ferramenta nova `rng_scan` (scan da
      IWRAM pela assinatura do LCG, tolerante a re-seed); espécies confirmadas
      pelo oráculo de sprite (`rng_scan --sprites`). Alvos: mascote da versão +
      Rayquaza + Regis + 3 iniciais + **selvagem** (#76: `gMain.inBattle` =
      0x03001BAD e cursor do menu = 0x02024E60, achados empiricamente no Ruby e
      conferidos no Sapphire rev 2; 20/20 fugas no save real do Ruby).
- [X] Método: lendário que **não espera interação** (`HuntMethod::SoftResetApproach`)
      — Groudon/Kyogre das cavernas do evento de clima anormal (Terra Cave /
      Marine Cave), que vêm pra cima do jogador por proximidade em vez de esperar
      um A. Depois do soft-reset o Hunter segura a direção do lendário o bastante
      pra **andar** 1 tile (16 frames) e solta por 24, em ciclo, com o A-mash
      rodando nas folgas. Direções confirmadas em jogo: Groudon à direita, Kyogre
      à esquerda. **Validado na ROM real.**
- [~] FireRed/LeafGreen (BPRE/BPGE): perfis completos — endereços dos símbolos
      do decomp pokefirered (**iguais nas 4 builds** FR/LG × rev 0/1).
      `gRngValue=0x03005000` **confirmado empiricamente** com `rng_scan` nas duas
      ROMs (rev 1); espécies confirmadas pelo oráculo de sprite. `gMain.inBattle`
      (0x03003529, bit 0x02) pelo layout do `struct Main` (igual ao Emerald).
      Iniciais de Kanto = Balls na mesa ⇒ `StaticGift` (sem menu de cursor).
      Alvos: 3 iniciais, Mewtwo, pássaros, Snorlax, Lapras, Eevee,
      Hitmonlee/Hitmonchan, 3 fósseis, selvagem. **Bugfix 0.70.2**: o FRLG
      re-semeia o RNG com o Timer1 ao sair do título (frame ~338, depois da
      injeção do frame 200), e todo reset dava o mesmo PID. Agora o Hunter detecta
      o re-seed (valor do `gRngValue` fora do alcance do LCG) e reinjeta.
      Validado sem interface no save real do FireRed (inicial Bulbasaur: PIDs
      distintos a cada reset). Falta uma caça completa em jogo (lendários/presentes).
- [X] Método: **prêmios do cassino de Celadon** (`HuntMethod::GameCornerPrize`,
      #91) — o prêmio nasce no "give Pokémon" do script, então funciona como
      presente (soft-reset re-rola o PID, varredura do time, aviso de time cheio).
      FireRed rev 1, achado com o save real no balcão: lista Abra 180 · Clefairy
      500 · Dratini 2.800 · Scyther 5.500 · Porygon 9.999 (lida da tela); cursor
      da lista em `0x0203ADE6`, forçado só quando o máximo (`0x0203ADE8`) é 5 (o
      SIM/NÃO usa o mesmo struct); moedas = `u16` em `[0x03005008]+0x294` XOR
      chave em `[0x0300500C]+0xAF8` — sem moedas pro prêmio a caça para e avisa
      (`NotEnoughCoins`). **Armadilha**: com o A martelado, um toque no meio do
      "Which prize…" termina o texto e o menu nasce já escolhendo o 1º item; no
      cassino o A só toca com texto esperando (`0x0300222B == 3`), com a lista
      aberta ou após 240 frames. **Validado em jogo** (Dratini/Abra/Clefairy).
      LeafGreen (Pinsir no lugar do Scyther) pendente de um save no balcão.
- [X] **Pula a retrospectiva do FireRed** ("Previously on your quest…", #91):
      flag `0x02031DD8 == 1` enquanto ela roda; um B ≥5 frames depois de ela
      começar a encerra (no 1º frame é ignorado). Tentativa do cassino caiu de
      ~8,3 s pra ~3,9 s. LeafGreen pendente de conferir.
- [X] **"❓ Como usar"** no Shiny Hunter (desktop e Android): modal com o passo a
      passo do modo do alvo e o vídeo de exemplo logo abaixo, em loop. Texto único
      em `crates/shiny/src/help.rs`; vídeos em `assets/hunter-help/` (desktop
      decodifica H.264 baseline com `openh264`, Android usa `VideoView`). Vídeos
      gravados: iniciais RSE, iniciais FRLG e selvagens. **Faltam vídeos** de
      presentes, fósseis, lendários, Groudon/Kyogre e pesca (hoje só texto).
- [X] Método: random encounters selvagem (`HuntMethod::WildSpin`) — sem reset.
      O boneco fica na mesma moita e o Hunter cicla as 4 direções (cada uma
      segurada o bastante pra virar **passo**, não só rotação; mudar a direção
      encarada já dispara o sorteio de encontro). Quando um selvagem carrega no
      `gEnemyParty`, checa o shiny. **Marco 2** (fuga encadeada, jun/2026):
      não-shiny ⇒ foge e volta a girar. Desde o #75 a fuga confirma CORRER em
      malha fechada (`action_cursor` = `gActionSelectionCursor`, achado
      empiricamente em Emerald/FireRed/Ruby) — às cegas ela às vezes abria a bag
      e travava. Disponível nos cinco jogos (R/S desde o #76).
- [X] Método: presente estático (`HuntMethod::StaticGift`) — presentes dados por
      script onde pegar roda `CreateMon(Random())`, então soft-reset re-rola o
      PID. Diferença-chave: o presente cai no **primeiro slot livre** de
      `gPlayerParty` (não no slot 0), então a caça **varre os 6 slots** pela
      espécie. Roteiro é A-mash puro (sem cursor) e **exige um slot de time
      livre** (com time cheio o presente vai pro PC e a varredura não acha).
      Desde o #77 a caça fotografa o time quando o save carrega: time cheio ⇒
      para na hora com aviso (antes o A-mash ia até a tela de apelido e batizava
      o presente de `AAAAAAA`), e o presente só conta num slot que estava vazio
      (um Beldum que já estava no time não é confundido com ele).
      Cobre o **Beldum** na casa do Steven pós-E4 e os **iniciais de Johto**
      (presente do Birch pós-National Dex).
- [~] Método: pesca (rod) — tentativa estacionada. A ideia é o mesmo selvagem
      sem reset, mas disparado por usar a vara em vez de andar; faltou mapear a
      máquina de estados do minigame de fisgada (mordida → A no tempo certo) e a
      leitura confiável do encontro. Parado (o WildSpin Marco 2 já está pronto).
- [ ] (Opcional avançado) RNG manipulation

### Fase 7 — Port Android (4–6 semanas)

- [X] Wrapper JNI sobre o core Rust (create/destroy/loadRom/renderFrame/setButtons)
- [X] App Kotlin (Gradle): SurfaceView com o framebuffer + overlay de controles touch
- [X] Build via cargo-ndk + Gradle; **validado rodando Pokémon Emerald no emulador**
- [X] Áudio (AudioTrack 32768 Hz estéreo; pacing pelo write bloqueante)
- [X] Render via OpenGL ES (escala na GPU, menos calor)
- [X] Saves: `.sav` automático + save states (8 slots) no menu, espelhados numa
      pasta visível `saves/` via SAF tree, com import e sync por timestamp
- [X] Biblioteca: pasta de ROMs persistente via SAF tree
- [X] Suporte a controle Bluetooth / gamepad físico (com remapeamento pelo menu)
- [X] Shiny Hunter no celular: modo retrato com painel (sprite alvo + stats)
- [X] Extras de UX: fast-forward 2x/4x/8x, captura de tela (PNG na galeria),
      botão de reset, pacing de apresentação (~59,73 fps ao compositor)
- [ ] Empacotamento APK assinado / distribuição → virou a **Fase 11**
      (lançamento no Google Play).
- [X] **Redefinição da UI** (pedido em 23/nov/2026; desenhada antes no canvas):
      idiomas en/pt/es (#80), menu + Configurações (#81), modelos de gamepad
      (#82), estados com miniatura (#83), Shiny Hunter (#84) e editor de layout
      dos controles (#85). O Desktop ganhou o mesmo redesenho (#86–#90).

### Fase 8 — ✨ Polimento & features extras (contínuo)

Recursos de qualidade de vida e apresentação, sem ordem fixa — entram conforme
fazem sentido.

**Vídeo / apresentação:**

- [X] **Integer scaling + aspect lock** nas duas frentes — desktop com escala
      inteira persistida; Android com letterbox em múltiplo inteiro de 240×160.
- [X] **Shaders** (formato próprio single-pass, "shader-como-dado"): pipeline +
      embutidos **scanlines / LCD-grid / LCD3x / CRT** entregues ponta a ponta, mais
      **importar `.frag` de arquivo** (efeito "Custom"). Fonte canônica única em
      `assets/shaders/` (contrato de uniforms documentado no README); desktop via
      callback `egui_glow` (glow), Android via GLES20; seletor + custom persistidos
      nas duas frentes. **Motor multipass** (ping-pong de FBO nas duas frentes,
      manifesto `.mpass` encadeando vários `.frag`) entregue, exercitado pelo efeito
      `blur` (2 passes) e agora pelo **ScaleFX** (Sp00kyFox, MIT; upscaler de
      pixel-art de 5 passes — melhor em texto/curvas que o xBRZ). ABI multipass
      expandida com `uOrigTex` (imagem original em todos os passes), `uPrevTex`
      (`prev = N`) e framebuffers meio-float (RGBA16F; Android gateia por
      `EXT_color_buffer_half_float`, escondendo o efeito onde falta). _Próximo_:
      Anime4K. _Stretch goal_ restante: compat de presets `.glslp` do RetroArch.
- [X] Filtros de upscale como alternativa aos shaders: **HQ2x/3x/4x** e
      **xBRZ2x/3x/4x** entregues ponta a ponta (crate `auroragba-scale` sobre
      `hqx` + `xbrz-rs`; desktop amplia na CPU e sobe numa textura de tamanho
      variável; Android amplia no nativo e o Kotlin redimensiona a textura pelo
      fator; seletor persistido nas duas frentes). O contrato de seleção Android
      passou de fator-inteiro para **chave do algoritmo** (JNI `setUpscale(String)`),
      pra HQ e xBRZ do mesmo fator coexistirem. **ScaleFX** (GPU, sobre o motor
      multipass) já entregue; _futuro_: Anime4K.
- [ ] Bordas / molduras (skins de GBA ao redor da tela)

**Cheats:**

- [ ] Cheat codes GameShark / CodeBreaker / Action Replay (parser + aplicação
      por frame na RAM)
- [ ] Gerenciador de cheats na UI (lista por jogo, liga/desliga, persistido)
- [ ] Importar arquivos `.cht` / formato compatível

**Acessibilidade & UX:**

- [X] **Configurações gerais** (desktop **e** Android; pedido em 23/nov/2026):
      Android (#81, editor de layout no #85) e Desktop (#87) — pasta de ROMs,
      áudio, velocidade, vídeo, controles, idioma e sobre. Tudo persistido.
- [X] Localização: **pt / en / es**, automático pelo idioma do sistema —
      Android (#80) e Desktop (#86–#90).
- [ ] Temas de UI além do dark (seguindo o accent holográfico/shiny)
- [ ] Macros / turbo de botão (auto-fire configurável)

**Já encaminhado em outra fase:**

- [~] Netplay → virou a **Fase Link** (link cable real na LAN, ver abaixo)

### Fase 9 — ⚡ Performance & fluidez (planejada em 9/jun/2026)

Medição na máquina de referência (desktop, release, headless): core roda
Emerald a **108 fps (×1,81 do tempo real)** e Ruby a 131 fps, pinando 100% de
um núcleo. Consequências confirmadas: teto do fast-forward ≈ ×1,8 (bate com o
×1,7 medido na GUI), calor no Android, e margem apertada no jogo normal
(~9 ms de emulação num orçamento de 16,7 ms/frame → picos estouram e soluçam).
As engasgadas em scroll/movimento têm uma 2ª causa independente: o **pacing por
áudio entrega vídeo em rajadas** (0–4 frames por update) + desencontro monitor
60 Hz × GBA 59,73 Hz → judder mesmo com CPU sobrando.

Plano em 3 etapas (ordem decidida):

- [X] **1. Otimizar o core guiado por profiling** (flamegraph rodando Emerald):
      batch da PPU + fast-path do bus (#22, ×1,43) e batch dos timers com
      catch-up ciclo-exato (#27, áudio bit-idêntico). A otimização mais pesada
      (cache de decode + handlers monomorfizados) migrou para a **Fase 10**.
- [X] **2. Pacing uniforme no desktop** — reaberta e **implementada em
      23/nov/2026**, validada em uso (#74). Medição com o trace
      `AURORAGBA_PACING_TRACE=1` num monitor de 144 Hz: o pacing antigo, ritmado
      pelo áudio, rodava 2–3 frames por vez ~24×/s (só ~24 imagens distintas por
      segundo). Agora o ritmo vem do monitor (`crates/desktop/src/pacing.rs`):
      - Monitor ~múltiplo de 59,73 Hz (60/120 Hz…): **trava** em 1 frame a cada
        N vsyncs — cadência perfeita, jogo a `Hz/N` (+0,46% em 60 Hz).
      - Outros (144 Hz…): frames distribuídos pelo tempo real — em 144 Hz,
        60 imagens/s espaçadas por 2 ou 3 vsyncs, sem rajadas.
      - Áudio reamostra na velocidade do pacer + controle de taxa dinâmico
        (±0,5%) contra a deriva entre os relógios; fila quase vazia é completada
        com silêncio.
      - Limite: em 144 Hz a alternância 2/3 é inevitável (144/59,73 = 2,41).
        Fica 100% liso em 60/120 Hz ou com VRR.
- [X] **3. Fast-forward personalizável**: no Android via menu (2x/4x/8x, #35);
      no desktop continua no atalho de Espaço com orçamento fixo.

### Fase 10 — ⚙️ Aceleração do core (continuação da Fase 9.1)

Otimizações estruturais do interpretador, medidas em release headless.

- [X] Cache de decode por endereço de ROM (#37) — evita redecodificar o mesmo
      opcode a cada execução.
- [X] Handlers monomorfizados por sub-opcode (#38) — dispatch especializado em
      vez de ramificar dentro do handler genérico.
- [ ] JIT (recompilação dinâmica) — só se o profiling provar necessidade depois
      de esgotar o interpretador. Trabalho de semanas; mantido como último recurso.

### Fase 11 — 🚀 Lançamento 1.0 (Google Play)

> **Critério da 1.0.0:** é a primeira versão publicada em **produção** no
> Google Play, depois do teste fechado. Até lá seguimos na série 0.x (a cada
> PR: feature sobe o número do meio, correção sobe o último). Decidido em
> 25/nov/2026, com as duas interfaces redesenhadas.

**Requisitos técnicos do Play:**

- [ ] Subir `compileSdk`/`targetSdk` (hoje 34) pro alvo exigido pelo Play pra
      apps novos (conferir o número atual no Play Console) e testar as mudanças
      de comportamento do Android novo (tela cheia/edge-to-edge, permissões, SAF).
- [ ] Build de release: **AAB** assinado (chave de upload + Play App Signing),
      `.so` das arquiteturas suportadas, R8/minify avaliado, e o release testado
      no celular (não só o debug).
- [ ] Política de privacidade publicada (URL) e formulário de **segurança de
      dados**: o app usa internet só pro link na rede local e não coleta dados.
- [ ] Classificação etária (questionário IARC) e categoria.

**Jurídico / marcas — decidir antes de publicar:**

- [ ] Ficha da loja **sem marcas de terceiros**: nada de "Pokémon", "Nintendo",
      "Game Boy" no nome, descrição, ícone ou prints (prints com homebrew ou
      sem jogo reconhecível).
- [ ] Decidir como o **Shiny Hunter** aparece na versão da loja (ele é feito sob
      medida pra Pokémon — maior risco de remoção por pedido da Nintendo/TPC).
      Pesquisar como outros emuladores lidam com isso.
- [ ] Revisar textos dentro do app que citam nomes de jogos/marcas e o aviso
      "não tem relação com a Nintendo" (já existe no Sobre).
- [ ] GPL-3.0: link do código-fonte no app (já no Sobre) e na ficha; licenças de
      terceiros completas (ScaleFX MIT, HQx LGPL-2.1, xBRZ GPL-3.0, Inter OFL).

**Compatibilidade e qualidade:**

- [ ] Rodada de compatibilidade com uns **15–20 jogos populares** além de
      Pokémon (registrar o resultado de cada um e corrigir o que travar).
- [ ] Testar em aparelhos variados: Android antigo (minSdk 24), aparelho fraco,
      tablet, com e sem controle físico.
- [ ] **Relatório de travamentos** que respeite a privacidade (sem coletar dados
      pessoais; ex.: log local que o usuário pode compartilhar).
- [ ] Decidir o que fazer com o **link Android↔PC** no Wi-Fi (ainda engasga):
      marcar como experimental ou esconder até a Fase Link resolver a latência.
- [ ] LeafGreen: cassino e pulo da retrospectiva conferidos num save real.
- [ ] Revisar `BUGS.md` (o salto de vsync a 144 Hz no desktop aparece aberto).

**Ficha da loja:**

- [ ] Nome, descrição curta e longa (en/pt/es), ícone 512×512, imagem de
      destaque 1024×500 e prints do celular (e tablet, se for suportar).

**Publicação:**

- [ ] Conta de desenvolvedor no Play Console.
- [ ] **Teste fechado** com testadores (contas pessoais novas precisam de um
      teste fechado com um mínimo de testadores por ~14 dias antes da produção;
      conferir a regra atual no Console) e correção do que aparecer.
- [ ] Produção → **versão 1.0.0**.
- [ ] Desktop: binários Linux/Windows da 1.0.0 nos Releases do GitHub.

### Fase Link — 🔗 Cabo de link / multiplayer

> Visão: trocar/batalhar **a qualquer hora e lugar**, cross-platform
> (celular↔celular, pc↔pc, mesmo aparelho). Hoje o link já funciona na LAN;
> o "qualquer lugar" (relay/internet) é a fase final.

**Núcleo do protocolo (no core + crate `link`):**

- [X] Registradores SIO com semântica de cabo desconectado (#39, etapa a)
- [X] Lockstep TCP entre duas instâncias desktop (#40, etapa b)
- [X] Trade multiplayer Gen 3 via sync event-driven (#44, etapa c) — troca de
      Pokémon validada entre duas instâncias
- [X] **L1** — protocolo extraído para a crate portátil `link` (transporte + sync)

**Frontend desktop (L2):**

- [X] **L2a** — painel de Link no desktop, conexão em thread de fundo
- [X] **L2b** — descoberta de parceiros na LAN via UDP broadcast
- [X] **L2c** — descoberta por mDNS junto com o UDP broadcast

**Android (L3):**

- [X] **L3a** — bindings JNI do link cable no Android
- [X] **L3b** — painel de Link na UI Android

**Pendências:**

- [~] Link Android↔PC sobre Wi-Fi: o lockstep síncrono trava (N×RTT por frame);
      PC↔PC na LAN cabeada roda liso. Frente "thread de emulação separada" foi
      **revertida** (piorou) — não refazer. Caminho a atacar: latência
      (rollback / netcode tolerante a atraso).
- [ ] **L4** — relay/internet ("qualquer lugar"): jogar fora da mesma LAN.
- [ ] Battle link (além de trade) entre jogos Gen 3.

---

## 5. Design / UX

- **Tema:** dark mode por padrão
- **Accent color:** gradiente holográfico/shiny (azul → roxo → rosa) como assinatura visual
- **Inspirações:** Delta Emulator (iOS) e mGBA

### Telas principais

1. Biblioteca de ROMs (grid com capas)
2. Tela de jogo (fullscreen + overlay configurável)
3. Painel do Shiny Hunter (contador grande, sprite alvo, estatísticas em tempo real)
4. Configurações (vídeo, áudio, controles, BIOS)

---

## 6. Considerações legais

- **BIOS do GBA:** não pode ser distribuída. Usuário fornece o `gba_bios.bin`, ou implementamos HLE (High-Level Emulation).
- **ROMs:** sempre fornecidas pelo usuário; nunca distribuir.
- **Google Play:** emuladores são permitidos desde 2024, mas evitar menções diretas a "Pokémon" nos metadados da store.

---

## 7. Referências de estudo

- [GBATEK](https://problemkaputt.de/gbatek.htm) — referência técnica definitiva do GBA
- [mGBA](https://github.com/mgba-emu/mgba) — emulador C de referência
- [rustboyadvance-ng](https://github.com/michelhe/rustboyadvance-ng) — emulador Rust de referência
- [Pokémon Gen 3 RAM Map (Bulbapedia / Datacrystal)](https://datacrystal.tcrf.net/wiki/Pok%C3%A9mon_Emerald/RAM_map)

---

## 8. Status atual

- [X] Roadmap definido
- [X] Nome: **AuroraGBA**
- [X] Linguagem: **Rust**
- [X] Fase 0 — workspace, CI, scripts de ROM de teste
- [X] Fase 1 — CPU ARM7TDMI (ARM + THUMB completos, IRQ, banking)

- [X] Fase 2 — PPU completa: scanline (HBlank/VBlank/VCount IRQs), modos bitmap
  3/4/5, modos tile 0/1/2 (texto + afim), sprites (normal/afim, 1D/2D), e os
  efeitos — janelas (WIN0/1/OBJ), blending (BLDCNT/BLDALPHA/BLDY), mosaic e OBJ
  semitransparente, via composição top-1/top-2 por pixel.
- [X] Fase 3 — Timers + IRQ + DMA 4 canais (imediato/VBlank/HBlank/special) +
  **APU completa**: 4 canais PSG + Direct Sound (FIFO/Timer/DMA special) + saída
  no host via `cpal` (emulação paçada pelo consumo de áudio). RTC/GPIO (S-3511A).

- [X] BIOS HLE (SWI + trampolim de IRQ + direct boot) — sem BIOS oficial
- [X] Joypad (KEYINPUT/KEYCNT + IRQ de keypad + input no desktop)

- [X] Fase 5 — Frontend desktop: janela egui, abrir ROM, framebuffer escalável,
  input, persistência de save, slider de velocidade, **save states UI** (8 slots,
  F5/F9), **fast-forward** (Espaço), **rewind** (R), **screenshots** (F12),
  **config de teclas + gamepad** (gilrs, remapeável e persistido) e **biblioteca
  de ROMs** (grid com capas: box art do libretro + fallback de screenshot). Fase
  completa.

- [X] Fase 4 — saves: SRAM + Flash 64K/128K + **EEPROM** (512 B/8 KB) +
  persistência `.sav` + **save states** (serde+bincode). Fase completa.
- [~] Fase 6 — Shiny Hunter: perfis data-driven + leitura/descripto Gen 3 + loop
  de soft-reset + **injeção de seed no RNG** + UI + painel com sprite normal/shiny
  da ROM, **validado na ROM real do Emerald**. Métodos: **starter** (3 iniciais de
  Hoenn via cursor em malha fechada), **selvagem** (`WildSpin`, sem reset),
  **presente estático** (`StaticGift` — Beldum pós-E4 + iniciais de Johto pós-Dex,
  varrendo os 6 slots do party) e **lendário por proximidade** (`SoftResetApproach`
  — Groudon/Kyogre do clima anormal, com passo de aproximação). Ruby/Sapphire
  também fechados. FireRed/LeafGreen com perfis prontos; FireRed validado em
  jogo no **cassino de Celadon** (#91), com a retrospectiva pulada por B.
  Selvagem com fuga encadeada (Marco 2) robusta desde o #75 e disponível nos
  cinco jogos (R/S no #76). Falta a pesca (estacionada).
- [~] Fase 7 — Android: ponte JNI + app Kotlin (render GL ES + controles touch +
  **áudio via AudioTrack**), build via cargo-ndk + Gradle, **validado no emulador
  rodando Pokémon Emerald**. Saves (`.sav` + estados, espelho SAF), biblioteca de
  ROMs persistente, gamepad físico remapeável, fast-forward, screenshot, reset e
  Shiny Hunter em retrato — tudo entregue, e a **UI redesenhada** (#80–#85).
  A distribuição virou a Fase 11.
- [~] Fase 8 — Polimento/apresentação: integer scaling + aspect lock nas duas
  frentes; **shaders** (pipeline "shader-como-dado" single-pass, embutidos
  scanlines/LCD-grid/LCD3x/CRT + importar `.frag` "Custom", seletor persistido) e
  **filtros de upscale HQ2x/3x/4x + xBRZ2x/3x/4x** (crate `auroragba-scale` sobre
  `hqx` + `xbrz-rs`; seleção Android por chave de algoritmo no JNI) fechados ponta a
  ponta. **Motor multipass** (`.mpass`, ping-pong de FBO) com **blur** e **ScaleFX**
  (5 passes, meio-float) entregues. **Configurações gerais** e idiomas
  pt/en/es nas duas frentes (#80–#90). Próximos: Anime4K, cheats, molduras.
- [X] Fase 9 — Performance: batch da PPU + fast-path do bus (×1,43) + batch dos
  timers ciclo-exato; fast-forward por menu no Android; **pacing uniforme no
  desktop** (ritmo pelo monitor + controle de taxa dinâmico no áudio, #74).
- [~] Fase 10 — Aceleração do core: cache de decode + handlers monomorfizados
  entregues; JIT só se o profiling provar necessidade.
- [~] Fase Link — Cabo de link: SIO desconectado → lockstep TCP → **trade Gen 3**
  validada, crate `link` portátil, painel + descoberta LAN (UDP + mDNS) no desktop
  e bindings + painel no Android. Falta link Android↔PC sobre Wi-Fi (latência) e
  o relay de internet (L4).
- [ ] Fase 11 — Lançamento 1.0 no Google Play: checklist definido (requisitos do
  Play, marcas, compatibilidade, teste fechado); a 1.0.0 sai na produção.

### Validação (jsmolka gba-tests)

- [X] `arm.gba` — **All tests passed** (corrigidos SWP/SWPB, S-bit user-bank,
  rlist vazia e base-na-lista no LDM/STM).
- [X] `thumb.gba` — **All tests passed** (quirks de LDMIA/STMIA fmt15).
- [X] `memory.gba` — **All tests passed** (quirk de STRB em memória de vídeo).

### Pendências conhecidas

- **Bugs**: os bugs pontuais conhecidos vivem em [`BUGS.md`](./BUGS.md).
- **Timing de ciclos**: hoje cada instrução conta como 1 ciclo (placeholder);
  falta wait states por região de memória (afeta precisão fina e pitch de áudio).
- **Faltam**: Fase 6 (LeafGreen em jogo + método de pesca, estacionado),
  mosaic afim (só BG texto + OBJ implementados), a Fase 11 (lançamento no
  Google Play), link Android↔PC sobre Wi-Fi (latência) e o relay de internet
  (Fase Link L4).

---

## 9. Ferramentas de desenvolvimento & debug

> Build oficial (Desktop + Android) e checks de CI ficam no `CLAUDE.md`. Aqui só
> ferramentas de inspeção e atalhos do dia a dia.

- **Smoke test com dump de tela**: roda uma ROM e salva o framebuffer.
  ```bash
  AURORA_DUMP=/tmp/frame.ppm cargo run --release --bin smoke -- <rom.gba> 12000000
  ```
  Depois converter o PPM para PNG para inspecionar (foi assim que mapeamos os
  "Failed test N" do jsmolka). O smoke também imprime nº de cores distintas.
- **Mapear "Failed test N"** dos ROMs jsmolka: o nº do teste mapeia para um
  arquivo `.asm` em `github.com/jsmolka/gba-tests` (ex.: arm tests 450–499 =
  `data_swap.asm`). Útil para achar bugs de CPU.
- **Detector de RAM (Shiny Hunter)**: no desktop, oculto atrás da flag
  `SHOW_RAM_FINDER` — acha endereços por versão (estilo Cheat Engine). Para
  `gRngValue`/espécies de jogos novos, usar `rng_scan` (scan da IWRAM pela
  assinatura do LCG; `--sprites` para confirmar pelo oráculo de sprite).
- **Trace do Cable Link**: variável `AURORAGBA_LINK_TRACE` liga o trace do link.

### Comandos rápidos

```bash
cargo test                                  # todos os testes
cargo clippy --all-targets -- -D warnings   # lint estrito (CI)
cargo run --release -p auroragba-desktop    # abre a GUI
./scripts/fetch-test-roms.sh                # baixa os ROMs de teste do jsmolka
```

### Atalhos do desktop

Padrão (remapeáveis em Configurações → Controles): Z=A, X=B, Enter=Start,
Backspace=Select, setas=direcional, A=L, S=R. **Gamepad** (gilrs) suportado e
persistido. **F5** salva / **F9** carrega o slot de save state, **F12**
screenshot (PNG em `<rom>/screenshots/`), **Espaço** (segurar) fast-forward,
**R** (segurar) rewind. Menu "Estado" escolhe o slot (1–8).
