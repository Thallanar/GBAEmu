# 🌌 AuroraGBA

Emulador de **Game Boy Advance** escrito em Rust, multiplataforma (Windows, Linux, Android), com modo diferencial **Shiny Hunter** para automação de caça a Pokémon shiny.

> ⚠️ **Em desenvolvimento inicial (Fase 0).** Veja o [ROADMAP](ROADMAP.md) para o plano completo.

---

## Features planejadas

- ✅ Emulação completa do GBA (CPU ARM7TDMI, PPU, APU, DMA, timers)
- ✅ Save states e fast-forward
- ✅ Gamepad e teclado configuráveis
- ✨ **Shiny Hunter Mode** — automação de soft reset com detecção via leitura de RAM
- 📱 Build para Android com overlay touch

## Build (desktop)

Requer Rust ≥ 1.75. Instale via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Compile e rode:

```bash
cargo run --release -p auroragba-desktop
```

## Estrutura do workspace

```
auroragba/
├── crates/
│   ├── core/      # Engine de emulação (CPU, PPU, APU, memória)
│   ├── desktop/   # Frontend desktop (egui)
│   ├── shiny/     # Módulo Shiny Hunter
│   └── android/   # Bindings JNI para Android
├── ROADMAP.md
└── README.md
```

## Aviso legal

- **BIOS:** o `gba_bios.bin` não é distribuído com este projeto. Forneça o seu próprio.
- **ROMs:** carregue apenas ROMs que você possui legalmente.

## Licença

GPL-3.0-or-later.
