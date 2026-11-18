//! Ajuda "Como usar" do Shiny Hunter: texto explicativo por alvo + vídeo de
//! exemplo (quando existe). Fica aqui, no crate compartilhado, pra que desktop e
//! Android mostrem **o mesmo** conteúdo — cada frontend só decide como desenhar.
//!
//! Os vídeos vivem em `assets/hunter-help/` na raiz do repositório, com o mesmo
//! nome-base ([`HelpVideo::stem`]) nos dois formatos:
//! - `desktop/<stem>.h264` — Annex B, embutido e decodificado pelo desktop;
//! - `android/hunter_help/<stem>.mp4` — empacotado nos assets do APK.

use crate::games::{Approach, GameProfile, HuntMethod, TargetDef};

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
const SAVE_NOTE: &str = "Importante: o Hunter usa o save DO JOGO (Start → SAVE), não os \
estados salvos do emulador. A cada tentativa ele reinicia o console e continua desse save — \
por isso o lugar onde você salva é o ponto de partida de toda tentativa.";

/// Rodapé dos métodos sem vídeo.
pub const NO_VIDEO_NOTE: &str = "🎬 Ainda não há vídeo de exemplo para este modo.";

/// Monta a ajuda do alvo escolhido.
pub fn help_for(profile: &GameProfile, target: &TargetDef) -> Help {
    let _ = profile; // reservado: textos por jogo (hoje o alvo já basta)
    let (title, steps, video) = match target.method {
        HuntMethod::Starter => (
            "Iniciais de Hoenn",
            "1. Jogue até o Prof. Birch pedir socorro na Rota 101 — a bolsa dele fica no chão.\n\
             2. Pare em frente à bolsa, virado para ela, e salve o jogo (Start → SAVE).\n\
             3. Escolha o inicial em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
             O Hunter reinicia o jogo, abre a bolsa, escolhe o inicial e confere se ele \
             é shiny. Se não for, repete sozinho. Use a Velocidade para acelerar."
                .to_string(),
            Some(HelpVideo::StartersRse),
        ),
        HuntMethod::StaticGift if target.name.contains("(inicial)") => (
            "Iniciais de Kanto",
            "1. No laboratório do Prof. Oak, quando ele pedir para você escolher, ande até a \
             mesa e pare em frente à Poké Ball do inicial que você quer, virado para ela.\n\
             2. Salve o jogo ali (Start → SAVE).\n\
             3. Escolha o mesmo inicial em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
             O Hunter reinicia o jogo, pega o Pokémon da Poké Ball e confere se ele é \
             shiny. Se não for, repete sozinho."
                .to_string(),
            Some(HelpVideo::StartersFrlg),
        ),
        HuntMethod::StaticGift => (
            "Presentes",
            format!(
                "1. Vá até onde o Pokémon é entregue (a Poké Ball ou a pessoa que dá o \
                 presente) e pare em frente, virado para ela.\n\
                 2. Deixe pelo menos UMA vaga livre no time: com o time cheio o presente vai \
                 para o PC e o Hunter não o encontra.\n\
                 3. Salve o jogo ali e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia, aperta A até receber o presente e confere se ele é \
                 shiny.{}",
                gift_tip(target.name)
            ),
            None,
        ),
        HuntMethod::SoftResetLegendary => (
            "Lendários e estáticos",
            format!(
                "1. Vá até o Pokémon e pare no tile em frente, virado para ele — sem falar \
                 com ele ainda.\n\
                 2. Salve o jogo ali (Start → SAVE).\n\
                 3. Escolha o alvo e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia, aperta A até a batalha começar e confere se o Pokémon é \
                 shiny. Se não for, repete sozinho.{}",
                legendary_tip(target.name)
            ),
            None,
        ),
        HuntMethod::SoftResetApproach(dir) => (
            "Lendários que vêm até você",
            format!(
                "{} não espera você falar: a batalha começa quando você chega perto.\n\n\
                 1. Na caverna do evento do clima anormal, pare no tile ANTES do lendário, \
                 com ele um passo à {} do personagem.\n\
                 2. Salve o jogo ali (Start → SAVE).\n\
                 3. Escolha o alvo e clique em ▶ Iniciar caça.\n\n\
                 O Hunter reinicia, dá o passo até o lendário sozinho e confere se ele é \
                 shiny.",
                target.name,
                direction_name(dir)
            ),
            None,
        ),
        HuntMethod::WildSpin => (
            "Selvagens",
            "1. Pare em cima da grama alta com o personagem virado para a frente (de \
             frente para você, olhando para baixo na tela).\n\
             2. Escolha \"Selvagem (qualquer)\" em \"Alvo\" e clique em ▶ Iniciar caça.\n\n\
             O Hunter gira o personagem no lugar para provocar encontros, confere cada \
             Pokémon que aparece e foge dos que não são shiny (opção CORRER). Ele para no \
             primeiro shiny, de qualquer espécie.\n\n\
             Aqui não há reinício: não precisa salvar antes. Evite usar Repel."
                .to_string(),
            Some(HelpVideo::Wild),
        ),
    };

    let mut body = steps;
    if target.method != HuntMethod::WildSpin {
        body.push_str("\n\n");
        body.push_str(SAVE_NOTE);
    }
    Help { title, body, video }
}

/// Onde achar cada presente (dica extra, quando conhecida).
fn gift_tip(name: &str) -> &'static str {
    let tips: [(&str, &str); 8] = [
        (
            "Beldum",
            "\n\nBeldum: casa do Steven em Mossdeep, depois da Liga Pokémon.",
        ),
        (
            "Chikorita",
            "\n\nIniciais de Johto: laboratório do Birch, depois de completar a Pokédex de \
             Hoenn. Salve em frente à Poké Ball do alvo.",
        ),
        (
            "Cyndaquil",
            "\n\nIniciais de Johto: laboratório do Birch, depois de completar a Pokédex de \
             Hoenn. Salve em frente à Poké Ball do alvo.",
        ),
        (
            "Totodile",
            "\n\nIniciais de Johto: laboratório do Birch, depois de completar a Pokédex de \
             Hoenn. Salve em frente à Poké Ball do alvo.",
        ),
        (
            "Lapras",
            "\n\nLapras: Silph Co., 7º andar, com o funcionário.",
        ),
        (
            "Eevee",
            "\n\nEevee: Mansão de Celadon, na Poké Ball da sala do último andar.",
        ),
        (
            "Hitmon",
            "\n\nHitmonlee/Hitmonchan: Dojo de Saffron, depois de vencer o mestre. Salve em \
             frente à Poké Ball do alvo.",
        ),
        (
            "(fóssil)",
            "\n\nFósseis: entregue o fóssil ao cientista do laboratório de Cinnabar, saia e \
             volte, e salve em frente a ele ANTES de falar de novo.",
        ),
    ];
    tips.iter()
        .find(|(k, _)| name.contains(k))
        .map_or("", |(_, tip)| tip)
}

/// Dicas de lendários com pegadinha.
fn legendary_tip(name: &str) -> &'static str {
    if name.starts_with("Snorlax") {
        "\n\nSnorlax: tenha a Poké Flute na mochila — o Hunter responde SIM para tocá-la."
    } else {
        ""
    }
}

fn direction_name(dir: Approach) -> &'static str {
    match dir {
        Approach::Left => "esquerda",
        Approach::Right => "direita",
        Approach::Up => "frente (para cima)",
        Approach::Down => "frente (para baixo)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{self, PROFILES};

    fn help(code: &str, name: &str) -> Help {
        let p = games::detect(code).unwrap();
        let t = p.targets.iter().find(|t| t.name == name).unwrap();
        help_for(p, t)
    }

    #[test]
    fn every_target_has_help() {
        for p in PROFILES {
            for t in p.targets {
                let h = help_for(p, t);
                assert!(!h.body.is_empty(), "{} / {}", p.name, t.name);
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
    fn wild_needs_no_save_note() {
        assert!(!help("BPEE", "Selvagem (qualquer)")
            .body
            .contains("Start → SAVE), não"));
        assert!(help("BPRE", "Mewtwo").body.contains(SAVE_NOTE));
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
