//! Idioma dos textos que o Shiny Hunter mostra (nome do alvo selvagem e a ajuda
//! "Como usar"). Cada frontend escolhe: o Android passa o idioma do app; o
//! desktop usa português até ganhar o seletor dele.

/// Idiomas com tradução.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Pt,
    En,
    Es,
}

impl Lang {
    /// Pela tag/código de idioma (`"pt"`, `"pt-BR"`, `"en"`, `"es-MX"`…). Idioma
    /// sem tradução cai no inglês, como o resto do app no Android.
    pub fn from_tag(tag: &str) -> Self {
        let lang = tag
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match lang.as_str() {
            "pt" => Lang::Pt,
            "es" => Lang::Es,
            _ => Lang::En,
        }
    }

    /// Escolhe o texto deste idioma entre `[pt, en, es]`.
    pub fn pick<T>(self, [pt, en, es]: [T; 3]) -> T {
        match self {
            Lang::Pt => pt,
            Lang::En => en,
            Lang::Es => es,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Lang::from_tag("pt-BR"), Lang::Pt);
        assert_eq!(Lang::from_tag("pt"), Lang::Pt);
        assert_eq!(Lang::from_tag("es_MX"), Lang::Es);
        assert_eq!(Lang::from_tag("en"), Lang::En);
        assert_eq!(Lang::from_tag("fr"), Lang::En);
        assert_eq!(Lang::from_tag(""), Lang::En);
    }
}
