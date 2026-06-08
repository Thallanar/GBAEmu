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
    /// Inicial no laboratório: amassa A pra chegar na bag e (se preciso) segura
    /// uma direção pra mover o cursor até o inicial certo.
    Starter,
}

/// No menu de seleção do inicial (3 Poké Balls em linha), de que lado fica o
/// alvo. Mapeia direto pro valor do byte da seleção (`gTasks[i].data[0]`): a
/// caça lê esse byte e o **força** pro valor do alvo enquanto o menu está aberto
/// (ver [`StarterMenu`]) — controle em malha fechada, sem depender de timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StarterCursor {
    /// Treecko — Poké Ball da esquerda (`data[0] == 0`).
    Left,
    /// Torchic — Poké Ball do centro (`data[0] == 1`, default).
    Center,
    /// Mudkip — Poké Ball da direita (`data[0] == 2`).
    Right,
}

impl StarterCursor {
    /// Valor que o byte da seleção (`gTasks[i].data[0]`) assume nesta posição.
    pub fn value(self) -> u8 {
        match self {
            StarterCursor::Left => 0,
            StarterCursor::Center => 1,
            StarterCursor::Right => 2,
        }
    }
}

/// Endereços do menu de seleção do inicial (Gen 3), pra caça em **malha fechada**.
/// Em pokeemerald a seleção fica em `gTasks[i].data[0]` (0=esq, 1=centro, 2=dir)
/// e a task que processa ◄/► tem um `func` conhecido. O Hunter só força a
/// seleção quando esse `func` está ativo (menu de fato aberto), evitando escrever
/// em RAM compartilhada por outras telas. Endereços por versão — descobertos com
/// o detector de cursor do desktop.
#[derive(Debug, Clone, Copy)]
pub struct StarterMenu {
    /// Endereço do byte da seleção (`gTasks[i].data[0]`).
    pub cursor_addr: u32,
    /// Valor de `gTasks[i].func` (8 bytes antes do cursor, offset 0 da `Task`)
    /// quando a task que processa a direção está ativa — assinatura de "menu
    /// aberto". O bit 0 setado é o flag Thumb do ponteiro.
    pub input_func: u32,
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
    /// Posição do alvo no menu do inicial. Só vale para [`HuntMethod::Starter`];
    /// alvos de outros métodos usam [`StarterCursor::Center`] (ignorado).
    pub cursor: StarterCursor,
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
    /// Endereço de `gRngValue` (a seed do `Random()` do jogo, na IWRAM). O Hunter
    /// injeta aqui uma seed aleatória do host a cada tentativa — é o que faz o PID
    /// variar. `None` = sem injeção (caça determinística; ainda não mapeado).
    pub rng_addr: Option<u32>,
    /// Endereços do menu do inicial, pra forçar a seleção em malha fechada.
    /// `None` = jogo sem método Starter mapeado (a direção é ignorada).
    pub starter_menu: Option<StarterMenu>,
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
    // gRngValue do Emerald (confirmado empiricamente: injetar aqui no frame ~200
    // dá 20/20 PIDs distintos; sem injeção, 1 único PID).
    rng_addr: Some(0x0300_5D80),
    // Menu do inicial confirmado na ROM real com o detector de cursor: a seleção
    // é `gTasks[0].data[0]` em 0x03005E08 e a task de input tem func 0x0813425D.
    starter_menu: Some(StarterMenu {
        cursor_addr: 0x0300_5E08,
        input_func: 0x0813_425D,
    }),
    targets: &[
        // Lendários estáticos (soft-reset na frente). Índices INTERNOS do Gen 3
        // (≠ dex nacional): a cauda de Hoenn é REORDENADA — não é um offset fixo.
        // Valores tirados do `SPECIES_*` do pokeemerald (ordem interna da ROM) e
        // confirmados pelo sprite no próprio app: Regirock 401, Regice 402,
        // Registeel 403, Kyogre 404, Groudon 405, Rayquaza 406, Latias 407,
        // Latios 408 (Jirachi 409, Deoxys 410). O trio do clima vem ANTES dos
        // Lati internamente.
        TargetDef {
            name: "Rayquaza",
            species: 406,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Groudon",
            species: 405,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Kyogre",
            species: 404,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Regirock",
            species: 401,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Regice",
            species: 402,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Registeel",
            species: 403,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetLegendary,
            cursor: StarterCursor::Center,
        },
        // Iniciais de Hoenn: o alvo é o slot 0 do time do jogador. O cursor do
        // menu abre no centro (Torchic); os laterais saem segurando a direção.
        // Índices internos Gen 3 (confirmados na RAM do Emerald): Treecko=277,
        // Torchic=280, Mudkip=283.
        TargetDef {
            name: "Treecko (inicial)",
            species: 277,
            slot: Slot::Player,
            method: HuntMethod::Starter,
            cursor: StarterCursor::Left,
        },
        // Torchic: caminho reto (A abre a bag → A escolhe o do centro → A
        // confirma), casa com o A-mash sem direção.
        TargetDef {
            name: "Torchic (inicial)",
            species: 280,
            slot: Slot::Player,
            method: HuntMethod::Starter,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Mudkip (inicial)",
            species: 283,
            slot: Slot::Player,
            method: HuntMethod::Starter,
            cursor: StarterCursor::Right,
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
            cursor: StarterCursor::Center,
        };
        let player_target = TargetDef {
            slot: Slot::Player,
            ..enemy_target
        };
        assert_eq!(p.target_base(&enemy_target), p.enemy_party);
        assert_eq!(p.target_base(&player_target), p.player_party);
    }
}
