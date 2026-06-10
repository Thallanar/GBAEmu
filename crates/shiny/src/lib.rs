//! Módulo Shiny Hunter — automatiza a caça de Pokémon shiny.
//!
//! Suporte inicial: Gen 3 (Ruby/Sapphire/Emerald, FireRed/LeafGreen).
//!
//! Arquitetura (data-driven):
//!   - [`games::GameProfile`] descreve, em dados puros, onde cada jogo guarda os
//!     Pokémon na RAM. O emulador identifica o jogo pelo header e carrega o
//!     perfil ([`games::detect`]).
//!   - O [`Hunter`] dirige o loop genérico: reset → avança até o encontro → lê
//!     o PID do slot do alvo → checa a fórmula shiny → repete se não for.
//!
//! A shininess depende do **TID/SID do jogador** (constante do save), não do
//! Pokémon: por isso lemos o TID/SID do líder do time do jogador e o PID do
//! slot específico do alvo. Não fazemos varredura — endereçamos o slot exato,
//! então o time do jogador estar shiny nunca confunde a leitura do selvagem.

use auroragba_core::joypad::Button;
use auroragba_core::Gba;

pub mod games;
pub mod gfx;

use games::{GameProfile, HuntMethod, TargetDef};

/// Força (em malha fechada) a seleção do inicial pro Poké Ball do alvo.
///
/// Em vez de "apertar direção" (que andava com o personagem no overworld e
/// confirmava o centro assim que a bag abria), escrevemos o byte da seleção
/// (`gTasks[i].data[0]`) direto — **mas só** quando a task que processa a direção
/// está ativa (`gTasks[i].func == input_func`), i.e. a bag está aberta aceitando
/// input. Fora disso é no-op, então não há clobber em cutscene nem chute de
/// frame. O A do loop, ao registrar uma borda, confirma a seleção já forçada.
fn force_starter_cursor(gba: &mut Gba, profile: &GameProfile, target: &TargetDef) {
    if target.method != HuntMethod::Starter {
        return;
    }
    let Some(menu) = profile.starter_menu else {
        return;
    };
    // `func` (offset 0 da struct Task) fica 8 bytes antes de `data[0]`. A lista
    // tem um func por revisão da ROM (mesmo game code, endereços diferentes).
    if !menu
        .input_funcs
        .contains(&gba.bus.read_u32(menu.cursor_addr - 8))
    {
        return; // menu não está aberto/aceitando direção
    }
    gba.bus.write_u8(menu.cursor_addr, target.cursor.value());
}

/// Fórmula shiny da Gen 3.
/// Shiny se `(PID_hi ^ PID_lo ^ TID ^ SID) < 8`.
#[inline]
pub fn is_shiny_gen3(pid: u32, tid: u16, sid: u16) -> bool {
    shiny_value(pid, tid, sid) < 8
}

/// Valor bruto da fórmula shiny (`< 8` ⇒ shiny). Útil pra UI ("passou perto").
#[inline]
pub fn shiny_value(pid: u32, tid: u16, sid: u16) -> u16 {
    let pid_hi = (pid >> 16) as u16;
    let pid_lo = pid as u16;
    pid_hi ^ pid_lo ^ tid ^ sid
}

// ───────────────────────── leitura de Pokémon (Gen 3) ───────────────────────

/// Ordem das 4 sub-structs (Growth=0, Attacks=1, EVs=2, Misc=3) conforme
/// `PID % 24`. Cada linha lista os tipos na ordem em que aparecem na memória.
const SUBSTRUCT_ORDER: [[u8; 4]; 24] = [
    [0, 1, 2, 3],
    [0, 1, 3, 2],
    [0, 2, 1, 3],
    [0, 2, 3, 1],
    [0, 3, 1, 2],
    [0, 3, 2, 1],
    [1, 0, 2, 3],
    [1, 0, 3, 2],
    [1, 2, 0, 3],
    [1, 2, 3, 0],
    [1, 3, 0, 2],
    [1, 3, 2, 0],
    [2, 0, 1, 3],
    [2, 0, 3, 1],
    [2, 1, 0, 3],
    [2, 1, 3, 0],
    [2, 3, 0, 1],
    [2, 3, 1, 0],
    [3, 0, 1, 2],
    [3, 0, 2, 1],
    [3, 1, 0, 2],
    [3, 1, 2, 0],
    [3, 2, 0, 1],
    [3, 2, 1, 0],
];

/// Dados de um Pokémon Gen 3 lidos da RAM.
#[derive(Debug, Clone, Copy)]
pub struct Gen3Mon {
    /// Personality value (não-criptografado, offset 0x00).
    pub pid: u32,
    /// OT ID: TID nos 16 bits baixos, SID nos altos (offset 0x04).
    pub otid: u32,
    /// Espécie (índice interno), lida da sub-struct Growth descriptografada.
    pub species: u16,
    /// `true` se o checksum confere — ou seja, os dados são reais e frescos
    /// (não lixo de uma tentativa anterior nem memória zerada).
    pub valid: bool,
}

impl Gen3Mon {
    pub fn tid(&self) -> u16 {
        self.otid as u16
    }
    pub fn sid(&self) -> u16 {
        (self.otid >> 16) as u16
    }
}

/// Lê e descriptografa o Pokémon no endereço-base dado.
///
/// PID e OT ID ficam em claro nos primeiros 8 bytes. As 48 bytes seguintes são
/// 4 sub-structs de 12 bytes, cada palavra XOR com `key = PID ^ OTID`; a ordem
/// é dada por `PID % 24`. A espécie é o primeiro campo da sub-struct Growth.
pub fn read_mon(gba: &mut Gba, base: u32) -> Gen3Mon {
    let pid = gba.bus.read_u32(base);
    let otid = gba.bus.read_u32(base + 0x04);
    let stored_checksum = gba.bus.read_u16(base + 0x1C);
    let key = pid ^ otid;

    // Descriptografa as 12 palavras (48 bytes) e acumula o checksum.
    let mut words = [0u32; 12];
    let mut sum: u32 = 0;
    for (i, w) in words.iter_mut().enumerate() {
        let enc = gba.bus.read_u32(base + 0x20 + (i as u32) * 4);
        let dec = enc ^ key;
        *w = dec;
        sum = sum.wrapping_add(dec & 0xFFFF).wrapping_add(dec >> 16);
    }
    // pid==0 ⇒ slot vazio (ex.: time sem Pokémon antes de escolher o inicial):
    // não é um encontro real, mesmo que o checksum "bata" (0 == 0).
    let valid = pid != 0 && (sum as u16) == stored_checksum;

    // A espécie é o 1º halfword da sub-struct Growth (tipo 0).
    let order = SUBSTRUCT_ORDER[(pid % 24) as usize];
    let growth_slot = order.iter().position(|&t| t == 0).unwrap();
    let species = (words[growth_slot * 3] & 0xFFFF) as u16;

    Gen3Mon {
        pid,
        otid,
        species,
        valid,
    }
}

// ───────────────────────────── Hunter (loop) ────────────────────────────────

/// Resultado de uma checagem de encontro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    /// O encontro ainda não está pronto (dados inválidos / espécie errada).
    NotReady,
    /// Encontro pronto, mas não é shiny.
    NotShiny,
    /// ✨ Shiny!
    Shiny,
}

/// Driver da caça. Mantém o estado entre tentativas para a UI exibir.
pub struct Hunter {
    pub attempts: u64,
    pub found: bool,
    /// PID lido na última checagem (pra UI/log).
    pub last_pid: u32,
    /// Valor bruto da fórmula na última checagem (`< 8` ⇒ shiny).
    pub last_shiny_value: u16,
    /// Espécie (índice interno) lida no último encontro — confirma na UI que a
    /// caça parou no Pokémon certo.
    pub last_species: u16,
    /// Menor `shiny_value` já visto nesta caça (quão perto chegou de um shiny —
    /// `0xFFFF` = nada ainda). A UI mostra isso como "mais perto".
    pub best_shiny_value: u16,
    /// Número da tentativa em que o `best_shiny_value` aconteceu.
    pub best_attempt: u64,
    /// Frames já gastos na tentativa em andamento (controle do `tick`).
    frames_this_attempt: u32,
    /// Seed do RNG do jogo a injetar **nesta** tentativa (sorteada do PRNG do
    /// host). É o que de fato dá um PID diferente a cada reset — ver
    /// [`Hunter::maybe_inject_seed`].
    pending_seed: u32,
    /// `true` depois que `pending_seed` já foi escrito na RAM nesta tentativa
    /// (evita reinjetar a cada frame).
    seed_injected: bool,
    /// Estado do PRNG do host (SplitMix64). Semeado no `new()` por entropia real
    /// (relógio + PID do processo), então **único por instância** — é isso que
    /// faz vários emuladores abertos juntos gerarem PIDs diferentes em vez de
    /// rodarem a mesma sequência determinística.
    rng_state: u64,
}

/// Frame (contado desde o reset) em que injetamos a seed do RNG do jogo. Tem que
/// cair *depois* de o jogo inicializar `gRngValue` no boot e *antes* de o PID do
/// encontro ser sorteado. Medido empiricamente no Emerald: a janela segura vai
/// de ~60 a ~500 frames (o PID é rolado entre 500 e 800); 200 fica folgado nas
/// duas pontas.
const SEED_INJECT_FRAME: u32 = 200;
/// Largura de cada meio-ciclo do tap de A (8 pressionado / 8 solto). Cadência
/// fixa: a entropia agora vem da seed injetada, não mais do timing.
const MASH_PERIOD: u32 = 8;

impl Hunter {
    pub fn new() -> Self {
        let mut h = Self {
            attempts: 0,
            found: false,
            last_pid: 0,
            last_shiny_value: 0xFFFF,
            last_species: 0,
            best_shiny_value: 0xFFFF,
            best_attempt: 0,
            frames_this_attempt: 0,
            pending_seed: 0,
            seed_injected: false,
            rng_state: Self::host_seed(),
        };
        // Sorteia a seed da 1ª tentativa já no boot (a 1ª caça não passa por
        // soft_reset antes do primeiro encontro).
        h.reroll_seed();
        h
    }

    /// Semente de entropia **real** do host: instante atual (nanos) misturado com
    /// o PID do processo. Dois emuladores abertos no mesmo instante ainda diferem
    /// pelo PID, então cada instância parte de um ponto distinto do PRNG.
    fn host_seed() -> u64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        nanos ^ ((std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    /// Próximo valor do PRNG do host (SplitMix64). Não-periódico na prática
    /// (período 2⁶⁴) e semeado por entropia real — substitui a antiga fórmula
    /// `attempts % 300`, que repetia a cada 300 tentativas e era idêntica entre
    /// instâncias.
    fn next_rng(&mut self) -> u64 {
        self.rng_state = self.rng_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Sorteia, do PRNG do host, a seed do RNG do jogo pra próxima tentativa.
    /// Como vem de um PRNG de período 2⁶⁴ semeado por instância, nunca repete em
    /// ciclo nem coincide entre emuladores — cada tentativa pega um ponto
    /// independente do espaço de 2³² PIDs.
    fn reroll_seed(&mut self) {
        self.pending_seed = self.next_rng() as u32;
        self.seed_injected = false;
    }

    /// No frame certo da tentativa, escreve `pending_seed` em `gRngValue` do jogo
    /// (endereço no perfil). É a fonte de entropia da caça: sem isso o RNG do
    /// Emerald é determinístico (seed fixa no boot) e todo reset, com o mesmo
    /// roteiro de inputs, geraria o **mesmo** PID — medido: 1 único PID em 20
    /// resets. Com a injeção: 20/20 distintos. Jogos sem `rng_addr` não têm
    /// entropia (a caça vira determinística) — por ora só Emerald é suportado.
    fn maybe_inject_seed(&mut self, gba: &mut Gba, profile: &GameProfile) {
        if self.seed_injected || self.frames_this_attempt != SEED_INJECT_FRAME {
            return;
        }
        if let Some(addr) = profile.rng_addr {
            gba.bus.write_u32(addr, self.pending_seed);
            self.seed_injected = true;
        }
    }

    /// Power-cycle do console (preserva o Flash). Volta o jogo à tela de título;
    /// daí o `tick`/`advance_to_encounter` amassa A até a batalha. Sorteia uma
    /// nova seed pra próxima tentativa render um PID diferente.
    pub fn soft_reset(&mut self, gba: &mut Gba) {
        gba.reset();
        self.frames_this_attempt = 0;
        self.reroll_seed();
    }

    /// Avança a emulação "amassando" A e Start (passa título → continuar →
    /// diálogos → batalha) até o alvo carregar válido na RAM, ou estourar
    /// `max_frames`. Retorna `true` se o encontro ficou pronto.
    pub fn advance_to_encounter(
        &mut self,
        gba: &mut Gba,
        profile: &GameProfile,
        target: &TargetDef,
        max_frames: u32,
    ) -> bool {
        let base = profile.target_base(target);
        self.frames_this_attempt = 0;
        self.seed_injected = false;
        for _ in 0..max_frames {
            self.maybe_inject_seed(gba, profile);
            // Força o cursor do inicial pro alvo quando a bag está aberta (no-op
            // pros demais alvos/telas).
            force_starter_cursor(gba, profile, target);
            // Tapa A (ver `tick`): cobre título → continuar → bag → diálogos.
            let press = (self.frames_this_attempt / MASH_PERIOD).is_multiple_of(2);
            gba.bus.io.joypad.set_button(Button::A, press);
            gba.run_frame();
            self.frames_this_attempt += 1;

            if self.encounter_ready(gba, base, target) {
                gba.bus.io.joypad.set_button(Button::A, false);
                return true;
            }
        }
        false
    }

    /// O encontro está pronto? Exige checksum válido e, se o alvo especificar
    /// uma espécie, que ela bata.
    fn encounter_ready(&self, gba: &mut Gba, base: u32, target: &TargetDef) -> bool {
        let mon = read_mon(gba, base);
        mon.valid && (target.species == 0 || mon.species == target.species)
    }

    /// Checa o slot do alvo contra a fórmula shiny, usando o TID/SID do líder do
    /// time do jogador. Atualiza o estado pra UI.
    pub fn check(
        &mut self,
        gba: &mut Gba,
        profile: &GameProfile,
        target: &TargetDef,
    ) -> CheckResult {
        let base = profile.target_base(target);
        let target_mon = read_mon(gba, base);
        if !target_mon.valid {
            return CheckResult::NotReady;
        }
        // Encontro real → conta como uma tentativa.
        self.attempts += 1;

        // TID/SID do JOGADOR vêm do OT ID do líder do seu time.
        let player = read_mon(gba, profile.player_party);
        let (tid, sid) = (player.tid(), player.sid());

        self.last_pid = target_mon.pid;
        self.last_species = target_mon.species;
        self.last_shiny_value = shiny_value(target_mon.pid, tid, sid);

        // Recorde de "quão perto" — menor valor já visto e em que tentativa.
        if self.last_shiny_value < self.best_shiny_value {
            self.best_shiny_value = self.last_shiny_value;
            self.best_attempt = self.attempts;
        }

        if self.last_shiny_value < 8 {
            self.found = true;
            log::info!(
                "✨ SHINY {} após {} tentativas! PID={:08X}",
                target.name,
                self.attempts,
                target_mon.pid
            );
            CheckResult::Shiny
        } else {
            CheckResult::NotShiny
        }
    }

    /// Passo **não-bloqueante** da caça, pra rodar 1×/update da UI sem travá-la.
    ///
    /// Avança até `batch` frames amassando A/Start. Quando o encontro carrega,
    /// checa shiny: se for, marca `found` e para (a UI pausa na tela do shiny);
    /// se não, faz soft-reset e a próxima chamada recomeça. Se a tentativa
    /// passar de `attempt_timeout` frames sem chegar ao encontro, reseta também
    /// (evita travar numa tela inesperada).
    pub fn tick(
        &mut self,
        gba: &mut Gba,
        profile: &GameProfile,
        target: &TargetDef,
        batch: u32,
        attempt_timeout: u32,
    ) -> CheckResult {
        if self.found {
            return CheckResult::Shiny;
        }
        let base = profile.target_base(target);

        for _ in 0..batch {
            // No frame certo, injeta a seed do RNG do jogo (a entropia da caça).
            self.maybe_inject_seed(gba, profile);

            // Com a bag aberta, força o cursor pro inicial alvo (malha fechada);
            // no-op nas demais telas e pros demais alvos.
            force_starter_cursor(gba, profile, target);

            // Tapa A (8 frames pressionado / 8 solto): confirma no título,
            // escolhe "Continuar", abre a bag, confirma o inicial (já forçado) e
            // avança diálogos — tudo com A. Tap (não segurar) pra cada prompt
            // registrar uma borda de tecla.
            let press = (self.frames_this_attempt / MASH_PERIOD).is_multiple_of(2);
            gba.bus.io.joypad.set_button(Button::A, press);
            gba.run_frame();
            self.frames_this_attempt += 1;

            if self.encounter_ready(gba, base, target) {
                gba.bus.io.joypad.set_button(Button::A, false);
                let result = self.check(gba, profile, target);
                if result != CheckResult::Shiny {
                    self.soft_reset(gba);
                }
                return result;
            }

            if self.frames_this_attempt >= attempt_timeout {
                self.soft_reset(gba);
                return CheckResult::NotReady;
            }
        }
        CheckResult::NotReady
    }

    /// Uma tentativa completa (bloqueante): reset → avança → checa.
    /// Usada em testes/headless; a UI roda os passos separados pra não travar.
    pub fn try_once(
        &mut self,
        gba: &mut Gba,
        profile: &GameProfile,
        target: &TargetDef,
    ) -> CheckResult {
        // Lendários precisam de soft-reset; outros métodos virão depois.
        if target.method == HuntMethod::SoftResetLegendary {
            self.soft_reset(gba);
        }
        if !self.advance_to_encounter(gba, profile, target, 60 * 60) {
            return CheckResult::NotReady;
        }
        self.check(gba, profile, target)
    }
}

impl Default for Hunter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use games::{Slot, StarterCursor, StarterMenu, TargetDef};

    #[test]
    fn force_starter_cursor_only_writes_when_menu_open() {
        let cursor_addr = 0x0300_5E08;
        let func_addr = cursor_addr - 8;
        let input_func = 0x0813_425D;
        let profile = GameProfile {
            code: "TEST",
            name: "test",
            player_party: 0x0200_0000,
            enemy_party: 0x0200_1000,
            rng_addr: None,
            starter_menu: Some(StarterMenu {
                cursor_addr,
                input_funcs: &[0x0813_425D],
            }),
            targets: &[],
        };
        let mudkip = TargetDef {
            name: "Mudkip",
            species: 283,
            slot: Slot::Player,
            method: HuntMethod::Starter,
            cursor: StarterCursor::Right, // valor 2
        };

        let mut gba = Gba::new();
        // Menu fechado (func != input_func): não escreve.
        gba.bus.write_u32(func_addr, 0xDEAD_BEEF);
        gba.bus.write_u8(cursor_addr, 1);
        force_starter_cursor(&mut gba, &profile, &mudkip);
        assert_eq!(gba.bus.read_u8(cursor_addr), 1, "menu fechado não deve forçar");

        // Menu aberto: força a seleção pro Poké Ball do alvo (2 = direita).
        gba.bus.write_u32(func_addr, input_func);
        force_starter_cursor(&mut gba, &profile, &mudkip);
        assert_eq!(gba.bus.read_u8(cursor_addr), 2, "deveria forçar Mudkip (2)");

        // Método não-Starter (lendário) é no-op mesmo com a func batendo.
        let legend = TargetDef {
            method: HuntMethod::SoftResetLegendary,
            ..mudkip
        };
        gba.bus.write_u8(cursor_addr, 1);
        force_starter_cursor(&mut gba, &profile, &legend);
        assert_eq!(gba.bus.read_u8(cursor_addr), 1, "lendário não mexe no cursor");
    }

    #[test]
    fn shiny_formula_known_values() {
        assert!(is_shiny_gen3(0, 0, 0));
        assert!(!is_shiny_gen3(0xABCD_1234, 0x1111, 0x2222));
    }

    /// Escreve um Pokémon Gen 3 sintético na RAM (EWRAM) pra testar a leitura.
    /// Faz o caminho inverso da descriptografia: cifra a sub-struct Growth com
    /// a espécie e calcula o checksum correto.
    fn write_synthetic_mon(gba: &mut Gba, base: u32, pid: u32, otid: u32, species: u16) {
        gba.bus.write_u32(base, pid);
        gba.bus.write_u32(base + 0x04, otid);

        let key = pid ^ otid;
        // Monta 12 palavras descriptografadas; só Growth[0] (espécie) é não-zero.
        let mut words = [0u32; 12];
        let order = SUBSTRUCT_ORDER[(pid % 24) as usize];
        let growth_slot = order.iter().position(|&t| t == 0).unwrap();
        words[growth_slot * 3] = species as u32;

        // Checksum = soma dos 24 halfwords das palavras em claro.
        let mut sum: u32 = 0;
        for w in words {
            sum = sum.wrapping_add(w & 0xFFFF).wrapping_add(w >> 16);
        }
        gba.bus.write_u16(base + 0x1C, sum as u16);

        // Grava as palavras cifradas (XOR key).
        for (i, w) in words.iter().enumerate() {
            gba.bus.write_u32(base + 0x20 + (i as u32) * 4, w ^ key);
        }
    }

    #[test]
    fn read_mon_decrypts_species_and_validates() {
        let mut gba = Gba::new();
        let base = 0x0200_0000; // EWRAM
        write_synthetic_mon(&mut gba, base, 0x1234_5678, 0xDEAD_BEEF, 384);

        let mon = read_mon(&mut gba, base);
        assert_eq!(mon.pid, 0x1234_5678);
        assert_eq!(mon.otid, 0xDEAD_BEEF);
        assert_eq!(mon.species, 384);
        assert!(mon.valid, "checksum deveria conferir");
    }

    #[test]
    fn read_mon_empty_slot_is_invalid() {
        // Slot zerado (time sem Pokémon): checksum 0 == 0 "bate", mas pid==0
        // ⇒ tem que ser inválido pra não disparar falso encontro no inicial.
        let mut gba = Gba::new();
        let mon = read_mon(&mut gba, 0x0200_0200);
        assert_eq!(mon.pid, 0);
        assert!(!mon.valid);
    }

    #[test]
    fn read_mon_flags_garbage_as_invalid() {
        let mut gba = Gba::new();
        let base = 0x0200_0100;
        // Sem escrever nada coerente: checksum não vai bater.
        gba.bus.write_u32(base, 0x1111_1111);
        gba.bus.write_u16(base + 0x1C, 0x9999);
        let mon = read_mon(&mut gba, base);
        assert!(!mon.valid);
    }

    /// As seeds de uma única instância não podem repetir em ciclo curto (o bug
    /// antigo: a fórmula `attempts % 300` repetia a cada 300 tentativas). Colhe
    /// muitas seeds e exige diversidade quase total.
    #[test]
    fn seeds_do_not_cycle() {
        let mut h = Hunter::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..5000 {
            h.reroll_seed();
            seen.insert(h.pending_seed);
        }
        // PRNG de 32 bits: 5000 sorteios quase não colidem (aniversário ~0.3%).
        assert!(
            seen.len() > 4990,
            "seeds pouco diversas ({} únicas) — voltou a repetir em ciclo?",
            seen.len()
        );
    }

    /// Duas instâncias criadas separadamente não podem produzir a mesma
    /// sequência de seeds (o bug antigo: 4 emuladores idênticos explorando os
    /// mesmos PIDs).
    #[test]
    fn instances_diverge() {
        let (mut a, mut b) = (Hunter::new(), Hunter::new());
        let seq = |h: &mut Hunter| {
            (0..50)
                .map(|_| {
                    h.reroll_seed();
                    h.pending_seed
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(seq(&mut a), seq(&mut b), "instâncias geraram a MESMA sequência");
    }

    /// `best_shiny_value`/`best_attempt` guardam o menor valor já visto e quando.
    #[test]
    fn tracks_closest_shiny_value() {
        let mut gba = Gba::new();
        let profile = GameProfile {
            code: "TEST",
            name: "test",
            player_party: 0x0200_0000,
            enemy_party: 0x0200_1000,
            rng_addr: None,
            starter_menu: None,
            targets: &[],
        };
        let target = TargetDef {
            name: "alvo",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        };
        let otid = 0x2222_1111; // TID=0x1111, SID=0x2222 → TID^SID = 0x3333
        write_synthetic_mon(&mut gba, profile.player_party, 0xAAAA_BBBB, otid, 1);

        let mut hunter = Hunter::new();
        // PIDs escolhidos pra dar shiny_value 50, depois 20, depois 80.
        for (pid, expected_sv) in [(0x0000_3301u32, 50u16), (0x0000_3327, 20), (0x0000_3363, 80)] {
            write_synthetic_mon(&mut gba, profile.enemy_party, pid, otid, 100);
            hunter.check(&mut gba, &profile, &target);
            assert_eq!(hunter.last_shiny_value, expected_sv);
        }
        // O recorde é o menor (20), fixado na 2ª tentativa.
        assert_eq!(hunter.best_shiny_value, 20);
        assert_eq!(hunter.best_attempt, 2);
    }

    #[test]
    fn check_detects_shiny_from_player_ids() {
        let mut gba = Gba::new();
        // Perfil sintético apontando pra dois cantos da EWRAM.
        let profile = GameProfile {
            code: "TEST",
            name: "test",
            player_party: 0x0200_0000,
            enemy_party: 0x0200_1000,
            rng_addr: None,
            starter_menu: None,
            targets: &[],
        };
        let target = TargetDef {
            name: "alvo",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        };

        // Jogador com TID=0x1111, SID=0x2222 (OTID = SID<<16 | TID).
        let otid = 0x2222_1111;
        write_synthetic_mon(&mut gba, profile.player_party, 0xAAAA_BBBB, otid, 1);

        // PID do alvo escolhido pra dar shiny: (PIDhi^PIDlo^TID^SID) < 8.
        // Com TID^SID = 0x1111^0x2222 = 0x3333, escolho PID tal que
        // PIDhi^PIDlo == 0x3333 → resultado 0 (shiny).
        let pid = 0x0000_3333; // hi=0x0000, lo=0x3333 → 0x3333
        write_synthetic_mon(&mut gba, profile.enemy_party, pid, otid, 100);

        let mut hunter = Hunter::new();
        assert_eq!(
            hunter.check(&mut gba, &profile, &target),
            CheckResult::Shiny
        );
        assert!(hunter.found);
        assert_eq!(hunter.last_pid, pid);

        // Um PID claramente não-shiny.
        write_synthetic_mon(&mut gba, profile.enemy_party, 0x1234_5678, otid, 100);
        let mut hunter2 = Hunter::new();
        assert_eq!(
            hunter2.check(&mut gba, &profile, &target),
            CheckResult::NotShiny
        );
    }
}
