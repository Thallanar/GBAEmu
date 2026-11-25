//! Ajuda "Como usar" do Shiny Hunter: texto explicativo por alvo + vídeo de
//! exemplo (quando existe). Fica aqui, no crate compartilhado, pra que desktop e
//! Android mostrem **o mesmo** conteúdo — cada frontend só decide como desenhar.
//!
//! Os vídeos vivem em `assets/hunter-help/` na raiz do repositório, com o mesmo
//! nome-base ([`HelpVideo::stem`]) nos dois formatos:
//! - `desktop/<stem>.h264` — Annex B, embutido e decodificado pelo desktop;
//! - `android/hunter_help/<stem>.mp4` — empacotado nos assets do APK.

use crate::games::{Approach, GameProfile, HuntMethod, TargetDef};
use crate::lang::Lang;

/// Vídeos de exemplo disponíveis. Métodos sem vídeo ainda (presentes, fósseis,
/// lendários, Groudon/Kyogre, pesca) mostram só o texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpVideo {
    /// Iniciais de Hoenn (Ruby/Sapphire/Emerald): bolsa do Birch na Rota 101.
    StartersRse,
    /// Iniciais de Kanto (FireRed/LeafGreen): Poké Balls na mesa do Oak.
    StartersFrlg,
    /// Selvagem na grama.
    Wild,
}

impl HelpVideo {
    /// Nome-base do arquivo em `assets/hunter-help/{desktop,android/hunter_help}/`
    /// (sem extensão).
    pub fn stem(self) -> &'static str {
        match self {
            HelpVideo::StartersRse => "starters_rse",
            HelpVideo::StartersFrlg => "starters_frlg",
            HelpVideo::Wild => "wild",
        }
    }
}

/// Ajuda de um alvo: título do modo, passo a passo e o vídeo (se houver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Help {
    /// Nome do modo de caça (ex.: "Iniciais de Kanto").
    pub title: &'static str,
    /// Texto explicativo, em parágrafos separados por linha em branco.
    pub body: String,
    /// Vídeo de exemplo; `None` = ainda não gravado.
    pub video: Option<HelpVideo>,
}

/// Lembrete comum a todo método que reinicia o jogo.
pub const SAVE_NOTE: [&str; 3] = [
    "Importante: o Hunter usa o save DO JOGO (Start → SAVE), não os \
estados salvos do emulador. A cada tentativa ele reinicia o console e continua desse save — \
por isso o lugar onde você salva é o ponto de partida de toda tentativa.",
    "Important: the Hunter uses the GAME's save (Start → SAVE), not the emulator's save \
states. On every attempt it resets the console and continues from that save — so where you \
save is the starting point of every attempt.",
    "Importante: el Hunter usa la partida DEL JUEGO (Start → GUARDAR), no los estados \
guardados del emulador. En cada intento reinicia la consola y sigue desde esa partida — por \
eso el lugar donde guardas es el punto de partida de todos los intentos.",
];

/// Rodapé dos métodos sem vídeo.
pub const NO_VIDEO_NOTE: [&str; 3] = [
    "🎬 Ainda não há vídeo de exemplo para este modo.",
    "🎬 There's no example video for this mode yet.",
    "🎬 Todavía no hay video de ejemplo para este modo.",
];

/// Monta a ajuda do alvo escolhido, no idioma [`Lang`].
pub fn help_for(profile: &GameProfile, target: &TargetDef, lang: Lang) -> Help {
    let _ = profile; // reservado: textos por jogo (hoje o alvo já basta)
    let name = target.display_name(lang);
    let (title, steps, video) = match target.method {
        HuntMethod::Starter => (
            lang.pick(["Iniciais de Hoenn", "Hoenn starters", "Iniciales de Hoenn"]),
            lang.pick([
                "1. Jogue até o Prof. Birch pedir socorro na Rota 101 — a bolsa dele fica no chão.\n\
                 2. Pare em frente à bolsa, virado para ela, e salve o jogo (Start → SAVE).\n\
                 3. Escolha o inicial em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia o jogo, abre a bolsa, escolhe o inicial e confere se ele \
                 é shiny. Se não for, repete sozinho. Use a Velocidade para acelerar.",
                "1. Play until Prof. Birch calls for help on Route 101 — his bag is on the ground.\n\
                 2. Stand in front of the bag, facing it, and save the game (Start → SAVE).\n\
                 3. Pick the starter as the target and start the hunt.\n\n\
                 The Hunter resets the game, opens the bag, picks the starter and checks \
                 whether it's shiny. If not, it repeats on its own. Use fast speed to speed it up.",
                "1. Juega hasta que el Prof. Abedul pida ayuda en la Ruta 101 — su mochila está en el suelo.\n\
                 2. Ponte frente a la mochila, mirándola, y guarda la partida (Start → GUARDAR).\n\
                 3. Elige el inicial como objetivo e inicia la caza.\n\n\
                 El Hunter reinicia el juego, abre la mochila, elige el inicial y comprueba si \
                 es shiny. Si no lo es, repite solo. Usa la velocidad acelerada para ir más rápido.",
            ])
            .to_string(),
            Some(HelpVideo::StartersRse),
        ),
        HuntMethod::StaticGift if target.name.contains("(inicial)") => (
            lang.pick(["Iniciais de Kanto", "Kanto starters", "Iniciales de Kanto"]),
            lang.pick([
                "1. No laboratório do Prof. Oak, quando ele pedir para você escolher, ande até a \
                 mesa e pare em frente à Poké Ball do inicial que você quer, virado para ela.\n\
                 2. Salve o jogo ali (Start → SAVE).\n\
                 3. Escolha o mesmo inicial em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia o jogo, pega o Pokémon da Poké Ball e confere se ele é \
                 shiny. Se não for, repete sozinho.",
                "1. In Prof. Oak's lab, when he asks you to choose, walk to the table and stand \
                 in front of the Poké Ball of the starter you want, facing it.\n\
                 2. Save the game there (Start → SAVE).\n\
                 3. Pick that same starter as the target and start the hunt.\n\n\
                 The Hunter resets the game, takes the Pokémon from the Poké Ball and checks \
                 whether it's shiny. If not, it repeats on its own.",
                "1. En el laboratorio del Prof. Oak, cuando te pida elegir, camina hasta la mesa \
                 y ponte frente a la Poké Ball del inicial que quieres, mirándola.\n\
                 2. Guarda la partida ahí (Start → GUARDAR).\n\
                 3. Elige ese mismo inicial como objetivo e inicia la caza.\n\n\
                 El Hunter reinicia el juego, toma el Pokémon de la Poké Ball y comprueba si es \
                 shiny. Si no lo es, repite solo.",
            ])
            .to_string(),
            Some(HelpVideo::StartersFrlg),
        ),
        HuntMethod::StaticGift => (
            lang.pick(["Presentes", "Gifts", "Regalos"]),
            format!(
                "{}{}",
                lang.pick([
                    "1. Vá até onde o Pokémon é entregue (a Poké Ball ou a pessoa que dá o \
                     presente) e pare em frente, virado para ela.\n\
                     2. Deixe pelo menos UMA vaga livre no time: com o time cheio o presente vai \
                     para o PC e o Hunter não o encontra.\n\
                     3. Salve o jogo ali e clique em ▶ Iniciar caça.\n\n\
                     O Hunter reinicia, aperta A até receber o presente e confere se ele é \
                     shiny.",
                    "1. Go to where the Pokémon is handed over (the Poké Ball or the person who \
                     gives it) and stand in front of it, facing it.\n\
                     2. Leave at least ONE free party slot: with a full party the gift goes to \
                     the PC and the Hunter can't find it.\n\
                     3. Save the game there and start the hunt.\n\n\
                     The Hunter resets, presses A until it receives the gift and checks whether \
                     it's shiny.",
                    "1. Ve a donde se entrega el Pokémon (la Poké Ball o la persona que da el \
                     regalo) y ponte enfrente, mirándola.\n\
                     2. Deja al menos UN espacio libre en el equipo: con el equipo lleno el regalo \
                     va al PC y el Hunter no lo encuentra.\n\
                     3. Guarda la partida ahí e inicia la caza.\n\n\
                     El Hunter reinicia, pulsa A hasta recibir el regalo y comprueba si es shiny.",
                ]),
                gift_tip(target.name, lang)
            ),
            None,
        ),
        HuntMethod::SoftResetLegendary => (
            lang.pick(["Lendários e estáticos", "Legendaries and statics", "Legendarios y estáticos"]),
            format!(
                "{}{}",
                lang.pick([
                    "1. Vá até o Pokémon e pare no tile em frente, virado para ele — sem falar \
                     com ele ainda.\n\
                     2. Salve o jogo ali (Start → SAVE).\n\
                     3. Escolha o alvo e clique em ▶ Iniciar caça.\n\n\
                     O Hunter reinicia, aperta A até a batalha começar e confere se o Pokémon é \
                     shiny. Se não for, repete sozinho.",
                    "1. Go to the Pokémon and stand on the tile in front of it, facing it — \
                     without talking to it yet.\n\
                     2. Save the game there (Start → SAVE).\n\
                     3. Pick the target and start the hunt.\n\n\
                     The Hunter resets, presses A until the battle starts and checks whether the \
                     Pokémon is shiny. If not, it repeats on its own.",
                    "1. Ve hasta el Pokémon y ponte en la casilla de enfrente, mirándolo — sin \
                     hablarle todavía.\n\
                     2. Guarda la partida ahí (Start → GUARDAR).\n\
                     3. Elige el objetivo e inicia la caza.\n\n\
                     El Hunter reinicia, pulsa A hasta que empieza el combate y comprueba si el \
                     Pokémon es shiny. Si no lo es, repite solo.",
                ]),
                legendary_tip(target.name, lang)
            ),
            None,
        ),
        HuntMethod::SoftResetApproach(dir) => (
            lang.pick([
                "Lendários que vêm até você",
                "Legendaries that come to you",
                "Legendarios que vienen hacia ti",
            ]),
            lang.pick([
                "{name} não espera você falar: a batalha começa quando você chega perto.\n\n\
                 1. Na caverna do evento do clima anormal, pare no tile ANTES do lendário, \
                 com ele um passo à {dir} do personagem.\n\
                 2. Salve o jogo ali (Start → SAVE).\n\
                 3. Escolha o alvo e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia, dá o passo até o lendário sozinho e confere se ele é \
                 shiny.",
                "{name} doesn't wait for you to talk: the battle starts when you get close.\n\n\
                 1. In the cave of the abnormal weather event, stand on the tile BEFORE the \
                 legendary, with it one step to the {dir} of your character.\n\
                 2. Save the game there (Start → SAVE).\n\
                 3. Pick the target and start the hunt.\n\n\
                 The Hunter resets, takes the step toward the legendary on its own and checks \
                 whether it's shiny.",
                "{name} no espera a que le hables: el combate empieza cuando te acercas.\n\n\
                 1. En la cueva del evento del clima anómalo, ponte en la casilla ANTES del \
                 legendario, con él a un paso a la {dir} de tu personaje.\n\
                 2. Guarda la partida ahí (Start → GUARDAR).\n\
                 3. Elige el objetivo e inicia la caza.\n\n\
                 El Hunter reinicia, da el paso hacia el legendario solo y comprueba si es shiny.",
            ])
            .replace("{name}", name)
            .replace("{dir}", direction_name(dir, lang)),
            None,
        ),
        HuntMethod::WildSpin => (
            lang.pick(["Selvagens", "Wild Pokémon", "Salvajes"]),
            lang.pick([
                "1. Pare em cima da grama alta com o personagem virado para a frente (de \
                 frente para você, olhando para baixo na tela).\n\
                 2. Escolha \"Selvagem (qualquer)\" em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
                 O Hunter gira o personagem no lugar para provocar encontros, confere cada \
                 Pokémon que aparece e foge dos que não são shiny (opção CORRER). Ele para no \
                 primeiro shiny, de qualquer espécie.\n\n\
                 Aqui não há reinício: não precisa salvar antes. Evite usar Repel.",
                "1. Stand on tall grass with your character facing forward (facing you, \
                 looking down on the screen).\n\
                 2. Pick \"Any wild Pokémon\" as the target and start the hunt.\n\n\
                 The Hunter spins your character in place to trigger encounters, checks every \
                 Pokémon that shows up and runs from the ones that aren't shiny (RUN option). It \
                 stops at the first shiny, of any species.\n\n\
                 There's no reset here: no need to save first. Avoid using Repel.",
                "1. Ponte sobre la hierba alta con tu personaje mirando al frente (hacia ti, \
                 mirando hacia abajo en la pantalla).\n\
                 2. Elige \"Cualquier salvaje\" como objetivo e inicia la caza.\n\n\
                 El Hunter gira a tu personaje en el sitio para provocar encuentros, comprueba \
                 cada Pokémon que aparece y huye de los que no son shiny (opción HUIR). Se detiene \
                 en el primer shiny, de cualquier especie.\n\n\
                 Aquí no hay reinicio: no hace falta guardar antes. Evita usar Repelente.",
            ])
            .to_string(),
            Some(HelpVideo::Wild),
        ),
    };

    let mut body = steps;
    if target.method != HuntMethod::WildSpin {
        body.push_str("\n\n");
        body.push_str(lang.pick(SAVE_NOTE));
    }
    Help { title, body, video }
}

/// Onde achar cada presente (dica extra, quando conhecida).
fn gift_tip(name: &str, lang: Lang) -> &'static str {
    const JOHTO: [&str; 3] = [
        "\n\nIniciais de Johto: laboratório do Birch, depois de completar a Pokédex de \
         Hoenn. Salve em frente à Poké Ball do alvo.",
        "\n\nJohto starters: Birch's lab, after completing the Hoenn Pokédex. Save in front \
         of the target's Poké Ball.",
        "\n\nIniciales de Johto: laboratorio de Abedul, tras completar la Pokédex de Hoenn. \
         Guarda frente a la Poké Ball del objetivo.",
    ];
    let tips: [(&str, [&str; 3]); 8] = [
        (
            "Beldum",
            [
                "\n\nBeldum: casa do Steven em Mossdeep, depois da Liga Pokémon.",
                "\n\nBeldum: Steven's house in Mossdeep, after the Pokémon League.",
                "\n\nBeldum: casa de Máximo en Algaria, después de la Liga Pokémon.",
            ],
        ),
        ("Chikorita", JOHTO),
        ("Cyndaquil", JOHTO),
        ("Totodile", JOHTO),
        (
            "Lapras",
            [
                "\n\nLapras: Silph Co., 7º andar, com o funcionário.",
                "\n\nLapras: Silph Co., 7th floor, from the employee.",
                "\n\nLapras: Silph S.A., 7.ª planta, del empleado.",
            ],
        ),
        (
            "Eevee",
            [
                "\n\nEevee: Mansão de Celadon, na Poké Ball da sala do último andar.",
                "\n\nEevee: Celadon Mansion, in the Poké Ball in the top-floor room.",
                "\n\nEevee: Mansión Azulona, en la Poké Ball de la sala del último piso.",
            ],
        ),
        (
            "Hitmon",
            [
                "\n\nHitmonlee/Hitmonchan: Dojo de Saffron, depois de vencer o mestre. Salve em \
                 frente à Poké Ball do alvo.",
                "\n\nHitmonlee/Hitmonchan: Saffron Dojo, after beating the master. Save in \
                 front of the target's Poké Ball.",
                "\n\nHitmonlee/Hitmonchan: Dojo de Azafrán, tras vencer al maestro. Guarda \
                 frente a la Poké Ball del objetivo.",
            ],
        ),
        (
            "(fóssil)",
            [
                "\n\nFósseis: entregue o fóssil ao cientista do laboratório de Cinnabar, saia e \
                 volte, e salve em frente a ele ANTES de falar de novo.",
                "\n\nFossils: give the fossil to the scientist at the Cinnabar lab, leave and come \
                 back, and save in front of him BEFORE talking to him again.",
                "\n\nFósiles: entrega el fósil al científico del laboratorio de Canela, sal y \
                 vuelve, y guarda frente a él ANTES de hablarle de nuevo.",
            ],
        ),
    ];
    tips.iter()
        .find(|(k, _)| name.contains(k))
        .map_or("", |(_, tip)| lang.pick(*tip))
}

/// Dicas de lendários com pegadinha.
fn legendary_tip(name: &str, lang: Lang) -> &'static str {
    if name.starts_with("Snorlax") {
        lang.pick([
            "\n\nSnorlax: tenha a Poké Flute na mochila — o Hunter responde SIM para tocá-la.",
            "\n\nSnorlax: carry the Poké Flute in your bag — the Hunter answers YES to play it.",
            "\n\nSnorlax: lleva la Poké Flauta en la mochila — el Hunter responde SÍ para tocarla.",
        ])
    } else {
        ""
    }
}

fn direction_name(dir: Approach, lang: Lang) -> &'static str {
    match dir {
        Approach::Left => lang.pick(["esquerda", "left", "izquierda"]),
        Approach::Right => lang.pick(["direita", "right", "derecha"]),
        Approach::Up => lang.pick(["frente (para cima)", "front (up)", "frente (hacia arriba)"]),
        Approach::Down => lang.pick([
            "frente (para baixo)",
            "front (down)",
            "frente (hacia abajo)",
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{self, PROFILES};

    fn help(code: &str, name: &str) -> Help {
        let p = games::detect(code).unwrap();
        let t = p.targets.iter().find(|t| t.name == name).unwrap();
        help_for(p, t, Lang::Pt)
    }

    #[test]
    fn every_target_has_help() {
        for p in PROFILES {
            for t in p.targets {
                for lang in [Lang::Pt, Lang::En, Lang::Es] {
                    let h = help_for(p, t, lang);
                    assert!(!h.body.is_empty(), "{} / {} / {lang:?}", p.name, t.name);
                    assert!(
                        !h.body.contains('{'),
                        "placeholder sobrou: {} / {lang:?}",
                        t.name
                    );
                }
            }
        }
    }

    #[test]
    fn videos_match_recorded_examples() {
        // Só iniciais (Hoenn e Kanto) e selvagens têm vídeo gravado.
        assert_eq!(
            help("BPEE", "Mudkip (inicial)").video,
            Some(HelpVideo::StartersRse)
        );
        assert_eq!(
            help("AXVE", "Treecko (inicial)").video,
            Some(HelpVideo::StartersRse)
        );
        assert_eq!(
            help("BPRE", "Bulbasaur (inicial)").video,
            Some(HelpVideo::StartersFrlg)
        );
        assert_eq!(
            help("BPGE", "Squirtle (inicial)").video,
            Some(HelpVideo::StartersFrlg)
        );
        assert_eq!(
            help("BPEE", "Selvagem (qualquer)").video,
            Some(HelpVideo::Wild)
        );
        for (code, name) in [
            ("BPEE", "Groudon"),
            ("BPEE", "Beldum (presente)"),
            ("BPEE", "Rayquaza"),
            ("BPRE", "Kabuto (fóssil)"),
            ("BPRE", "Mewtwo"),
        ] {
            assert_eq!(help(code, name).video, None, "{name} não tem vídeo ainda");
        }
    }

    #[test]
    fn approach_text_names_the_direction() {
        assert!(help("BPEE", "Groudon").body.contains("direita"));
        assert!(help("BPEE", "Kyogre").body.contains("esquerda"));
    }

    #[test]
    fn approach_text_is_translated() {
        let p = games::detect("BPEE").unwrap();
        let groudon = p.targets.iter().find(|t| t.name == "Groudon").unwrap();
        assert!(help_for(p, groudon, Lang::En).body.contains("to the right"));
        assert!(help_for(p, groudon, Lang::Es).body.contains("derecha"));
    }

    #[test]
    fn wild_needs_no_save_note() {
        assert!(!help("BPEE", "Selvagem (qualquer)")
            .body
            .contains("Start → SAVE), não"));
        assert!(help("BPRE", "Mewtwo").body.contains(SAVE_NOTE[0]));
    }

    #[test]
    fn video_files_exist() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/hunter-help");
        for v in [
            HelpVideo::StartersRse,
            HelpVideo::StartersFrlg,
            HelpVideo::Wild,
        ] {
            for f in [
                dir.join(format!("desktop/{}.h264", v.stem())),
                dir.join(format!("android/hunter_help/{}.mp4", v.stem())),
            ] {
                assert!(f.is_file(), "faltando {}", f.display());
            }
        }
    }
}
