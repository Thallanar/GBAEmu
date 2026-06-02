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

use games::{GameProfile, HuntMethod, TargetDef};

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
    /// Frames já gastos na tentativa em andamento (controle do `tick`).
    frames_this_attempt: u32,
    /// Frames de "espera" no início da tentativa antes de começar a amassar A.
    /// Varia por tentativa pra injetar entropia de timing — essencial no Emerald,
    /// cujo RNG é determinístico (seed 0 no boot); sem isso, todo reset geraria
    /// o mesmo PID.
    entropy_delay: u32,
}

impl Hunter {
    pub fn new() -> Self {
        Self {
            attempts: 0,
            found: false,
            last_pid: 0,
            last_shiny_value: 0xFFFF,
            frames_this_attempt: 0,
            entropy_delay: 0,
        }
    }

    /// Espera (em frames) a aplicar no início da próxima tentativa, derivada do
    /// nº de tentativas via hash — pseudo-aleatória mas determinística/reproduzível.
    /// Espalha o "frame de geração" do PID ao longo de ~5s pra variar a seed.
    fn next_delay(&self) -> u32 {
        (self.attempts.wrapping_mul(0x9E37_79B1) % 300) as u32
    }

    /// Power-cycle do console (preserva o Flash). Volta o jogo à tela de título;
    /// daí o `tick`/`advance_to_encounter` amassa A até a batalha. Sorteia uma
    /// nova espera de entropia pra próxima tentativa render um PID diferente.
    pub fn soft_reset(&mut self, gba: &mut Gba) {
        gba.reset();
        self.frames_this_attempt = 0;
        self.entropy_delay = self.next_delay();
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
        for frame in 0..max_frames {
            // Tapa A (ver `tick`): cobre título → continuar → bag → diálogos.
            let press = (frame / 8).is_multiple_of(2);
            gba.bus.io.joypad.set_button(Button::A, press);
            gba.run_frame();

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
        self.last_shiny_value = shiny_value(target_mon.pid, tid, sid);

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
            // Espera de entropia: idle no começo da tentativa pra deslocar o
            // frame de geração do PID (ver `entropy_delay`).
            if self.frames_this_attempt < self.entropy_delay {
                gba.bus.io.joypad.set_button(Button::A, false);
                gba.run_frame();
                self.frames_this_attempt += 1;
                continue;
            }

            // Tapa A (8 frames pressionado / 8 solto): confirma no título,
            // escolhe "Continuar", abre a bag, seleciona/confirma o inicial e
            // avança diálogos — tudo com A. Tap (não segurar) pra cada prompt
            // registrar uma borda de tecla.
            let phase = self.frames_this_attempt - self.entropy_delay;
            let press = (phase / 8).is_multiple_of(2);
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
    use games::{Slot, TargetDef};

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

    #[test]
    fn check_detects_shiny_from_player_ids() {
        let mut gba = Gba::new();
        // Perfil sintético apontando pra dois cantos da EWRAM.
        let profile = GameProfile {
            code: "TEST",
            name: "test",
            player_party: 0x0200_0000,
            enemy_party: 0x0200_1000,
            targets: &[],
        };
        let target = TargetDef {
            name: "alvo",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
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
