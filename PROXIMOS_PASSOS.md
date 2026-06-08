# AuroraGBA — Próximos passos

> Arquivo de continuidade: "de onde paramos". O roadmap completo está em
> [`ROADMAP.md`](./ROADMAP.md); aqui fica o estado atual + o que atacar a seguir.

_Última atualização: 2026-06-07_

---

## Onde paramos (estado atual)

O emulador **boota, renderiza com efeitos e toca som**; roda Pokémon Emerald com
vídeo+áudio+input, e o **Shiny Hunter caça de verdade** (RNG com entropia real).

Funcionando e validado:
- **CPU ARM7TDMI** completa (ARM + THUMB), IRQ, banking de registradores.
- **BIOS HLE** (sem BIOS oficial): ~25 funções SWI + trampolim de IRQ + direct boot.
- **DMA** (4 canais: imediato/VBlank/HBlank/special).
- **Timers** (4, com cascade e prescalers) + **IRQ**.
- **Joypad** (KEYINPUT/KEYCNT + IRQ de keypad; teclado mapeado no desktop).
- **PPU completa**: modos bitmap 3/4/5 + tile 0/1/2 (texto e afim) + sprites (OBJ
  normal/afim, 1D/2D) + **efeitos** — janelas (WIN0/1/OBJ), blending
  (BLDCNT/BLDALPHA/BLDY), mosaic, OBJ semitransparente. Composição com resolução
  **top-1/top-2 por pixel** (`apply_effects`/`window_mask` em `ppu.rs`).
- **APU (som) completa**: 4 canais PSG + Direct Sound (FIFO + Timer 0/1 + DMA
  special) + saída no host via `cpal`; a emulação é **paçada pelo consumo de
  áudio** (corrige aceleração). RTC/GPIO (S-3511A).
- **Saves**: SRAM (32 KB) + Flash 64K/128K (máquina de comandos completa) +
  **EEPROM** (512 B/8 KB, serial via DMA na região 0x0D) + persistência em
  `.sav` + **save states** (serde+bincode, feature `save-states`). Fase 4 ✅.
- **Frontend desktop** (egui): abrir ROM, framebuffer escalável, input, save,
  slider de velocidade, menu Shiny Hunter.
- **110** testes unitários, clippy estrito limpo.
- ✅ **jsmolka gba-tests: arm.gba, thumb.gba, memory.gba passam 100%**.

Controles do desktop (padrão, **remapeáveis** em Configurações → Controles): Z=A,
X=B, Enter=Start, Backspace=Select, setas=direcional, A=L, S=R. **Gamepad**
suportado (gilrs): East=A, South=B, Select/Start, D-Pad, R/L gatilhos. O
mapeamento é persistido (storage do eframe). Atalhos: **F5** salva / **F9**
carrega o slot atual de save state, **F12** screenshot (PNG em
`<rom>/screenshots/`), **Espaço** (segurar) fast-forward, **R** (segurar) rewind.
Menu "Estado" escolhe o slot (1–8) e salva/carrega.

### 🌟 Shiny Hunter — funcional e validado (Emerald/Torchic)
Arquitetura **data-driven**: identifica o jogo pelo game code do header e carrega
um `GameProfile` de `crates/shiny/src/games.rs` (1 entrada por jogo).
- [X] `read_mon()` (descripto Gen 3), detector shiny, loop de soft-reset,
      `Gba::reset()` (preserva Flash), UI no desktop.
- [X] **Injeção de entropia no RNG** (`gRngValue` do Emerald em `0x03005D80`):
      sem ela o emulador é determinístico e todo reset dá o MESMO PID. O `Hunter`
      sorteia a seed de um PRNG SplitMix64 semeado por instância e injeta no frame
      ~200. Validado: Torchic **10/10 PIDs distintos**.
- [X] Emerald: player/enemy party + Torchic (espécie 280) confirmados na ROM real.

---

## Próximos passos (em ordem sugerida)

### 1. 🌟 Shiny Hunter — expandir (o diferencial)
- [ ] **Outros jogos**: Ruby/Sapphire (AXVE/AXPE), FireRed/LeafGreen (BPRE/BPGE).
      Cada um precisa dos endereços de `gPlayerParty`/`gEnemyParty` **e do
      `gRngValue`** daquele jogo (achar via scan da IWRAM pela assinatura do LCG
      `seed*0x41C64E6D+0x6073`, ou pelos símbolos do decomp).
- [ ] **Starter Treecko/Mudkip**: precisam de **input direcional** no menu do lab
      (hoje o roteiro só amassa A → pega o do meio, Torchic).
- [ ] **Método random encounters**: detectar a tela de batalha de selvagem (andar
      na grama / repel) e ler o slot inimigo.

### 2. Frontend — qualidade de vida (Fase 5) ✅ COMPLETA
- [X] **Save states** (core: serde+bincode atrás da feature `save-states`).
- [X] **Save states UI** (desktop): 8 slots em disco (`<rom>.ss1`..`.ss8`),
      F5 salva / F9 carrega o slot atual, menu "Estado" pra escolher slot.
- [X] **Fast-forward** (segurar Espaço) / **rewind** (segurar R, anel de snapshots
      em RAM) / **screenshots** (F12 → PNG, encoder próprio em `png.rs`).
- [X] **Gamepad** (gilrs) + **configuração de teclas**: remapeável em
      Configurações → Controles (teclado e gamepad por botão), persistido via
      storage do eframe (feature `persistence`).
- [X] **Biblioteca de ROMs** (Arquivo → Biblioteca): varre uma pasta por `.gba`,
      grid de capas clicáveis. Capa = box art do libretro (jogos conhecidos,
      casados pelo game code) com **fallback de screenshot** (boota a ROM headless
      e captura o frame). Worker em background (rede + emulação fora da thread da
      UI) + cache em disco; pasta persistida.

### 3. Correção / base
- [ ] **Timing de ciclos exato** (wait states por região de memória) — hoje cada
      instrução conta como 1 ciclo (placeholder). Melhora precisão e o pitch fino
      do áudio.
- [ ] `cargo fmt --all` num commit dedicado de estilo (pro CI de fmt passar).
- [ ] Testes de integração rodando ROMs homebrew/comerciais.

### 4. Cartridge / Saves — resto
- [X] EEPROM via DMA (região 0x0D) — 512 B/8 KB, detecção automática do tamanho.

### 5. Android (Fase 7)
- [ ] Concretizar o port (a crate `crates/android/` tem só o esqueleto JNI).

---

## Ferramentas/dicas de debug úteis

- **Smoke test com dump de tela**: roda uma ROM e salva o framebuffer.
  ```bash
  AURORA_DUMP=/tmp/frame.ppm cargo run --release --bin smoke -- <rom.gba> 12000000
  ```
  Depois converter o PPM para PNG para inspecionar (foi assim que mapeamos os
  "Failed test N" do jsmolka). O smoke também imprime nº de cores distintas.
- **Mapear "Failed test N"** dos ROMs jsmolka: o nº do teste mapeia para um
  arquivo `.asm` em `github.com/jsmolka/gba-tests` (ex.: arm tests 450–499 =
  `data_swap.asm`). Útil para achar bugs de CPU.
- **Rodar o emulador com GUI**: `cargo run --release -p auroragba-desktop`.

## Comandos rápidos
```bash
cargo test                              # todos os testes
cargo clippy --all-targets -- -D warnings   # lint estrito (CI)
cargo run --release -p auroragba-desktop    # abre a GUI
./scripts/fetch-test-roms.sh            # baixa os ROMs de teste do jsmolka
```
