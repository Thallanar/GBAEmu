# AuroraGBA — Roadmap

> Emulador de Game Boy Advance multiplataforma (Windows, Linux, Android) com modo diferencial **Shiny Hunter** para automação de caça a Pokémon shiny.

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
      Rayquaza + Regis + 3 iniciais.
- [ ] FireRed/LeafGreen (BPRE/BPGE): RAM + gRngValue + endereços do menu do
      inicial por versão (mapear com `rng_scan`/símbolos do decomp pokefirered)
- [ ] Método: random encounters (detecção de tela de batalha de selvagem)
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
- [ ] Empacotamento APK assinado / distribuição

### Fase 8 — ✨ Polimento & features extras (contínuo)

Recursos de qualidade de vida e apresentação, sem ordem fixa — entram conforme
fazem sentido. Nenhum entregue ainda.

**Vídeo / apresentação:**

- [ ] Shaders (LCD grid, scanlines, CRT, integer scaling, aspect lock)
- [ ] Filtros de upscale (xBRZ / HQ2x) como alternativa aos shaders
- [ ] Bordas / molduras (skins de GBA ao redor da tela)

**Cheats:**

- [ ] Cheat codes GameShark / CodeBreaker / Action Replay (parser + aplicação
      por frame na RAM)
- [ ] Gerenciador de cheats na UI (lista por jogo, liga/desliga, persistido)
- [ ] Importar arquivos `.cht` / formato compatível

**Acessibilidade & UX:**

- [ ] Localização PT-BR (e estrutura i18n para outros idiomas)
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
- [~] **2. Pacing uniforme no desktop**: tentado e **estacionado** — baseline
      mantido. O dynamic rate control não resolveu o judder de forma limpa; as
      lições ficaram registradas. No Android, o pacing de apresentação (~59,73
      fps anunciados ao compositor) já foi entregue (#28).
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
  da ROM, **validado na ROM real do Emerald**. Os 3 iniciais de Hoenn
  (Torchic/Treecko/Mudkip) caçam via controle do cursor do menu em **malha
  fechada** (endereços achados com o detector de RAM do desktop). Ruby/Sapphire
  também fechados. Faltam FireRed/LeafGreen e o método de random encounters.
- [~] Fase 7 — Android: ponte JNI + app Kotlin (render GL ES + controles touch +
  **áudio via AudioTrack**), build via cargo-ndk + Gradle, **validado no emulador
  rodando Pokémon Emerald**. Saves (`.sav` + estados, espelho SAF), biblioteca de
  ROMs persistente, gamepad físico remapeável, fast-forward, screenshot, reset e
  Shiny Hunter em retrato — tudo entregue. Falta só APK assinado / distribuição.
- [X] Fase 9 — Performance: batch da PPU + fast-path do bus (×1,43) + batch dos
  timers ciclo-exato. Pacing uniforme no desktop tentado e **estacionado**
  (baseline mantido); fast-forward por menu no Android.
- [~] Fase 10 — Aceleração do core: cache de decode + handlers monomorfizados
  entregues; JIT só se o profiling provar necessidade.
- [~] Fase Link — Cabo de link: SIO desconectado → lockstep TCP → **trade Gen 3**
  validada, crate `link` portátil, painel + descoberta LAN (UDP + mDNS) no desktop
  e bindings + painel no Android. Falta link Android↔PC sobre Wi-Fi (latência) e
  o relay de internet (L4).

### Validação (jsmolka gba-tests)

- [X] `arm.gba` — **All tests passed** (corrigidos SWP/SWPB, S-bit user-bank,
  rlist vazia e base-na-lista no LDM/STM).
- [X] `thumb.gba` — **All tests passed** (quirks de LDMIA/STMIA fmt15).
- [X] `memory.gba` — **All tests passed** (quirk de STRB em memória de vídeo).

### Pendências conhecidas

- **Formatação**: o repositório não passa por `cargo fmt` (commits anteriores ao
  HLE não foram formatados); arquivos novos/reescritos já estão fmt-clean.
- **Timing de ciclos**: hoje cada instrução conta como 1 ciclo (placeholder);
  falta wait states por região de memória (afeta precisão fina e pitch de áudio).
- **Faltam**: Fase 6 (FireRed/LeafGreen + método de random encounters no Shiny
  Hunter), mosaic afim (só BG texto + OBJ implementados), APK Android assinado,
  link Android↔PC sobre Wi-Fi (latência) e o relay de internet (Fase Link L4).
