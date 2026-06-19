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
    /// Selvagem na grama: sem reset. O personagem fica na mesma moita e o Hunter
    /// cicla as direções (cada uma segurada o bastante pra virar **passo**, não só
    /// rotação) — o ciclo ►/▲/◄/▼ volta à origem, então a caça é auto-contida
    /// (não precisa saber qual tile vizinho está livre). Cada passo na grama rola
    /// o sorteio de encontro; quando um selvagem carrega no `gEnemyParty`, checa o
    /// shiny. (A fuga pra encadear tentativas vem no Marco 2.)
    WildSpin,
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
    /// Valores de `gTasks[i].func` (8 bytes antes do cursor, offset 0 da `Task`)
    /// quando a task que processa a direção está ativa — assinatura de "menu
    /// aberto". O bit 0 setado é o flag Thumb do ponteiro. É uma **lista** porque
    /// o endereço da função muda entre revisões da ROM (o game code não): cada
    /// entrada cobre uma revisão.
    pub input_funcs: &'static [u32],
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
        input_funcs: &[0x0813_425D],
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
        // Selvagem na grama (qualquer espécie). `species: 0` = não filtra: a caça
        // para no PRIMEIRO selvagem que carregar, qualquer que seja. Use deixando
        // o personagem parado EM CIMA da grama; o Hunter cicla as direções pra dar
        // passos. Pra mirar uma espécie específica, troque `species` pelo índice
        // interno Gen 3 do alvo (no Marco 2, os não-alvo são fugidos e ignorados).
        TargetDef {
            name: "Selvagem (qualquer)",
            species: 0,
            slot: Slot::Enemy,
            method: HuntMethod::WildSpin,
            cursor: StarterCursor::Center,
        },
    ],
};

// ───── Ruby/Sapphire (AXVE/AXPE) ─────
//
// Endereços tirados dos símbolos do decomp pret/pokeruby (branch `symbols`),
// conferidos nas TRÊS revisões (rev 0/1/2) de cada jogo: gPlayerParty,
// gEnemyParty, gRngValue e gTasks são IDÊNTICOS em todas — só o endereço de
// `Task_StarterChoose2` (o handler de ◄/►/A do menu do inicial) muda da rev 0
// pras rev 1/2, por isso `input_funcs` lista os dois. Diferença pro Emerald:
// em R/S as parties ficam na IWRAM (0x03xx), não na EWRAM.
//
// O cursor do inicial é `gTasks[0].data[0]` (gTasks 0x03004B20 + 8): o decomp
// mostra que a tela do inicial faz ResetTasks() e cria a task de input em
// seguida — ela cai no slot 0, como no Emerald.

/// Alvos comuns a Ruby e Sapphire (cada versão acrescenta seu mascote).
/// Índices internos Gen 3 — a tabela de espécies é a MESMA de R/S a Emerald.
const RAYQUAZA_RS: TargetDef = TargetDef {
    name: "Rayquaza",
    species: 406,
    slot: Slot::Enemy,
    method: HuntMethod::SoftResetLegendary,
    cursor: StarterCursor::Center,
};
const REGIROCK_RS: TargetDef = TargetDef {
    name: "Regirock",
    species: 401,
    ..RAYQUAZA_RS
};
const REGICE_RS: TargetDef = TargetDef {
    name: "Regice",
    species: 402,
    ..RAYQUAZA_RS
};
const REGISTEEL_RS: TargetDef = TargetDef {
    name: "Registeel",
    species: 403,
    ..RAYQUAZA_RS
};
const TREECKO_RS: TargetDef = TargetDef {
    name: "Treecko (inicial)",
    species: 277,
    slot: Slot::Player,
    method: HuntMethod::Starter,
    cursor: StarterCursor::Left,
};
const TORCHIC_RS: TargetDef = TargetDef {
    name: "Torchic (inicial)",
    species: 280,
    cursor: StarterCursor::Center,
    ..TREECKO_RS
};
const MUDKIP_RS: TargetDef = TargetDef {
    name: "Mudkip (inicial)",
    species: 283,
    cursor: StarterCursor::Right,
    ..TREECKO_RS
};

/// Menu do inicial de R/S: mesmo desenho do Emerald (seleção em
/// `gTasks[0].data[0]`, forçada só com a task de input ativa). Funcs por
/// revisão: rev 0 = 0x0810A178, rev 1/2 = 0x0810A198 (+1 do bit Thumb).
const RS_STARTER_MENU: StarterMenu = StarterMenu {
    cursor_addr: 0x0300_4B28,
    input_funcs: &[0x0810_A179, 0x0810_A199],
};

/// Pokémon Ruby (AXVE). Mascote: Groudon (Caverna Ancestral, nv. 45); Rayquaza
/// fica no Pilar Celeste (pós-game). Kyogre não existe nesta versão. O roamer
/// (Latios) fica de fora pelo mesmo motivo do Emerald: o PID dele é fixado no
/// save quando é gerado — soft-reset não re-rola.
const RUBY: GameProfile = GameProfile {
    code: "AXVE",
    name: "Pokémon Ruby",
    player_party: 0x0300_4360,
    enemy_party: 0x0300_45C0,
    rng_addr: Some(0x0300_4818),
    starter_menu: Some(RS_STARTER_MENU),
    targets: &[
        TargetDef {
            name: "Groudon",
            species: 405,
            ..RAYQUAZA_RS
        },
        RAYQUAZA_RS,
        REGIROCK_RS,
        REGICE_RS,
        REGISTEEL_RS,
        TREECKO_RS,
        TORCHIC_RS,
        MUDKIP_RS,
    ],
};

/// Pokémon Sapphire (AXPE). Igual ao Ruby, com Kyogre no lugar de Groudon
/// (e Latias como roamer, também de fora).
const SAPPHIRE: GameProfile = GameProfile {
    code: "AXPE",
    name: "Pokémon Sapphire",
    player_party: 0x0300_4360,
    enemy_party: 0x0300_45C0,
    rng_addr: Some(0x0300_4818),
    starter_menu: Some(RS_STARTER_MENU),
    targets: &[
        TargetDef {
            name: "Kyogre",
            species: 404,
            ..RAYQUAZA_RS
        },
        RAYQUAZA_RS,
        REGIROCK_RS,
        REGICE_RS,
        REGISTEEL_RS,
        TREECKO_RS,
        TORCHIC_RS,
        MUDKIP_RS,
    ],
};

/// Todos os perfis conhecidos.
pub const PROFILES: &[GameProfile] = &[EMERALD, RUBY, SAPPHIRE];

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
    fn detects_ruby_and_sapphire_by_code() {
        let r = detect("AXVE").expect("Ruby deveria ser reconhecido");
        let s = detect("AXPE").expect("Sapphire deveria ser reconhecido");
        // Mascote de cada versão presente — e só na sua versão.
        assert!(r.targets.iter().any(|t| t.name == "Groudon"));
        assert!(!r.targets.iter().any(|t| t.name == "Kyogre"));
        assert!(s.targets.iter().any(|t| t.name == "Kyogre"));
        assert!(!s.targets.iter().any(|t| t.name == "Groudon"));
        // Caça de inicial em malha fechada disponível nos dois.
        assert!(r.starter_menu.is_some() && s.starter_menu.is_some());
        assert!(r.rng_addr.is_some() && s.rng_addr.is_some());
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
