//! Configurações: janela modal com navegação lateral por seções (Pasta de ROMs,
//! Áudio, Velocidade, Vídeo, Controles, Idioma, Sobre), como no canvas.

use std::sync::Arc;

use eframe::egui::{self, vec2, RichText};

use crate::i18n::{self, t, Lang, LangPref};
use crate::shader::{MultipassKind, ShaderKind};
use crate::{theme, widgets, AuroraApp, InputConfig, Rebind, GBA_NAMES};
use auroragba_scale::Upscale;

/// Seção aberta nas Configurações.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    Roms,
    Audio,
    Speed,
    #[default]
    Video,
    Controls,
    Language,
    About,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::Roms,
        Page::Audio,
        Page::Speed,
        Page::Video,
        Page::Controls,
        Page::Language,
        Page::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Roms => t("Pasta de ROMs", "ROM folder", "Carpeta de ROM"),
            Page::Audio => t("Áudio", "Audio", "Audio"),
            Page::Speed => t("Velocidade", "Speed", "Velocidad"),
            Page::Video => t("Vídeo", "Video", "Vídeo"),
            Page::Controls => t("Controles", "Controls", "Controles"),
            Page::Language => t("Idioma", "Language", "Idioma"),
            Page::About => t("Sobre", "About", "Acerca de"),
        }
    }
}

/// Ordem dos botões na tabela de controles (índices de [`GBA_NAMES`]): A, B,
/// L, R, Start, Select e as direções.
const CONTROL_ORDER: [usize; 10] = [0, 1, 9, 8, 3, 2, 6, 7, 5, 4];

/// Nome traduzido de um botão do GBA (índice de [`GBA_NAMES`]).
pub fn button_label(i: usize) -> &'static str {
    match i {
        4 => t("Direita", "Right", "Derecha"),
        5 => t("Esquerda", "Left", "Izquierda"),
        6 => t("Cima", "Up", "Arriba"),
        7 => t("Baixo", "Down", "Abajo"),
        _ => GBA_NAMES[i],
    }
}

const SOURCE_URL: &str = "https://github.com/Thallanar/GBAEmu";

impl AuroraApp {
    /// Desenha as Configurações (se abertas).
    pub(crate) fn settings_window(
        &mut self,
        ctx: &egui::Context,
        gl: Option<&Arc<eframe::glow::Context>>,
    ) {
        if !self.settings_open {
            return;
        }
        // Remapeando: o Esc cancela a captura em vez de fechar a janela.
        let esc_closes = self.rebinding.is_none();
        let close = widgets::modal(ctx, "settings", vec2(980.0, 660.0), esc_closes, |ui| {
            let full = ui.max_rect();
            let nav_rect = egui::Rect::from_min_size(full.min, vec2(240.0, full.height()));
            let body_rect =
                egui::Rect::from_min_max(egui::pos2(nav_rect.right(), full.top()), full.max);
            ui.painter().rect_filled(
                nav_rect,
                egui::Rounding {
                    nw: 20.0,
                    sw: 20.0,
                    ne: 0.0,
                    se: 0.0,
                },
                theme::BG_DEEP,
            );
            ui.painter().vline(
                nav_rect.right(),
                nav_rect.y_range(),
                egui::Stroke::new(1.0_f32, theme::LINE),
            );

            // Navegação.
            let mut nav = ui.child_ui(
                nav_rect.shrink2(vec2(12.0, 20.0)),
                egui::Layout::top_down(egui::Align::Min),
            );
            nav.spacing_mut().item_spacing.y = 4.0;
            nav.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(t("Configurações", "Settings", "Ajustes"))
                        .font(theme::bold(20.0))
                        .color(theme::TEXT),
                );
            });
            nav.add_space(8.0);
            for page in Page::ALL {
                if widgets::nav_item(&mut nav, page.title(), self.settings_page == page).clicked() {
                    self.settings_page = page;
                    self.rebinding = None;
                }
            }
            nav.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("AuroraGBA {}", env!("CARGO_PKG_VERSION")))
                            .font(theme::regular(12.0))
                            .color(theme::FAINT),
                    );
                });
            });

            // Conteúdo.
            let mut body = ui.child_ui(
                body_rect.shrink2(vec2(28.0, 20.0)),
                egui::Layout::top_down(egui::Align::Min),
            );
            body.horizontal(|ui| {
                ui.label(
                    RichText::new(self.settings_page.title())
                        .font(theme::bold(22.0))
                        .color(theme::TEXT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::close_button(ui).clicked() {
                        self.settings_open = false;
                        self.rebinding = None;
                    }
                });
            });
            body.add_space(8.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(&mut body, |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    match self.settings_page {
                        Page::Roms => self.page_roms(ui),
                        Page::Audio => self.page_audio(ui),
                        Page::Speed => self.page_speed(ui),
                        Page::Video => self.page_video(ui, gl),
                        Page::Controls => self.page_controls(ui),
                        Page::Language => self.page_language(ui),
                        Page::About => self.page_about(ui),
                    }
                });
        });
        if close {
            self.settings_open = false;
            self.rebinding = None;
        }
    }

    fn page_roms(&mut self, ui: &mut egui::Ui) {
        let mut pick = false;
        widgets::card(ui, |ui| {
            let (path, sub) = match &self.library.dir {
                Some(d) => (
                    d.display().to_string(),
                    format!(
                        "{} {}",
                        self.library.entries.len(),
                        t("ROMs encontradas", "ROMs found", "ROM encontradas")
                    ),
                ),
                None => (
                    t(
                        "Nenhuma pasta escolhida",
                        "No folder chosen",
                        "Ninguna carpeta elegida",
                    )
                    .to_string(),
                    String::new(),
                ),
            };
            widgets::row(
                ui,
                &path,
                Some(&sub).filter(|s| !s.is_empty()).map(|s| s.as_str()),
                |ui| {
                    if widgets::outline_button(
                        ui,
                        t("Trocar pasta…", "Change folder…", "Cambiar carpeta…"),
                        40.0,
                    )
                    .clicked()
                    {
                        pick = true;
                    }
                },
            );
        });
        ui.label(
            RichText::new(t(
                "Os saves do jogo (.sav) e os estados ficam ao lado de cada ROM. Trocar de pasta não apaga nada da pasta antiga.",
                "Game saves (.sav) and save states stay next to each ROM. Changing folders doesn't delete anything from the old one.",
                "Las partidas (.sav) y los estados quedan junto a cada ROM. Cambiar de carpeta no borra nada de la anterior.",
            ))
            .font(theme::regular(13.0))
            .color(theme::MUTED),
        );
        if pick {
            self.pick_library_dir();
        }
    }

    fn page_audio(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        widgets::card(ui, |ui| {
            widgets::row(
                ui,
                t("Som", "Sound", "Sonido"),
                Some(t(
                    "Desligar silencia o jogo sem pausar",
                    "Turning it off mutes the game without pausing",
                    "Apagarlo silencia el juego sin pausar",
                )),
                |ui| changed |= widgets::switch(ui, &mut self.sound_on).changed(),
            );
        });
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("Volume", "Volume", "Volumen"))
                        .font(theme::semibold(15.0))
                        .color(theme::TEXT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{}%", self.volume))
                            .font(theme::semibold(14.0))
                            .color(theme::ACCENT),
                    );
                });
            });
            ui.spacing_mut().slider_width = ui.available_width();
            changed |= ui
                .add_enabled(
                    self.sound_on,
                    egui::Slider::new(&mut self.volume, 0..=100)
                        .step_by(5.0)
                        .show_value(false),
                )
                .changed();
        });
        widgets::card(ui, |ui| {
            widgets::row(
                ui,
                t(
                    "Som ao acelerar",
                    "Sound while fast-forwarding",
                    "Sonido al acelerar",
                ),
                Some(t(
                    "Mudo evita o som picotado; acelerado toca rápido",
                    "Muted avoids choppy sound; fast plays it sped up",
                    "Silencio evita el sonido entrecortado; acelerado lo reproduce rápido",
                )),
                |ui| {
                    let mut sel = usize::from(self.ff_sound);
                    let opts = [
                        t("Mudo", "Muted", "Silencio"),
                        t("Acelerado", "Fast", "Acelerado"),
                    ];
                    if widgets::segmented(ui, &mut sel, &opts, 220.0) {
                        self.ff_sound = sel == 1;
                    }
                },
            );
        });
        if changed {
            self.apply_audio_gain();
        }
    }

    fn page_speed(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, |ui| {
            widgets::titled(
                ui,
                t(
                    "Velocidade ao acelerar",
                    "Fast-forward speed",
                    "Velocidad al acelerar",
                ),
                Some(t(
                    "Vale pro botão Acelerar e pra tecla Espaço",
                    "Applies to the Fast forward button and the Space key",
                    "Vale para el botón Acelerar y la tecla Espacio",
                )),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let opts: [(u32, String); 4] = [
                    (2, "2×".into()),
                    (4, "4×".into()),
                    (8, "8×".into()),
                    (0, t("Máximo", "Maximum", "Máximo").into()),
                ];
                let w = (ui.available_width() - 3.0 * 8.0) / 4.0;
                for (m, label) in opts {
                    if widgets::option_button(ui, &label, self.ff_mult == m, vec2(w, 48.0))
                        .clicked()
                    {
                        self.ff_mult = m;
                    }
                }
            });
        });
        widgets::card(ui, |ui| {
            widgets::row(
                ui,
                t("Tecla Espaço", "Space key", "Tecla Espacio"),
                Some(if self.space_toggle {
                    t(
                        "Cada toque liga ou desliga o acelerado",
                        "Each press turns fast-forward on or off",
                        "Cada pulsación activa o desactiva el acelerado",
                    )
                } else {
                    t(
                        "Segurar acelera só enquanto pressionada",
                        "Hold to fast-forward only while pressed",
                        "Mantener acelera solo mientras está pulsada",
                    )
                }),
                |ui| {
                    let mut sel = usize::from(self.space_toggle);
                    let opts = [
                        t("Segurar", "Hold", "Mantener"),
                        t("Liga/desliga", "Toggle", "Alternar"),
                    ];
                    if widgets::segmented(ui, &mut sel, &opts, 220.0) {
                        self.space_toggle = sel == 1;
                        self.ff_toggle = false;
                    }
                },
            );
        });
    }

    fn page_video(&mut self, ui: &mut egui::Ui, gl: Option<&Arc<eframe::glow::Context>>) {
        widgets::card(ui, |ui| {
            widgets::titled(
                ui,
                t("Escala da tela", "Screen scale", "Escala de pantalla"),
                Some(t(
                    "Sempre inteira, pra os pixels saírem nítidos. Ajustar à janela escolhe a maior que cabe.",
                    "Always a whole number, so pixels stay sharp. Fit to window picks the largest that fits.",
                    "Siempre entera, para que los píxeles salgan nítidos. Ajustar a la ventana elige la mayor que cabe.",
                )),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let w = (ui.available_width() - 6.0 * 6.0) / 7.0;
                for k in 1..=6 {
                    let on = self.scale.round() as i32 == k;
                    if widgets::option_button(ui, &format!("{k}×"), on, vec2(w, 40.0)).clicked() {
                        self.scale = k as f32;
                    }
                }
                let fit = t("Ajustar", "Fit", "Ajustar");
                if widgets::option_button(ui, fit, self.scale < 1.0, vec2(w, 40.0)).clicked() {
                    self.scale = 0.0;
                }
            });
        });

        if self.shader.is_none() {
            ui.label(
                RichText::new(t(
                    "Shaders e upscale indisponíveis: o vídeo não está usando OpenGL.",
                    "Shaders and upscaling unavailable: video isn't using OpenGL.",
                    "Shaders y escalado no disponibles: el vídeo no usa OpenGL.",
                ))
                .color(theme::MUTED),
            );
            return;
        }

        let mut import = false;
        ui.columns(2, |cols| {
            widgets::card(&mut cols[0], |ui| {
                ui.label(
                    RichText::new("Shader")
                        .font(theme::semibold(15.0))
                        .color(theme::TEXT),
                );
                let w = (ui.available_width() - 6.0) / 2.0;
                egui::Grid::new("shader_grid")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        let mut n = 0;
                        for kind in ShaderKind::ALL {
                            let on = self.shader_multipass.is_none()
                                && !self.use_custom
                                && self.shader_kind == kind;
                            let label = if kind == ShaderKind::None {
                                t("Nenhum", "None", "Ninguno")
                            } else {
                                kind.label()
                            };
                            if widgets::option_button(ui, label, on, vec2(w, 36.0)).clicked() {
                                self.shader_kind = kind;
                                self.use_custom = false;
                                self.shader_multipass = None;
                            }
                            n += 1;
                            if n % 2 == 0 {
                                ui.end_row();
                            }
                        }
                        for mp in MultipassKind::ALL {
                            if widgets::option_button(
                                ui,
                                mp.label(),
                                self.shader_multipass == Some(mp),
                                vec2(w, 36.0),
                            )
                            .clicked()
                            {
                                self.shader_multipass = Some(mp);
                                self.use_custom = false;
                            }
                            n += 1;
                            if n % 2 == 0 {
                                ui.end_row();
                            }
                        }
                        if !self.custom_label.is_empty() {
                            let label = self.custom_label.clone();
                            if widgets::option_button(ui, &label, self.use_custom, vec2(w, 36.0))
                                .clicked()
                            {
                                self.use_custom = true;
                                self.shader_multipass = None;
                            }
                        }
                    });
                if widgets::text_link(ui, t("Importar .frag…", "Import .frag…", "Importar .frag…"))
                    .clicked()
                {
                    import = true;
                }
                if let Some(err) = &self.shader_error {
                    ui.label(
                        RichText::new(format!(
                            "{}: {err}",
                            t("Não compilou", "Didn't compile", "No compiló")
                        ))
                        .font(theme::regular(12.0))
                        .color(egui::Color32::from_rgb(0xF4, 0xA3, 0xA4)),
                    );
                }
            });
            widgets::card(&mut cols[1], |ui| {
                ui.label(
                    RichText::new("Upscale")
                        .font(theme::semibold(15.0))
                        .color(theme::TEXT),
                );
                let w = (ui.available_width() - 6.0) / 2.0;
                egui::Grid::new("upscale_grid")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        for (n, u) in Upscale::ALL.into_iter().enumerate() {
                            let label = if u == Upscale::Off {
                                t("Nenhum", "None", "Ninguno")
                            } else {
                                u.label()
                            };
                            if widgets::option_button(ui, label, self.upscale == u, vec2(w, 36.0))
                                .clicked()
                            {
                                self.upscale = u;
                            }
                            if n % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.label(
                    RichText::new(t(
                        "Aplicado antes do shader.",
                        "Applied before the shader.",
                        "Se aplica antes del shader.",
                    ))
                    .font(theme::regular(12.0))
                    .color(theme::FAINT),
                );
            });
        });
        if import {
            self.import_shader(gl);
        }
    }

    fn page_controls(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(t(
                "Clique num campo e aperte a tecla ou o botão do controle. Esc cancela.",
                "Click a field and press the key or controller button. Esc cancels.",
                "Haz clic en un campo y pulsa la tecla o el botón del mando. Esc cancela.",
            ))
            .font(theme::regular(13.0))
            .color(theme::MUTED),
        );
        widgets::card(ui, |ui| {
            let col = (ui.available_width() - 2.0 * 12.0) / 3.0;
            egui::Grid::new("controls_grid")
                .num_columns(3)
                .spacing([12.0, 4.0])
                .min_col_width(col)
                .show(ui, |ui| {
                    for h in [
                        t("Botão do GBA", "GBA button", "Botón de GBA"),
                        t("Teclado", "Keyboard", "Teclado"),
                        t("Controle", "Controller", "Mando"),
                    ] {
                        widgets::section_label(ui, h);
                    }
                    ui.end_row();
                    let waiting = t("Aperte…", "Press…", "Pulsa…");
                    for i in CONTROL_ORDER {
                        ui.label(
                            RichText::new(button_label(i))
                                .font(theme::semibold(14.0))
                                .color(theme::TEXT),
                        );
                        let key = if self.rebinding == Some(Rebind::Key(i)) {
                            waiting.to_string()
                        } else {
                            key_display(self.input.keys[i])
                        };
                        if bind_button(ui, &key, self.rebinding == Some(Rebind::Key(i)), col)
                            .clicked()
                        {
                            self.rebinding = Some(Rebind::Key(i));
                        }
                        let pad = if self.rebinding == Some(Rebind::Pad(i)) {
                            waiting.to_string()
                        } else {
                            self.input.pads[i]
                                .map(pad_display)
                                .unwrap_or_else(|| "—".to_string())
                        };
                        if bind_button(ui, &pad, self.rebinding == Some(Rebind::Pad(i)), col)
                            .clicked()
                        {
                            self.rebinding = Some(Rebind::Pad(i));
                        }
                        ui.end_row();
                    }
                });
        });
        let pads = self.gilrs.as_ref().map_or(0, |g| g.gamepads().count());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{}: {pads}",
                    t(
                        "Controles conectados",
                        "Controllers connected",
                        "Mandos conectados"
                    )
                ))
                .font(theme::regular(13.0))
                .color(theme::MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::text_link(ui, t("Restaurar padrão", "Restore defaults", "Restablecer"))
                    .clicked()
                {
                    self.input = InputConfig::default();
                    self.rebinding = None;
                }
            });
        });
        ui.label(
            RichText::new(t(
                "Atalhos: F5 salvar · F9 carregar · F12 captura · Espaço acelerar · R voltar no tempo",
                "Shortcuts: F5 save · F9 load · F12 screenshot · Space fast forward · R rewind",
                "Atajos: F5 guardar · F9 cargar · F12 captura · Espacio acelerar · R retroceder",
            ))
            .font(theme::regular(12.0))
            .color(theme::FAINT),
        );
    }

    fn page_language(&mut self, ui: &mut egui::Ui) {
        let options = [
            (
                LangPref::Auto,
                t(
                    "Automático (idioma do sistema)",
                    "Automatic (system language)",
                    "Automático (idioma del sistema)",
                ),
            ),
            (LangPref::Fixed(Lang::Pt), "Português"),
            (LangPref::Fixed(Lang::En), "English"),
            (LangPref::Fixed(Lang::Es), "Español"),
        ];
        egui::Frame::none()
            .fill(theme::SURFACE)
            .rounding(14.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                for (n, (pref, name)) in options.into_iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 56.0),
                        egui::Sense::click(),
                    );
                    let on = self.lang_pref == pref;
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, 14.0, theme::SURFACE2);
                    }
                    if n > 0 {
                        p.hline(
                            rect.x_range().shrink(16.0),
                            rect.top(),
                            egui::Stroke::new(1.0_f32, theme::DIVIDER),
                        );
                    }
                    p.text(
                        egui::pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        name,
                        theme::semibold(15.0),
                        theme::TEXT,
                    );
                    let c = egui::pos2(rect.right() - 27.0, rect.center().y);
                    p.circle_stroke(
                        c,
                        10.0,
                        egui::Stroke::new(2.0_f32, if on { theme::ACCENT } else { theme::OFF }),
                    );
                    if on {
                        p.circle_filled(c, 5.0, theme::ACCENT);
                    }
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.lang_pref = pref;
                        pref.apply();
                    }
                }
            });
        if self.lang_pref == LangPref::Auto {
            let sys = match i18n::system() {
                Lang::Pt => "Português",
                Lang::En => "English",
                Lang::Es => "Español",
            };
            ui.label(
                RichText::new(format!(
                    "{}: {sys}",
                    t("Idioma do sistema", "System language", "Idioma del sistema")
                ))
                .font(theme::regular(13.0))
                .color(theme::MUTED),
            );
        }
    }

    fn page_about(&mut self, ui: &mut egui::Ui) {
        let icon = self.about_icon(ui.ctx());
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Image::new(&icon)
                        .fit_to_exact_size(vec2(56.0, 56.0))
                        .rounding(14.0),
                );
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(
                        RichText::new("AuroraGBA")
                            .font(theme::semibold(15.0))
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} {} · {}",
                            t("Versão", "Version", "Versión"),
                            env!("CARGO_PKG_VERSION"),
                            t(
                                "código aberto, GPL-3.0",
                                "open source, GPL-3.0",
                                "código abierto, GPL-3.0"
                            )
                        ))
                        .font(theme::regular(13.0))
                        .color(theme::MUTED),
                    );
                    ui.hyperlink_to(
                        RichText::new(t(
                            "Código no GitHub",
                            "Source on GitHub",
                            "Código en GitHub",
                        ))
                        .font(theme::semibold(13.0)),
                        SOURCE_URL,
                    );
                });
            });
        });
        widgets::card(ui, |ui| {
            widgets::section_label(ui, t("Terceiros", "Third party", "Terceros"));
            for (name, license) in [
                ("ScaleFX (Sp00kyFox)", "MIT"),
                ("HQx", "LGPL-2.1"),
                ("xBRZ", "GPL-3.0"),
                ("Inter (fonte)", "OFL-1.1"),
            ] {
                let name = if name.starts_with("Inter") {
                    t("Inter (fonte)", "Inter (font)", "Inter (fuente)")
                } else {
                    name
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(license).color(theme::MUTED));
                    });
                });
            }
        });
        ui.label(
            RichText::new(t(
                "O AuroraGBA não tem relação com a Nintendo. Use só ROMs dos seus próprios cartuchos.",
                "AuroraGBA isn't affiliated with Nintendo. Only use ROMs from cartridges you own.",
                "AuroraGBA no está afiliado a Nintendo. Usa solo ROM de tus propios cartuchos.",
            ))
            .font(theme::regular(12.0))
            .color(theme::FAINT),
        );
    }

    /// Ícone do app (o mesmo da janela) como textura, carregado uma vez.
    fn about_icon(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.about_icon
            .get_or_insert_with(|| {
                let img = image::load_from_memory(include_bytes!("../assets/auroragba.png"))
                    .map(|i| i.to_rgba8())
                    .unwrap_or_default();
                let (w, h) = (img.width() as usize, img.height() as usize);
                let color = egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
                ctx.load_texture("app-icon", color, egui::TextureOptions::LINEAR)
            })
            .clone()
    }

    /// Volume efetivo no áudio (0 com o som desligado).
    pub(crate) fn apply_audio_gain(&mut self) {
        let gain = if self.sound_on {
            self.volume as f32 / 100.0
        } else {
            0.0
        };
        if let Some(a) = &mut self.audio {
            a.set_gain(gain);
        }
    }
}

/// Nome de tecla pra mostrar (setas como símbolo).
fn key_display(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowUp => "↑".into(),
        egui::Key::ArrowDown => "↓".into(),
        egui::Key::ArrowLeft => "←".into(),
        egui::Key::ArrowRight => "→".into(),
        egui::Key::Space => t("Espaço", "Space", "Espacio").into(),
        k => k.name().into(),
    }
}

/// Nome de botão de controle pra mostrar (o nome estável de [`crate::pad_name`]
/// é técnico demais: "DPadUp", "LeftTrigger"…).
fn pad_display(b: gilrs::Button) -> String {
    use gilrs::Button as P;
    match b {
        P::South => t("Sul", "South", "Sur").into(),
        P::East => t("Leste", "East", "Este").into(),
        P::North => t("Norte", "North", "Norte").into(),
        P::West => t("Oeste", "West", "Oeste").into(),
        P::LeftTrigger => "LB".into(),
        P::RightTrigger => "RB".into(),
        P::LeftTrigger2 => "LT".into(),
        P::RightTrigger2 => "RT".into(),
        P::LeftThumb => "L3".into(),
        P::RightThumb => "R3".into(),
        P::DPadUp => "D-pad ↑".into(),
        P::DPadDown => "D-pad ↓".into(),
        P::DPadLeft => "D-pad ←".into(),
        P::DPadRight => "D-pad →".into(),
        b => crate::pad_name(b).into(),
    }
}

/// Campo da tabela de controles: mostra o binding; azul enquanto espera a tecla.
fn bind_button(ui: &mut egui::Ui, label: &str, waiting: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), egui::Sense::click());
    let p = ui.painter();
    let border = if waiting {
        theme::ACCENT
    } else if resp.hovered() {
        theme::TEXT_SOFT
    } else {
        theme::BORDER
    };
    p.rect_filled(rect, 8.0, theme::BG);
    p.rect_stroke(rect.shrink(0.5), 8.0, egui::Stroke::new(1.0_f32, border));
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        theme::regular(13.0),
        if waiting { theme::ACCENT } else { theme::TEXT },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
