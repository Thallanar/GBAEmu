# AuroraGBA — Próximos passos

> Arquivo de continuidade: "de onde paramos". O roadmap completo está em
> [`ROADMAP.md`](./ROADMAP.md); aqui fica o estado atual + o que atacar a seguir.

_Última atualização: 2026-06-02_

---

## Onde paramos (estado atual)

O emulador **boota e renderiza** jogos via vídeo+input, sem som ainda.

Funcionando e validado:
- **CPU ARM7TDMI** completa (ARM + THUMB), IRQ, banking de registradores.
- **BIOS HLE** (sem BIOS oficial): ~25 funções SWI + trampolim de IRQ + direct boot.
- **DMA** (4 canais: imediato/VBlank/HBlank).
- **Timers** (4, com cascade e prescalers).
- **Joypad** (KEYINPUT/KEYCNT + IRQ de keypad; teclado mapeado no desktop).
- **PPU**: modos bitmap 3/4/5 + tile 0/1/2 (texto e afim) + sprites (OBJ
  normal/afim, 1D/2D), composição por prioridade.
- **Saves**: SRAM (32 KB) + Flash 64K/128K (máquina de comandos completa: chip
  ID, apagar chip/setor, gravar byte, troca de banco) + persistência em `.sav`
  (carrega no boot, grava ~1×/s e ao fechar). EEPROM ainda pendente.
- **Frontend desktop** (egui): abrir ROM, framebuffer escalável, input, save.
- **81/81** testes unitários, clippy estrito limpo.
- ✅ **jsmolka gba-tests: arm.gba, thumb.gba, memory.gba passam 100%**
  ("All tests passed") — CPU e memória validados contra a suíte de referência.

Controles do desktop: Z=A, X=B, Enter=Start, Backspace=Select, setas=direcional,
A=L, S=R.

---

## Próximos passos (em ordem sugerida)

### 1. 🌟 Shiny Hunter Mode — ✅ esqueleto funcional, falta validar com ROM real
Arquitetura **data-driven**: o emulador identifica o jogo pelo game code do
header (`cartridge.game_code()`) e carrega um `GameProfile` de `crates/shiny/
src/games.rs`. Adicionar um jogo = uma entrada na tabela, sem mexer na lógica.

Já feito:
- [X] `read_mon()`: lê/descriptografa Pokémon Gen 3 (PID/OTID/espécie, valida por
      checksum). Chave `PID^OTID`, ordem das sub-structs por `PID%24`.
- [X] Detector shiny `is_shiny_gen3` / `shiny_value`.
- [X] `Hunter::tick()` não-bloqueante: amassa A/Start → detecta encontro pronto
      (checksum válido + espécie) → checa shiny → soft-reset se não for.
- [X] `Gba::reset()`: power-cycle preservando o Flash (o save sobrevive).
- [X] UI no desktop: menu Shiny Hunter (seletor de alvo, iniciar/parar, contador,
      último PID + valor, banner ao achar; pausa na tela do shiny).
- [X] Emerald semeado (Rayquaza/Groudon/Kyogre).

O que falta (precisa de **ROM de Pokémon Gen 3** do usuário):
- [ ] **Confirmar os endereços** `gPlayerParty`/`gEnemyParty` por versão. Se
      errados, `read_mon` lê lixo e a caça nunca "encontra" (o `valid` por
      checksum protege contra falso-positivo).
- [ ] Ajustar o roteiro de inputs (timing/menus reais).
- [ ] Preencher o índice **interno** de espécie nos `TargetDef` (Hoenn difere do
      dex nacional) p/ verificação mais estrita.
- [ ] Métodos: starters (menu do laboratório) e random encounters.

Como testar: salvar na frente do lendário → menu Shiny Hunter → escolher alvo →
Iniciar. Contador não sobe ⇒ endereços a corrigir.

### 2. APU — Som (deixado para depois; CONVERSADO em 2026-06-01)
Bloco grande, sensível a timing, **não ajuda o Shiny Hunter**. Duas metades:
- **4 canais PSG** (2 ondas quadradas, 1 wave programável, 1 ruído) — registradores
  `0x04000060`–`0x04000088`.
- **Direct Sound** (2 canais PCM 8-bit): FIFO + Timer 0/1 + **DMA "special"**
  (aquele modo de DMA que ficou pendente — implementar isto fecha o buraco).
- Saída no host via **`cpal`** (já na stack): ring buffer + callback em thread +
  resampling da taxa do GBA. O áudio normalmente vira o "relógio" da emulação.

Sub-tarefas:
- [ ] `apu.rs`: registradores + geração por canal + mixer (SOUNDCNT/SOUNDBIAS).
- [ ] DMA modo special acionado por overflow de Timer 0/1 alimentando a FIFO.
- [ ] Integração `cpal` + sincronização de framerate pelo consumo de áudio.

### 3. PPU — efeitos que faltam
- [ ] Janelas (WIN0/WIN1/OBJ window) — `WININ`/`WINOUT`.
- [ ] Blending / alpha (`BLDCNT`/`BLDALPHA`/`BLDY`) — transparências, fades.
- [ ] Mosaic (`MOSAIC`).
- [ ] Sprite semi-transparente e OBJ window mode.

### 4. Cartridge / Saves (Fase 4) — ✅ maior parte feita
- [X] SRAM (32 KB) + Flash 64K/128K (máquina de comandos) + persistência `.sav`.
- [ ] EEPROM via DMA (acesso serial na região 0x0D) — Pokémon Gen 3 NÃO usa, então
      é baixa prioridade; necessário só para jogos específicos.
- [ ] Save states (snapshot completo do estado do emulador — diferente do `.sav`
      do jogo; útil pra UI de save/load instantâneo).

### 5. Polimento / qualidade
- [ ] `cargo fmt --all` no repositório inteiro (dívida pré-existente: commits
      antigos não foram formatados; arquivos novos já estão fmt-clean). Fazer num
      commit dedicado de estilo para o CI de fmt passar.
- [ ] Timing de ciclos exato (wait states por região de memória) — hoje cada
      instrução conta como 1 ciclo (placeholder).
- [ ] Testes de integração rodando ROMs homebrew/comerciais.

### 6. Android (Fase 7)
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
