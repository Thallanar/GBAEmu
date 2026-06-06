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
│   └── shiny/       # módulo Shiny Hunter
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
- [ ] EEPROM (protocolo serial via DMA na região 0x0D)
- [ ] Save states (snapshot completo do emulador)

### Fase 5 — Frontend Desktop (3–4 semanas)

- [ ] File picker e biblioteca de ROMs
- [ ] Configuração de teclas e gamepad
- [ ] Save states UI
- [ ] Fast-forward, rewind, screenshots

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
- [ ] Outros jogos: Ruby/Sapphire, FireRed/LeafGreen (RAM + gRngValue + endereços
      do menu do inicial por versão)
- [ ] Método: random encounters (detecção de tela de batalha de selvagem)
- [ ] (Opcional avançado) RNG manipulation

### Fase 7 — Port Android (4–6 semanas)

- [ ] Wrapper JNI sobre o core Rust
- [ ] Overlay de controles touch
- [ ] Suporte a controle Bluetooth
- [ ] Empacotamento APK / distribuição

### Fase 8 — Polimento (contínuo)

- [ ] Shaders (LCD, scanlines, CRT)
- [ ] Cheat codes (GameShark / CodeBreaker)
- [ ] Localização PT-BR
- [ ] Netplay (futuro)

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

- [~] Fase 5 — Frontend desktop: janela egui, abrir ROM, framebuffer escalável,
  input, persistência de save, slider de velocidade. Faltam save states UI,
  gamepad (gilrs), fast-forward/rewind, screenshots, biblioteca de ROMs.

- [~] Fase 4 — saves: SRAM + Flash 64K/128K + persistência `.sav` (ok). EEPROM
  (detecção presente; protocolo não) e save states ainda pendentes.
- [~] Fase 6 — Shiny Hunter: perfis data-driven + leitura/descripto Gen 3 + loop
  de soft-reset + **injeção de seed no RNG** + UI + painel com sprite normal/shiny
  da ROM, **validado na ROM real do Emerald**. Os 3 iniciais de Hoenn
  (Torchic/Treecko/Mudkip) caçam via controle do cursor do menu em **malha
  fechada** (endereços achados com o detector de RAM do desktop). Faltam outros
  jogos e o método de random encounters.
- [ ] Fase 7 — Android

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
- **Faltam**: EEPROM (protocolo serial), save states, gamepad/fast-forward/
  screenshots no frontend, mosaic afim (só BG texto + OBJ implementados), e o
  port Android.
- Suíte: **110 testes** passam, clippy estrito limpo.
