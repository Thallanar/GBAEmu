//! Banco de dados de jogos (data-driven). Cada [`GameProfile`] descreve, **em
//! dados puros**, onde um jogo guarda os Pokémon na RAM e quais alvos de caça
//! ele oferece. Adicionar suporte a um jogo novo = adicionar uma entrada em
//! [`PROFILES`], sem tocar na lógica do Hunter.
//!
//! O emulador identifica o jogo pelo **game code** do header (offset 0xAC da
//! ROM); se não reconhecer, o usuário escolhe manualmente da lista.

/// De qual "party" (lista de Pokémon) o alvo deve ser lido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Slot 0 do time do jogador (ex.: inicial recém-escolhido).
    Player,
    /// Slot 0 do time inimigo (ex.: lendário/selvagem na batalha).
    Enemy,
}

/// Como a caça é conduzida (afeta o roteiro de inputs do loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntMethod {
    /// Soft-reset na frente de um lendário estático: dá A/Start até a batalha.
    SoftResetLegendary,
    /// Inicial no laboratório (navegação de menu — implementado depois).
    Starter,
}

/// Um alvo de caça concreto dentro de um jogo.
#[derive(Debug, Clone, Copy)]
pub struct TargetDef {
    /// Nome exibido (ex.: "Rayquaza").
    pub name: &'static str,
    /// Índice **interno** da espécie (como aparece na RAM do Gen 3), usado pra
    /// confirmar que o encontro certo carregou. `0` = não verificar espécie
    /// (confia só no checksum + estado de batalha).
    pub species: u16,
    /// De qual party ler o PID do alvo.
    pub slot: Slot,
    /// Método de caça.
    pub method: HuntMethod,
}

/// Perfil de um jogo: endereços de RAM + lista de alvos.
#[derive(Debug, Clone, Copy)]
pub struct GameProfile {
    /// Game code do header (offset 0xAC), ex.: "BPEE".
    pub code: &'static str,
    /// Nome amigável.
    pub name: &'static str,
    /// Endereço de `gPlayerParty` (slot 0). O TID/SID do jogador é lido do
    /// campo OT ID (offset +0x04) do líder deste time.
    pub player_party: u32,
    /// Endereço de `gEnemyParty` (slot 0).
    pub enemy_party: u32,
    /// Alvos de caça suportados neste jogo.
    pub targets: &'static [TargetDef],
}

impl GameProfile {
    /// Endereço-base do Pokémon-alvo, conforme o slot do alvo.
    pub fn target_base(&self, target: &TargetDef) -> u32 {
        match target.slot {
            Slot::Player => self.player_party,
            Slot::Enemy => self.enemy_party,
        }
    }
}

// ─────────────────────────── Banco de perfis ────────────────────────────────
//
// ATENÇÃO: os endereços abaixo são por versão do jogo e foram tirados dos mapas
// de RAM da comunidade (datacrystal / símbolos do decomp pokeemerald). Devem ser
// CONFIRMADOS contra a ROM real do usuário na primeira caça — a infra permite
// corrigir um número aqui sem mexer em mais nada.

/// Pokémon Emerald (BPEE).
const EMERALD: GameProfile = GameProfile {
    code: "BPEE",
    name: "Pokémon Emerald",
    player_party: 0x0202_44EC,
    enemy_party: 0x0202_4744,
    targets: &[
        // species=0 por ora (não verifica espécie): o índice interno do Gen 3
        // difere do dex nacional pros Pokémon de Hoenn; confirmamos na ROM e
        // preenchemos depois.
        TargetDef {
            name: "Rayquaza",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
        },
        TargetDef {
            name: "Groudon",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
        },
        TargetDef {
            name: "Kyogre",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
        },
        // Inicial Torchic: caminho reto (A abre a bag → A escolhe o do centro →
        // A confirma), por isso casa com o mashing de A do loop. O alvo é o slot
        // 0 do time do jogador. species=0: índice interno a confirmar (~280);
        // pular a checagem evita travar o loop se o índice estiver errado.
        // Treecko (esquerda) e Mudkip (direita) precisam de um passo direcional
        // — entram quando tivermos roteiro de input com direção.
        TargetDef {
            name: "Torchic (inicial)",
            species: 0,
            slot: Slot::Player,
            method: HuntMethod::Starter,
        },
    ],
};

/// Todos os perfis conhecidos.
pub const PROFILES: &[GameProfile] = &[EMERALD];

/// Procura um perfil pelo game code do header da ROM.
pub fn detect(game_code: &str) -> Option<&'static GameProfile> {
    PROFILES.iter().find(|p| p.code == game_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_emerald_by_code() {
        let p = detect("BPEE").expect("Emerald deveria ser reconhecido");
        assert_eq!(p.name, "Pokémon Emerald");
        assert!(!p.targets.is_empty());
    }

    #[test]
    fn unknown_code_returns_none() {
        assert!(detect("ZZZZ").is_none());
    }

    #[test]
    fn target_base_picks_right_party() {
        let p = detect("BPEE").unwrap();
        let enemy_target = TargetDef {
            name: "x",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
        };
        let player_target = TargetDef {
            slot: Slot::Player,
            ..enemy_target
        };
        assert_eq!(p.target_base(&enemy_target), p.enemy_party);
        assert_eq!(p.target_base(&player_target), p.player_party);
    }
}
