//! Módulo Shiny Hunter — automatiza a caça de Pokémon shiny.
//!
//! Suporte inicial: Gen 3 (Ruby/Sapphire/Emerald, FireRed/LeafGreen).
//!
//! Estratégia:
//!   - Cada jogo implementa [`ShinyTarget`], descrevendo onde encontrar o PID
//!     do Pokémon na RAM e a sequência de inputs para o reset.
//!   - O [`Hunter`] dirige o loop: roda emulação, lê PID, checa fórmula shiny,
//!     soft-reset e repete até encontrar.

use auroragba_core::Gba;

pub mod games;

/// Fórmula shiny da Gen 3.
/// Shiny se `(PID_hi ^ PID_lo ^ TID ^ SID) < 8`.
#[inline]
pub fn is_shiny_gen3(pid: u32, tid: u16, sid: u16) -> bool {
    let pid_hi = (pid >> 16) as u16;
    let pid_lo = pid as u16;
    (pid_hi ^ pid_lo ^ tid ^ sid) < 8
}

/// Descreve um alvo de caça em um jogo específico.
pub trait ShinyTarget {
    /// Nome do jogo + alvo (ex.: "Ruby / Latias").
    fn name(&self) -> &str;

    /// Endereço na RAM onde o PID do Pokémon-alvo aparece.
    fn pid_address(&self) -> u32;

    /// TID/SID do save (lido uma vez no início).
    fn trainer_ids(&self, gba: &Gba) -> (u16, u16);

    /// Executa a sequência de inputs para chegar ao encontro
    /// (soft-reset, navegação de menus, etc.).
    fn run_encounter(&self, gba: &mut Gba);
}

/// Driver principal: roda o loop de caça.
pub struct Hunter {
    pub attempts: u64,
    pub found: bool,
}

impl Hunter {
    pub fn new() -> Self {
        Self { attempts: 0, found: false }
    }

    /// Roda uma tentativa. Retorna `true` se encontrou shiny.
    pub fn try_once(&mut self, gba: &mut Gba, target: &dyn ShinyTarget) -> bool {
        self.attempts += 1;
        target.run_encounter(gba);

        let (tid, sid) = target.trainer_ids(gba);
        let pid_addr = target.pid_address();
        let pid = read_u32(gba, pid_addr);

        if is_shiny_gen3(pid, tid, sid) {
            self.found = true;
            log::info!("✨ SHINY encontrado após {} tentativas! PID={:08X}", self.attempts, pid);
            true
        } else {
            false
        }
    }
}

impl Default for Hunter {
    fn default() -> Self {
        Self::new()
    }
}

fn read_u32(gba: &Gba, addr: u32) -> u32 {
    let b0 = gba.bus.read_u8(addr) as u32;
    let b1 = gba.bus.read_u8(addr + 1) as u32;
    let b2 = gba.bus.read_u8(addr + 2) as u32;
    let b3 = gba.bus.read_u8(addr + 3) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shiny_formula_known_values() {
        // PID=0x00000000, TID=0, SID=0 → XOR=0 → shiny.
        assert!(is_shiny_gen3(0, 0, 0));
        // Caso obviamente não-shiny.
        assert!(!is_shiny_gen3(0xABCD_1234, 0x1111, 0x2222));
    }
}
