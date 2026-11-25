//! Idioma da interface (pt/en/es, como no Android).
//!
//! O idioma ativo é global (a UI inteira usa o mesmo), então qualquer ponto do
//! código traduz com [`t`] passando as três versões do texto, sem precisar de
//! `&self`. A preferência ("auto" ou um idioma fixo) é persistida pelo app; no
//! automático vale o idioma do sistema, com inglês pra idiomas sem tradução.

use std::sync::atomic::{AtomicU8, Ordering};

pub use auroragba_shiny::lang::Lang;

static CURRENT: AtomicU8 = AtomicU8::new(0);

fn to_u8(l: Lang) -> u8 {
    match l {
        Lang::Pt => 0,
        Lang::En => 1,
        Lang::Es => 2,
    }
}

/// Idioma ativo da interface.
pub fn get() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::Es,
        _ => Lang::Pt,
    }
}

fn set(l: Lang) {
    CURRENT.store(to_u8(l), Ordering::Relaxed);
}

/// Texto no idioma ativo.
pub fn t(pt: &'static str, en: &'static str, es: &'static str) -> &'static str {
    match get() {
        Lang::Pt => pt,
        Lang::En => en,
        Lang::Es => es,
    }
}

/// Idioma do sistema operacional (inglês se não der pra saber).
pub fn system() -> Lang {
    sys_locale::get_locale()
        .map(|tag| Lang::from_tag(&tag))
        .unwrap_or(Lang::En)
}

/// Preferência de idioma: seguir o sistema ou um idioma fixo.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LangPref {
    #[default]
    Auto,
    Fixed(Lang),
}

impl LangPref {
    /// Chave de persistência.
    pub fn key(self) -> &'static str {
        match self {
            LangPref::Auto => "auto",
            LangPref::Fixed(Lang::Pt) => "pt",
            LangPref::Fixed(Lang::En) => "en",
            LangPref::Fixed(Lang::Es) => "es",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "pt" => LangPref::Fixed(Lang::Pt),
            "en" => LangPref::Fixed(Lang::En),
            "es" => LangPref::Fixed(Lang::Es),
            _ => LangPref::Auto,
        }
    }

    /// Aplica a preferência (o automático resolve pelo sistema agora).
    pub fn apply(self) {
        set(match self {
            LangPref::Auto => system(),
            LangPref::Fixed(l) => l,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pref_key_roundtrips() {
        for p in [
            LangPref::Auto,
            LangPref::Fixed(Lang::Pt),
            LangPref::Fixed(Lang::En),
            LangPref::Fixed(Lang::Es),
        ] {
            assert_eq!(LangPref::from_key(p.key()), p);
        }
        assert_eq!(LangPref::from_key("xx"), LangPref::Auto);
    }

    #[test]
    fn t_follows_the_active_language() {
        LangPref::Fixed(Lang::Es).apply();
        assert_eq!(t("Salvar", "Save", "Guardar"), "Guardar");
        LangPref::Fixed(Lang::Pt).apply();
        assert_eq!(t("Salvar", "Save", "Guardar"), "Salvar");
    }
}
