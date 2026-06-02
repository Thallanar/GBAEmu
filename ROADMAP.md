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

### Fase 2 — PPU / Vídeo (3–5 semanas)

- [ ] Modos de vídeo 0–5
- [ ] Backgrounds, sprites, blending, windows
- [ ] Renderização por scanline
- [ ] Output via `wgpu`

### Fase 3 — APU + Timers + DMA + IRQ (3–4 semanas)

- [ ] 4 canais PSG
- [ ] 2 canais DMA de som
- [ ] Timers 0–3
- [ ] Sistema de interrupções
- [ ] 4 canais DMA gerais

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

- [ ] Banco de dados por jogo (Gen 3: Ruby/Sapphire/Emerald/FireRed/LeafGreen)
- [ ] Método: soft reset hunting (lendários, starters, fósseis)
- [ ] Método: random encounters (detecção de tela de batalha + leitura de PID)
- [ ] Detector shiny: `(PID ^ TID ^ SID ^ (PID>>16)) < 8`
- [ ] Automação: input scripting + save state + loop
- [ ] UI: contador de resets, ETA, log, notificação ao encontrar
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

- [~] Fase 2 — PPU:
  - [X] Máquina de estados de scanline (HBlank/VBlank/VCount IRQs)
  - [X] Modos bitmap 3/4/5
  - [X] Modos tile 0/1/2 (backgrounds texto + afim, prioridade)
  - [X] Sprites (OBJ) — normal + afim, 1D/2D, prioridade
  - [ ] Janelas, blending (BLDCNT), mosaic
- [~] Fase 3 — Timers + IRQ (ok); DMA 4 canais (imediato/VBlank/HBlank, ok);
  APU/som ainda pendente; DMA "special" (FIFO de som) pendente

- [X] BIOS HLE (SWI + trampolim de IRQ + direct boot) — sem BIOS oficial
- [X] Joypad (KEYINPUT/KEYCNT + IRQ de keypad + input no desktop)

- [~] Fase 5 — Frontend desktop: janela egui, abrir ROM, framebuffer, input

- [~] Fase 4 — saves: SRAM + Flash 64K/128K funcionando + persistência `.sav`;
  EEPROM e save states ainda pendentes
- [ ] Fase 6 — Shiny Hunter (esqueleto da crate criado; agora desbloqueado pelos
  saves de Flash)
- [ ] Fase 7 — Android

### Validação (jsmolka gba-tests)

- [X] `arm.gba` — **All tests passed** (corrigidos SWP/SWPB, S-bit user-bank,
  rlist vazia e base-na-lista no LDM/STM).
- [X] `thumb.gba` — **All tests passed** (quirks de LDMIA/STMIA fmt15).
- [X] `memory.gba` — **All tests passed** (quirk de STRB em memória de vídeo).

### Pendências conhecidas

- **Formatação**: o repositório não passa por `cargo fmt` (commits anteriores ao
  HLE não foram formatados); arquivos novos/reescritos já estão fmt-clean.
- Falta áudio (APU), janelas/blending/mosaic na PPU, DMA de som, EEPROM, save
  states, e timing de ciclos exato (wait states).
