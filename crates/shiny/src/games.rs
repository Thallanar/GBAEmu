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
    /// Soft-reset num lendário que **não espera interação**: ele vem pra cima do
    /// jogador quando você chega perto (Groudon na Terra Cave / Kyogre na Marine
    /// Cave, do evento de clima anormal pós-Liga). Como a batalha dispara por
    /// proximidade, o A repetido sozinho nunca chega ao encontro — o Hunter
    /// precisa **andar** um tile na direção do lendário. Igual ao
    /// [`HuntMethod::SoftResetLegendary`] no resto (soft-reset re-rola o PID,
    /// alvo no slot 0 do time inimigo).
    SoftResetApproach(Approach),
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
    /// Presente estático recebido por script (ex.: Beldum na casa do Steven pós-E4,
    /// iniciais de outras regiões pós-National Dex). O jogo roda `CreateMon(Random())`
    /// no momento em que você pega, então **soft-reset re-rola o PID** — mesma lógica
    /// do lendário/inicial. A diferença é a leitura: o presente entra no **primeiro
    /// slot livre de `gPlayerParty`** (não no slot 0, que tem o líder do time), então
    /// a caça **varre os 6 slots** procurando `species` em vez de ler um slot fixo.
    /// O roteiro é o A-mash (sem menu de cursor): mashar A pega a Poké Ball e avança
    /// os diálogos. **Exige um slot de time livre** — com o time cheio o presente vai
    /// pro PC (outro endereço/formato), e a varredura não o encontra.
    StaticGift,
    /// Prêmio do balcão do cassino (Celadon, FR/LG): é um presente por script
    /// (mesmo `CreateMon(Random())` na hora da troca), então funciona como o
    /// [`HuntMethod::StaticGift`] — soft-reset, A-mash, varredura do time. A
    /// diferença é a lista de prêmios: a caça força o cursor dela em `index`
    /// (ver [`PrizeMenu`]) e confere se as moedas cobrem `price` (ver
    /// [`CoinStore`]); sem moedas o jogo recusa a troca e a caça pararia em loop.
    GameCornerPrize { index: u8, price: u16 },
}

impl HuntMethod {
    /// Presente por script (o Pokémon chega no time do jogador): o
    /// [`HuntMethod::StaticGift`] e o prêmio do cassino.
    pub fn is_gift(self) -> bool {
        matches!(
            self,
            HuntMethod::StaticGift | HuntMethod::GameCornerPrize { .. }
        )
    }
}

/// Lista de prêmios do cassino (um `multichoice` do jogo, na struct de menu
/// genérica). O mesmo struct serve o SIM/NÃO da confirmação, então o Hunter só
/// força o cursor quando o "cursor máximo" bate com o da lista de prêmios.
/// Achado empiricamente no FireRed (rev 1) com o save do usuário no balcão.
#[derive(Debug, Clone, Copy)]
pub struct PrizeMenu {
    /// Byte do cursor (`sMenu.cursorPos`): 0 = primeiro prêmio.
    pub cursor_addr: u32,
    /// Byte do cursor máximo (`sMenu.maxCursorPos`).
    pub max_addr: u32,
    /// Valor do cursor máximo na lista de prêmios (5 prêmios + "não, obrigado").
    pub max: u8,
    /// `(endereço, valor)` que indica "texto parado esperando o A" (estado da
    /// impressora de texto). Com o A martelado, um toque no meio do "Qual
    /// prêmio?" termina o texto e o menu nasce já escolhendo o 1º prêmio, antes
    /// de dar pra forçar o cursor — então no cassino o A só é tocado com texto
    /// esperando, com a lista aberta ou depois de uma pausa longa.
    pub text_wait: (u32, u8),
}

/// Onde o jogo guarda as moedas do cassino. No FR/LG elas ficam no SaveBlock1,
/// cifradas com XOR pela chave do SaveBlock2; os dois blocos mudam de lugar na
/// memória, então a leitura vai pelos ponteiros (`gSaveBlock1Ptr`/`2Ptr`).
#[derive(Debug, Clone, Copy)]
pub struct CoinStore {
    /// Endereço do ponteiro do SaveBlock1.
    pub block1_ptr: u32,
    /// Offset das moedas (u16 cifrado) dentro do SaveBlock1.
    pub coins_offset: u32,
    /// Endereço do ponteiro do SaveBlock2.
    pub block2_ptr: u32,
    /// Offset da chave (u16 baixo) dentro do SaveBlock2.
    pub key_offset: u32,
}

/// Para onde o Hunter deve dar o passo de aproximação no
/// [`HuntMethod::SoftResetApproach`] — a direção em que o lendário está,
/// relativa ao tile onde o jogo foi salvo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approach {
    Left,
    Right,
    Up,
    Down,
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

/// Grupo do alvo na tela de escolha (derivado do método).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Lendários e estáticos (soft-reset na frente, ou que vêm até você).
    Legendary,
    /// Iniciais: menu do laboratório (Hoenn) ou Poké Ball na mesa (Kanto).
    Starter,
    /// Presentes por script (Beldum, Lapras, fósseis, iniciais de Johto…).
    Gift,
    /// Selvagem na grama.
    Wild,
    /// Prêmios do balcão do cassino (FR/LG).
    GameCorner,
}

impl TargetDef {
    /// Grupo do alvo. Os iniciais de Kanto são presentes por script no código
    /// ([`HuntMethod::StaticGift`]), mas pro jogador são iniciais.
    pub fn kind(&self) -> TargetKind {
        match self.method {
            HuntMethod::SoftResetLegendary | HuntMethod::SoftResetApproach(_) => {
                TargetKind::Legendary
            }
            HuntMethod::Starter => TargetKind::Starter,
            HuntMethod::StaticGift if self.name.contains("(inicial)") => TargetKind::Starter,
            HuntMethod::StaticGift => TargetKind::Gift,
            HuntMethod::WildSpin => TargetKind::Wild,
            HuntMethod::GameCornerPrize { .. } => TargetKind::GameCorner,
        }
    }

    /// Nome pra exibir sem o sufixo de grupo ("Beldum (presente)" → "Beldum"):
    /// o grupo já aparece no título da seção. O selvagem vira "Qualquer
    /// selvagem" no idioma pedido.
    pub fn display_name(&self, lang: crate::lang::Lang) -> &'static str {
        if self.kind() == TargetKind::Wild && self.species == 0 {
            return lang.pick(["Qualquer selvagem", "Any wild Pokémon", "Cualquier salvaje"]);
        }
        self.name.split(" (").next().unwrap_or(self.name)
    }
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
    /// Flag "em batalha" (ex.: `gMain.inBattle`): `(endereço, máscara do bit)`. A
    /// caça de selvagem ([`HuntMethod::WildSpin`]) usa pra saber quando a batalha
    /// acabou (o `gEnemyParty` fica sujo após a fuga, então não serve). `None` =
    /// não mapeado ⇒ a caça com fuga não funciona nesse jogo.
    pub battle_flag: Option<(u32, u8)>,
    /// Cursor do menu de ação da batalha (`gActionSelectionCursor[0]`): 0 LUTAR,
    /// 1 BAG, 2 POKÉMON, 3 CORRER. A fuga do selvagem só confirma com A quando
    /// ele está em CORRER (malha fechada) — às cegas, um ▼ perdido levava o ►
    /// pra BAG e o A abria a bag. `None` = não mapeado ⇒ fuga às cegas.
    pub action_cursor: Option<u32>,
    /// Lista de prêmios do cassino. `None` = sem prêmios mapeados.
    pub prize_menu: Option<PrizeMenu>,
    /// Moedas do cassino. `None` = não mapeado (a caça não confere as moedas).
    pub coins: Option<CoinStore>,
    /// Flag "retrospectiva rodando" (o "Previously on your quest…" do FR/LG ao
    /// carregar o save): vale 1 enquanto ela passa. A caça toca B pra pulá-la
    /// — sem isso cada tentativa perde uns 200 frames. `None` = sem retrospectiva
    /// (R/S/E) ou não mapeado.
    pub quest_log: Option<u32>,
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
    // gMain.inBattle (gMain 0x030022C0 + 0x439): 0x00 no overworld, 0x02 na
    // batalha. Confirmado na ROM real com o localizador de flag de batalha.
    battle_flag: Some((0x0300_26F9, 0x02)),
    // Achado empiricamente na ROM real (► ▼ ◄ ▲ no menu ⇒ 0,1,3,2,0).
    action_cursor: Some(0x0202_44AC),
    prize_menu: None,
    coins: None,
    quest_log: None,
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
        // Groudon/Kyogre da Emerald ficam nas cavernas do evento de clima anormal
        // (Terra Cave / Marine Cave) e disparam a batalha por proximidade, não por
        // interação — daí o `SoftResetApproach`. Direções confirmadas em jogo: com
        // o save feito no tile em frente, o Groudon fica um passo à DIREITA e o
        // Kyogre um passo à ESQUERDA.
        TargetDef {
            name: "Groudon",
            species: 405,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetApproach(Approach::Right),
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Kyogre",
            species: 404,
            slot: Slot::Enemy,
            method: HuntMethod::SoftResetApproach(Approach::Left),
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
        // Presente estático: Beldum na casa do Steven (Mossdeep), liberado após a
        // Elite dos Quatro. Soft-reset re-rola o PID. Índice INTERNO Gen 3: pela
        // ordem do pokeemerald (Bagon 395 / Shelgon 396 / Salamence 397 → Beldum
        // 398 / Metang 399 / Metagross 400 → Regirock 401). 398 a CONFIRMAR contra
        // a ROM real pelo sprite no app — como todo índice deste arquivo. O alvo é
        // lido do time (slot livre): deixe uma vaga no time, senão ele vai pro PC.
        TargetDef {
            name: "Beldum (presente)",
            species: 398,
            slot: Slot::Player,
            method: HuntMethod::StaticGift,
            cursor: StarterCursor::Center,
        },
        // Iniciais de Johto: presente do Birch (laboratório de Littleroot) após
        // completar a Dex de Hoenn / pegar a Nacional. NÃO é um menu de escolha —
        // são TRÊS Poké Balls na mesa, você anda até a que quer e o diálogo é só
        // "YES" (A-mash resolve, igual ao Beldum). Salve EM FRENTE à Ball do alvo:
        // o soft-reset devolve o personagem ao mesmo ponto, então cada reset
        // re-pega a MESMA espécie com PID re-rolado. Índices INTERNOS Gen 3: tudo
        // ≤251 NÃO é reordenado (a tabela só embaralha a cauda de Hoenn 252+), logo
        // o índice interno = nº da Dex nacional. Chikorita=152, Cyndaquil=155,
        // Totodile=158. Como todo índice deste arquivo, confirme pelo sprite no app.
        // Exige vaga no time (senão o presente vai pro PC e a varredura não acha).
        TargetDef {
            name: "Chikorita (presente)",
            species: 152,
            slot: Slot::Player,
            method: HuntMethod::StaticGift,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Cyndaquil (presente)",
            species: 155,
            slot: Slot::Player,
            method: HuntMethod::StaticGift,
            cursor: StarterCursor::Center,
        },
        TargetDef {
            name: "Totodile (presente)",
            species: 158,
            slot: Slot::Player,
            method: HuntMethod::StaticGift,
            cursor: StarterCursor::Center,
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

/// `gMain.inBattle` do R/S: `gMain` fica em 0x03001774 e o `struct Main` tem o
/// mesmo layout do Emerald (`inBattle` = bit 1 do byte em +0x439). Achado
/// empiricamente no Ruby (o `vblankCounter1` em +0x20 soma 1 por frame e o bit
/// liga só em batalha) e validado com 40 fugas seguidas do Hunter.
const RS_BATTLE_FLAG: (u32, u8) = (0x0300_1BAD, 0x02);

/// `gActionSelectionCursor` do R/S — achado empiricamente no Ruby (► ▼ ◄ ▲ no
/// menu ⇒ 0,1,3,2,0, candidato único).
const RS_ACTION_CURSOR: u32 = 0x0202_4E60;

/// Selvagem na grama (qualquer espécie) — mesmo WildSpin do Emerald/FRLG.
const WILD_ANY_RS: TargetDef = TargetDef {
    name: "Selvagem (qualquer)",
    species: 0,
    slot: Slot::Enemy,
    method: HuntMethod::WildSpin,
    cursor: StarterCursor::Center,
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
    battle_flag: Some(RS_BATTLE_FLAG),
    action_cursor: Some(RS_ACTION_CURSOR),
    prize_menu: None,
    coins: None,
    quest_log: None,
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
        WILD_ANY_RS,
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
    battle_flag: Some(RS_BATTLE_FLAG),
    action_cursor: Some(RS_ACTION_CURSOR),
    prize_menu: None,
    coins: None,
    quest_log: None,
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
        WILD_ANY_RS,
    ],
};

// ───── FireRed/LeafGreen (BPRE/BPGE) ─────
//
// Endereços dos símbolos do decomp pret/pokefirered (branch `symbols`),
// conferidos nas QUATRO builds (FR/LG × rev 0/1): gPlayerParty, gEnemyParty,
// gRngValue e gMain são IDÊNTICOS em todas. `gRngValue` (0x03005000) também foi
// confirmado empiricamente com o `rng_scan` nas ROMs FR e LG rev 1 (1199/1200
// avanços de LCG). Como no Emerald, as parties ficam na EWRAM.
//
// Diferenças de roteiro pro Hoenn:
// - **Iniciais**: não há menu de cursor. São três Poké Balls na mesa do Oak; você
//   anda até a que quer e o diálogo é só "YES" — é o mesmo caso dos iniciais de
//   Johto no Emerald, então usa [`HuntMethod::StaticGift`] (salve EM FRENTE à Ball
//   do alvo; o time está vazio, o inicial cai no slot 0 e a varredura acha).
// - Lendários estáticos (pássaros, Mewtwo, Snorlax) disparam a batalha com A —
//   [`HuntMethod::SoftResetLegendary`]. No Snorlax o A também responde "YES" pra
//   usar a Poké Flute. Os cães lendários são roamers (PID fixado no save quando
//   gerados — soft-reset não re-rola), ficam de fora como os Lati.
//
// Índices INTERNOS Gen 3: todos os alvos de Kanto são ≤251, faixa em que o
// índice interno = nº da Dex nacional (só a cauda de Hoenn é reordenada).

/// Alvos comuns a FireRed e LeafGreen, mais os de uma versão só (`$extra`, ex.:
/// os prêmios do cassino, que mudam entre as versões).
macro_rules! kanto_targets {
    ($($extra:expr),* $(,)?) => {
        &[
    // Iniciais (presente do Oak, Ball na mesa).
    TargetDef {
        name: "Bulbasaur (inicial)",
        species: 1,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Charmander (inicial)",
        species: 4,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Squirtle (inicial)",
        species: 7,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    // Lendários / estáticos (soft-reset na frente, A dispara a batalha).
    TargetDef {
        name: "Mewtwo",
        species: 150,
        slot: Slot::Enemy,
        method: HuntMethod::SoftResetLegendary,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Articuno",
        species: 144,
        slot: Slot::Enemy,
        method: HuntMethod::SoftResetLegendary,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Zapdos",
        species: 145,
        slot: Slot::Enemy,
        method: HuntMethod::SoftResetLegendary,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Moltres",
        species: 146,
        slot: Slot::Enemy,
        method: HuntMethod::SoftResetLegendary,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Snorlax",
        species: 143,
        slot: Slot::Enemy,
        method: HuntMethod::SoftResetLegendary,
        cursor: StarterCursor::Center,
    },
    // Presentes por script (A-mash; exigem vaga no time).
    TargetDef {
        name: "Lapras (presente)",
        species: 131,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Eevee (presente)",
        species: 133,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Hitmonlee (presente)",
        species: 106,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Hitmonchan (presente)",
        species: 107,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    // Fósseis revividos no laboratório de Cinnabar: salve em frente ao cientista
    // depois de entregar o fóssil (o PID nasce quando ele te entrega o Pokémon).
    TargetDef {
        name: "Omanyte (fóssil)",
        species: 138,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Kabuto (fóssil)",
        species: 140,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    TargetDef {
        name: "Aerodactyl (fóssil)",
        species: 142,
        slot: Slot::Player,
        method: HuntMethod::StaticGift,
        cursor: StarterCursor::Center,
    },
    // Selvagem na grama (qualquer espécie) — mesmo WildSpin do Emerald.
    TargetDef {
        name: "Selvagem (qualquer)",
        species: 0,
        slot: Slot::Enemy,
        method: HuntMethod::WildSpin,
        cursor: StarterCursor::Center,
    },
            $($extra,)*
        ]
    };
}

/// Prêmio do cassino de Celadon: presente na posição `index` da lista, por
/// `price` moedas.
const fn prize(name: &'static str, species: u16, index: u8, price: u16) -> TargetDef {
    TargetDef {
        name,
        species,
        slot: Slot::Player,
        method: HuntMethod::GameCornerPrize { index, price },
        cursor: StarterCursor::Center,
    }
}

/// FireRed: Kanto + prêmios do cassino (ordem e preços lidos da tela do jogo,
/// FR rev 1: Abra 180, Clefairy 500, Dratini 2800, Scyther 5500, Porygon 9999).
const FIRERED_TARGETS: &[TargetDef] = kanto_targets![
    prize("Abra (cassino)", 63, 0, 180),
    prize("Clefairy (cassino)", 35, 1, 500),
    prize("Dratini (cassino)", 147, 2, 2800),
    prize("Scyther (cassino)", 123, 3, 5500),
    prize("Porygon (cassino)", 137, 4, 9999),
];

/// LeafGreen: por ora sem os prêmios do cassino (a lista dele difere — Pinsir no
/// lugar do Scyther — e o menu ainda não foi conferido num save real).
const LEAFGREEN_TARGETS: &[TargetDef] = kanto_targets![];

/// `gMain` do FRLG fica em 0x030030F0; o `struct Main` tem o MESMO layout do
/// Emerald (`inBattle` = bit 1 do byte em +0x439).
const FRLG_BATTLE_FLAG: (u32, u8) = (0x0300_3529, 0x02);

/// `gActionSelectionCursor` do FRLG — achado empiricamente no FireRed (rev 1);
/// o LeafGreen compartilha os símbolos de EWRAM.
const FRLG_ACTION_CURSOR: u32 = 0x0202_3FF8;

/// Pokémon FireRed (BPRE).
const FIRERED: GameProfile = GameProfile {
    code: "BPRE",
    name: "Pokémon FireRed",
    player_party: 0x0202_4284,
    enemy_party: 0x0202_402C,
    rng_addr: Some(0x0300_5000),
    // Sem menu de cursor: os iniciais são StaticGift (ver acima).
    starter_menu: None,
    battle_flag: Some(FRLG_BATTLE_FLAG),
    action_cursor: Some(FRLG_ACTION_CURSOR),
    // Lista de prêmios do cassino e moedas: achados no FireRed rev 1 com o save
    // do usuário no balcão (cursor 0..4 ao descer; cursor máximo 5 na lista e
    // 1 no SIM/NÃO; moedas = SaveBlock1+0x294 XOR chave em SaveBlock2+0xAF8).
    prize_menu: Some(PrizeMenu {
        cursor_addr: 0x0203_ADE6,
        max_addr: 0x0203_ADE8,
        max: 5,
        // Achado no mesmo save: 3 com o texto esperando o A; 0 escrevendo ou
        // sem texto.
        text_wait: (0x0300_222B, 3),
    }),
    coins: Some(CoinStore {
        block1_ptr: 0x0300_5008,
        coins_offset: 0x294,
        block2_ptr: 0x0300_500C,
        key_offset: 0xAF8,
    }),
    // Achado no FireRed rev 1: 1 do início ao fim da retrospectiva; um B uns
    // frames depois de ela começar a encerra (~140 frames a menos por tentativa).
    quest_log: Some(0x0203_1DD8),
    targets: FIRERED_TARGETS,
};

/// Pokémon LeafGreen (BPGE). Mesmos endereços do FireRed; alvos sem o cassino
/// por enquanto (ver [`LEAFGREEN_TARGETS`]).
const LEAFGREEN: GameProfile = GameProfile {
    code: "BPGE",
    name: "Pokémon LeafGreen",
    prize_menu: None,
    // A pular quando conferido num save do LeafGreen.
    quest_log: None,
    targets: LEAFGREEN_TARGETS,
    ..FIRERED
};

/// Todos os perfis conhecidos.
pub const PROFILES: &[GameProfile] = &[EMERALD, RUBY, SAPPHIRE, FIRERED, LEAFGREEN];

/// Procura um perfil pelo game code do header da ROM.
pub fn detect(game_code: &str) -> Option<&'static GameProfile> {
    PROFILES.iter().find(|p| p.code == game_code)
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_kinds_and_display_names() {
        use crate::lang::Lang;
        let find = |code: &str, name: &str| {
            *detect(code)
                .unwrap()
                .targets
                .iter()
                .find(|t| t.name == name)
                .unwrap()
        };
        let beldum = find("BPEE", "Beldum (presente)");
        assert_eq!(beldum.kind(), TargetKind::Gift);
        assert_eq!(beldum.display_name(Lang::En), "Beldum");
        assert_eq!(find("BPEE", "Mudkip (inicial)").kind(), TargetKind::Starter);
        // Kanto: presente por script no código, inicial pro jogador.
        assert_eq!(
            find("BPRE", "Bulbasaur (inicial)").kind(),
            TargetKind::Starter
        );
        assert_eq!(find("BPRE", "Kabuto (fóssil)").kind(), TargetKind::Gift);
        assert_eq!(find("BPEE", "Groudon").kind(), TargetKind::Legendary);
        let wild = find("BPEE", "Selvagem (qualquer)");
        assert_eq!(wild.kind(), TargetKind::Wild);
        assert_eq!(wild.display_name(Lang::Es), "Cualquier salvaje");
        // Todo alvo cai num grupo e tem nome não vazio nos 3 idiomas.
        for p in PROFILES {
            for t in p.targets {
                for lang in [Lang::Pt, Lang::En, Lang::Es] {
                    assert!(!t.display_name(lang).is_empty(), "{}", t.name);
                }
            }
        }
    }

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
    fn emerald_has_johto_starter_gifts() {
        let p = detect("BPEE").unwrap();
        // Os 3 iniciais de Johto (presente do Birch pós-Dex): StaticGift, lidos do
        // time, índice interno = nº da Dex nacional (≤251 não é reordenado).
        for (name, species) in [
            ("Chikorita (presente)", 152u16),
            ("Cyndaquil (presente)", 155),
            ("Totodile (presente)", 158),
        ] {
            let t = p
                .targets
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name} deveria existir no Emerald"));
            assert_eq!(t.species, species, "{name}: índice interno errado");
            assert_eq!(
                t.method,
                HuntMethod::StaticGift,
                "{name}: deveria ser presente"
            );
            assert_eq!(
                t.slot,
                Slot::Player,
                "{name}: presente cai no time do jogador"
            );
        }
    }

    #[test]
    fn firered_has_game_corner_prizes_in_list_order() {
        let fr = detect("BPRE").unwrap();
        let prizes: Vec<(&str, u16, u8, u16)> = fr
            .targets
            .iter()
            .filter_map(|t| match t.method {
                HuntMethod::GameCornerPrize { index, price } => {
                    assert_eq!(t.kind(), TargetKind::GameCorner);
                    Some((t.name, t.species, index, price))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            prizes,
            vec![
                ("Abra (cassino)", 63, 0, 180),
                ("Clefairy (cassino)", 35, 1, 500),
                ("Dratini (cassino)", 147, 2, 2800),
                ("Scyther (cassino)", 123, 3, 5500),
                ("Porygon (cassino)", 137, 4, 9999),
            ]
        );
        assert!(fr.prize_menu.is_some() && fr.coins.is_some());
        // O LeafGreen ainda não tem os prêmios (lista diferente, sem save conferido).
        let lg = detect("BPGE").unwrap();
        assert!(lg.prize_menu.is_none());
        assert!(!lg
            .targets
            .iter()
            .any(|t| t.kind() == TargetKind::GameCorner));
        assert_eq!(lg.targets.len() + 5, fr.targets.len());
    }

    #[test]
    fn detects_firered_and_leafgreen_by_code() {
        let fr = detect("BPRE").expect("FireRed deveria ser reconhecido");
        let lg = detect("BPGE").expect("LeafGreen deveria ser reconhecido");
        for p in [fr, lg] {
            assert!(p.rng_addr.is_some() && p.battle_flag.is_some());
            // Iniciais de Kanto: presentes na mesa (StaticGift), sem menu de cursor.
            assert!(p.starter_menu.is_none());
            for (name, species) in [
                ("Bulbasaur (inicial)", 1u16),
                ("Charmander (inicial)", 4),
                ("Squirtle (inicial)", 7),
            ] {
                let t = p
                    .targets
                    .iter()
                    .find(|t| t.name == name)
                    .unwrap_or_else(|| panic!("{name} deveria existir em {}", p.name));
                assert_eq!(t.species, species);
                assert_eq!(t.method, HuntMethod::StaticGift);
            }
        }
        assert_eq!(fr.player_party, lg.player_party);
        assert_ne!(fr.name, lg.name);
    }

    /// Caça de selvagem exige o flag de batalha (pra saber quando a fuga acabou)
    /// e o cursor do menu (pra fuga não abrir a bag) — em todo jogo que a oferece.
    #[test]
    fn every_wild_hunt_has_battle_flag_and_action_cursor() {
        for p in PROFILES {
            if p.targets.iter().any(|t| t.method == HuntMethod::WildSpin) {
                assert!(p.battle_flag.is_some(), "{}: sem battle_flag", p.name);
                assert!(p.action_cursor.is_some(), "{}: sem action_cursor", p.name);
            }
        }
        // Os cinco jogos têm caça de selvagem.
        assert!(PROFILES
            .iter()
            .all(|p| p.targets.iter().any(|t| t.method == HuntMethod::WildSpin)));
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
