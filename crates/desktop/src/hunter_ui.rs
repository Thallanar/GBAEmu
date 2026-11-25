//! Painel do Shiny Hunter, em quatro estados (como no canvas):
//!
//! 1. **Esperando o alvo**: abre sempre assim, "Parado", sem sprite nenhum.
//!    Mostrar o sprite de cara fazia parecer que a caça já tinha começado.
//! 2. **Pronto**: alvo escolhido, sprites normal/shiny, dica de preparo e
//!    "Iniciar caça".
//! 3. **Caçando**: estatísticas, velocidade e detalhes técnicos recolhidos.
//! 4. **Encontrado**: o shiny em destaque; um aviso no topo do jogo oferece
//!    salvar o estado. Não pausa: o próprio jogo é o indicador.

use std::time::{Duration, Instant};

use auroragba_shiny::games::{TargetDef, TargetKind};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Rect, RichText, Sense, Stroke};

use crate::i18n::{self, t, Lang};
use crate::icons::{self, Icon};
use crate::widgets::{self, ButtonKind};
use crate::{theme, AuroraApp};

/// Velocidades da caça (frames de emulação por atualização de tela).
pub const HUNT_SPEEDS: [u32; 5] = [1, 4, 16, 64, 2000];

const WARN: Color32 = Color32::from_rgb(0xF0, 0xB0, 0x60);

/// Número com separador de milhar do idioma ("4.812" / "4,812").
fn fmt_int(n: u64) -> String {
    let sep = if i18n::get() == Lang::En { ',' } else { '.' };
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Decimal com a vírgula do idioma.
fn fmt_dec(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if i18n::get() == Lang::En {
        s
    } else {
        s.replace('.', ",")
    }
}

fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn kind_title(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Legendary => t("Lendários", "Legendaries", "Legendarios"),
        TargetKind::Starter => t("Iniciais", "Starters", "Iniciales"),
        TargetKind::Gift => t("Presentes", "Gifts", "Regalos"),
        TargetKind::Wild => t("Selvagem", "Wild", "Salvajes"),
    }
}

fn kind_single(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Legendary => t("Lendário", "Legendary", "Legendario"),
        TargetKind::Starter => t("Inicial", "Starter", "Inicial"),
        TargetKind::Gift => t("Presente", "Gift", "Regalo"),
        TargetKind::Wild => t("Selvagem", "Wild", "Salvaje"),
    }
}

/// Dica de preparo pro passo 2, por tipo de alvo.
fn prep_tip(target: &TargetDef, name: &str) -> String {
    match target.kind() {
        TargetKind::Legendary => format!(
            "{} {name}. {}",
            t("Salve o jogo parado na frente do", "Save the game standing in front of", "Guarda la partida parado frente a"),
            t(
                "A caça reinicia a partir desse save até ele sair shiny.",
                "The hunt restarts from that save until it comes out shiny.",
                "La caza se reinicia desde esa partida hasta que salga shiny.",
            )
        ),
        TargetKind::Starter => t(
            "Salve o jogo logo antes de escolher o inicial. A caça reinicia a partir desse save até ele sair shiny.",
            "Save the game right before picking the starter. The hunt restarts from that save until it comes out shiny.",
            "Guarda la partida justo antes de elegir el inicial. La caza se reinicia desde esa partida hasta que salga shiny.",
        )
        .to_string(),
        TargetKind::Gift => t(
            "Salve o jogo parado na frente de quem dá o presente, com um espaço livre no time.",
            "Save the game in front of whoever gives the gift, with a free slot in your party.",
            "Guarda la partida frente a quien da el regalo, con un hueco libre en el equipo.",
        )
        .to_string(),
        TargetKind::Wild => t(
            "Fique parado na grama com o primeiro do time saudável. A caça gira no lugar até aparecer um shiny.",
            "Stand still in the grass with a healthy lead Pokémon. The hunt spins in place until a shiny shows up.",
            "Quédate quieto en la hierba con el primero del equipo sano. La caza gira en el sitio hasta que aparezca un shiny.",
        )
        .to_string(),
    }
}

impl AuroraApp {
    fn hunt_elapsed(&self) -> Duration {
        match self.hunt_started {
            Some(start) => self.hunt_stopped.unwrap_or_else(Instant::now) - start,
            None => Duration::ZERO,
        }
    }

    /// Chance acumulada de já ter visto ao menos um shiny (1 em 8192 cada).
    fn hunt_chance(&self) -> f64 {
        1.0 - (1.0 - 1.0 / 8192.0_f64).powi(self.hunter.attempts.min(i32::MAX as u64) as i32)
    }

    /// Fecha o painel. Fora da caça, a escolha do alvo se perde: o painel
    /// sempre reabre esperando a seleção.
    pub(crate) fn close_hunter(&mut self) {
        if self.hunting {
            return;
        }
        self.show_hunter = false;
        self.selected_target = None;
        self.hunt_warning = None;
        if self.hunter.found {
            self.reset_hunt();
        }
    }

    fn reset_hunt(&mut self) {
        self.hunter = auroragba_shiny::Hunter::new();
        self.hunt_started = None;
        self.hunt_stopped = None;
        self.shiny_banner = false;
    }

    /// O painel lateral (largura fixa).
    pub(crate) fn hunter_panel(&mut self, ui: &mut egui::Ui) {
        let Some(profile) = self.profile else {
            return;
        };
        let full = ui.max_rect();
        let foot_h = 76.0;
        let body_rect = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - foot_h));
        let foot_rect = Rect::from_min_max(pos2(full.left(), full.bottom() - foot_h), full.max);
        ui.painter().hline(
            foot_rect.x_range(),
            foot_rect.top(),
            Stroke::new(1.0_f32, theme::LINE),
        );
        let mut body = ui.child_ui(
            body_rect.shrink2(vec2(16.0, 0.0)),
            egui::Layout::top_down(egui::Align::Min),
        );
        body.set_clip_rect(body_rect);
        body.spacing_mut().item_spacing.y = 12.0;
        let mut foot = ui.child_ui(
            foot_rect.shrink2(vec2(16.0, 15.0)),
            egui::Layout::left_to_right(egui::Align::Center),
        );
        let lang = i18n::get();

        if self.hunting {
            let target = profile.targets[self.selected_target.unwrap_or(0)];
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(&mut body, |ui| {
                    self.panel_hunting(ui, &target, target.display_name(lang))
                });
            let w = foot.available_width();
            if widgets::wide_button(
                &mut foot,
                t("Parar caça", "Stop hunt", "Detener caza"),
                vec2(w, 46.0),
                ButtonKind::Danger,
            )
            .clicked()
            {
                self.hunting = false;
                self.running = true;
                self.hunt_stopped = Some(Instant::now());
            }
        } else if self.hunter.found {
            let target = profile.targets[self.selected_target.unwrap_or(0)];
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(&mut body, |ui| self.panel_found(ui, &target));
            let w = (foot.available_width() - 10.0) / 2.0;
            foot.spacing_mut().item_spacing.x = 10.0;
            if widgets::wide_button(
                &mut foot,
                t("Salvar estado", "Save state", "Guardar estado"),
                vec2(w, 46.0),
                ButtonKind::Accent,
            )
            .clicked()
            {
                self.open_states_to_save();
            }
            if widgets::wide_button(
                &mut foot,
                t("Nova caça", "New hunt", "Nueva caza"),
                vec2(w, 46.0),
                ButtonKind::Outline,
            )
            .clicked()
            {
                self.reset_hunt();
                self.selected_target = None;
            }
        } else if let Some(i) = self.selected_target {
            let target = profile.targets[i];
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(&mut body, |ui| {
                    self.panel_ready(ui, profile.name, &target, target.display_name(lang))
                });
            foot.spacing_mut().item_spacing.x = 10.0;
            let help = help_button(&mut foot);
            if help.clicked() {
                self.show_hunt_help = true;
            }
            let w = foot.available_width();
            if widgets::wide_button(
                &mut foot,
                t("Iniciar caça", "Start hunt", "Iniciar caza"),
                vec2(w, 46.0),
                ButtonKind::Accent,
            )
            .clicked()
            {
                self.start_hunt();
            }
        } else {
            self.panel_pick(&mut body, profile.name, profile.targets);
            let w = foot.available_width();
            widgets::wide_button(
                &mut foot,
                t(
                    "Escolha um alvo acima",
                    "Pick a target above",
                    "Elige un objetivo arriba",
                ),
                vec2(w, 46.0),
                ButtonKind::Disabled,
            );
        }
    }

    /// Cabeçalho do painel. Devolve true se clicou em fechar.
    fn panel_header(
        &mut self,
        ui: &mut egui::Ui,
        eyebrow: Option<&str>,
        title: &str,
        sub: Option<&str>,
        chip: Option<(&str, bool)>,
        closable: bool,
    ) -> bool {
        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(e) = eyebrow {
                    ui.label(
                        RichText::new(e.to_uppercase())
                            .font(theme::semibold(11.0))
                            .color(theme::ACCENT),
                    );
                }
                ui.label(
                    RichText::new(title)
                        .font(theme::bold(if eyebrow.is_some() { 20.0 } else { 18.0 }))
                        .color(theme::TEXT),
                );
                if let Some(s) = sub {
                    ui.label(
                        RichText::new(s)
                            .font(theme::regular(13.0))
                            .color(theme::MUTED),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if closable {
                    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                    if resp.hovered() {
                        ui.painter()
                            .circle_filled(rect.center(), 16.0, theme::SURFACE);
                    }
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(rect.center(), vec2(16.0, 16.0)),
                        Icon::Close,
                        theme::MUTED,
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(t("Fechar", "Close", "Cerrar"))
                        .clicked()
                    {
                        close = true;
                    }
                }
                if let Some((label, running)) = chip {
                    status_chip(ui, label, running);
                }
            });
        });
        close
    }

    fn panel_pick(&mut self, ui: &mut egui::Ui, game: &str, targets: &[TargetDef]) {
        if self.panel_header(
            ui,
            None,
            "Shiny Hunter",
            Some(game),
            Some((t("Parado", "Stopped", "Detenido"), false)),
            true,
        ) {
            self.close_hunter();
            return;
        }
        step_card(
            ui,
            "1",
            t(
                "Escolha o que caçar",
                "Choose what to hunt",
                "Elige qué cazar",
            ),
            t(
                "Nada roda até você escolher um alvo e clicar em Iniciar.",
                "Nothing runs until you pick a target and click Start.",
                "Nada se ejecuta hasta que elijas un objetivo y hagas clic en Iniciar.",
            ),
        );
        let lang = i18n::get();
        let mut picked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for kind in [
                    TargetKind::Legendary,
                    TargetKind::Starter,
                    TargetKind::Gift,
                    TargetKind::Wild,
                ] {
                    let group: Vec<(usize, &TargetDef)> = targets
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.kind() == kind)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    ui.add_space(4.0);
                    widgets::section_label(ui, kind_title(kind));
                    let w = (ui.available_width() - 12.0) / 3.0;
                    for row in group.chunks(3) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (i, target) in row {
                                if target_chip(ui, target.display_name(lang), w).clicked() {
                                    picked = Some(*i);
                                }
                            }
                        });
                    }
                }
            });
        if picked.is_some() {
            self.selected_target = picked;
            self.hunt_warning = None;
        }
    }

    fn panel_ready(&mut self, ui: &mut egui::Ui, game: &str, target: &TargetDef, name: &str) {
        if self.panel_header(
            ui,
            None,
            "Shiny Hunter",
            Some(game),
            Some((t("Parado", "Stopped", "Detenido"), false)),
            true,
        ) {
            self.close_hunter();
            return;
        }
        let mut change = false;
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    widgets::section_label(ui, t("Alvo", "Target", "Objetivo"));
                    ui.label(
                        RichText::new(name)
                            .font(theme::bold(20.0))
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new(kind_single(target.kind()))
                            .font(theme::regular(12.0))
                            .color(theme::MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::text_link(ui, t("Trocar", "Change", "Cambiar")).clicked() {
                        change = true;
                    }
                });
            });
        });
        self.sprite_pair(ui, target.species, 96.0);
        self.speed_card(ui);
        if let Some(w) = &self.hunt_warning {
            egui::Frame::none()
                .fill(WARN.gamma_multiply(0.12))
                .stroke(Stroke::new(1.0_f32, WARN.gamma_multiply(0.6)))
                .rounding(12.0)
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(w.as_str())
                            .font(theme::regular(13.0))
                            .color(WARN),
                    );
                });
        }
        step_card(
            ui,
            "2",
            t(
                "Prepare e inicie",
                "Get ready and start",
                "Prepárate e inicia",
            ),
            &prep_tip(target, name),
        );
        if change {
            self.selected_target = None;
            self.hunt_warning = None;
        }
    }

    fn panel_hunting(&mut self, ui: &mut egui::Ui, target: &TargetDef, name: &str) {
        self.panel_header(
            ui,
            Some(t("Caçando", "Hunting", "Cazando")),
            name,
            None,
            Some((t("Rodando", "Running", "En marcha"), true)),
            false,
        );
        let species = if target.species != 0 {
            target.species
        } else {
            self.hunter.last_species
        };
        self.sprite_pair(ui, species, 72.0);

        let secs = self.hunt_elapsed().as_secs_f64();
        let rate = if secs > 0.5 {
            self.hunter.attempts as f64 / secs
        } else {
            0.0
        };
        ui.columns(2, |cols| {
            stat_card(
                &mut cols[0],
                t("Tentativas", "Attempts", "Intentos"),
                &fmt_int(self.hunter.attempts),
                None,
                26.0,
            );
            stat_card(
                &mut cols[1],
                t("Tempo · ritmo", "Time · rate", "Tiempo · ritmo"),
                &fmt_time(self.hunt_elapsed()),
                Some(&format!(
                    "{} {}",
                    fmt_dec(rate, 1),
                    t("tentativa/s", "attempts/s", "intentos/s")
                )),
                18.0,
            );
        });

        let chance = self.hunt_chance();
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t(
                        "Chance acumulada (1 em 8192)",
                        "Cumulative chance (1 in 8192)",
                        "Probabilidad acumulada (1 en 8192)",
                    ))
                    .font(theme::regular(12.0))
                    .color(theme::MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{}%", fmt_dec(chance * 100.0, 1)))
                            .font(theme::bold(15.0))
                            .color(theme::ACCENT),
                    );
                });
            });
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
            ui.painter().rect_filled(bar, 4.0, theme::DIVIDER);
            let mut fill = bar;
            fill.set_width((bar.width() * chance as f32).max(8.0));
            ui.painter().rect_filled(fill, 4.0, theme::ACCENT);
            if self.hunter.best_shiny_value != 0xFFFF {
                ui.label(
                    RichText::new(format!(
                        "{} {} {} {} ({})",
                        t(
                            "Mais perto até agora:",
                            "Closest so far:",
                            "Lo más cerca hasta ahora:"
                        ),
                        self.hunter.best_shiny_value,
                        t("na tentativa", "on attempt", "en el intento"),
                        fmt_int(self.hunter.best_attempt),
                        t(
                            "shiny é abaixo de 8",
                            "shiny is below 8",
                            "shiny es menor que 8"
                        ),
                    ))
                    .font(theme::regular(12.0))
                    .color(theme::MUTED),
                );
            }
        });

        self.speed_card(ui);

        let arrow = if self.hunt_details { "▾" } else { "▸" };
        if widgets::text_link(
            ui,
            &format!(
                "{arrow} {}",
                t(
                    "Detalhes técnicos",
                    "Technical details",
                    "Detalles técnicos"
                )
            ),
        )
        .clicked()
        {
            self.hunt_details = !self.hunt_details;
        }
        if self.hunt_details && self.hunter.last_pid != 0 {
            egui::Frame::none()
                .fill(theme::BG_DEEP)
                .rounding(12.0)
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let line = |ui: &mut egui::Ui, s: String| {
                        ui.label(
                            RichText::new(s)
                                .font(theme::regular(12.0))
                                .color(theme::MUTED),
                        );
                    };
                    line(
                        ui,
                        format!(
                            "{}: {:08X}",
                            t("Último PID", "Last PID", "Último PID"),
                            self.hunter.last_pid
                        ),
                    );
                    line(
                        ui,
                        format!(
                            "{}: {} ({})",
                            t("Valor shiny", "Shiny value", "Valor shiny"),
                            self.hunter.last_shiny_value,
                            t("shiny se < 8", "shiny if < 8", "shiny si < 8")
                        ),
                    );
                    line(
                        ui,
                        format!(
                            "{}: {}",
                            t("Espécie lida", "Species read", "Especie leída"),
                            self.hunter.last_species
                        ),
                    );
                });
        }
        if crate::SHOW_RAM_FINDER {
            self.cursor_finder_ui(ui);
            self.battle_state_finder_ui(ui);
        }
    }

    /// Velocidade da caça (frames de emulação por atualização de tela). Fica
    /// em "Pronto" e durante a caça; começa em 1×, como antes do redesenho.
    fn speed_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, |ui| {
            let mut idx = HUNT_SPEEDS
                .iter()
                .position(|&s| s >= self.hunt_speed)
                .unwrap_or(HUNT_SPEEDS.len() - 1);
            let names = [
                "1× (normal)",
                "4×",
                "16×",
                "64×",
                t("Máxima", "Maximum", "Máxima"),
            ];
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("Velocidade da caça", "Hunt speed", "Velocidad de caza"))
                        .font(theme::semibold(13.0))
                        .color(theme::TEXT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(names[idx])
                            .font(theme::semibold(13.0))
                            .color(theme::ACCENT),
                    );
                });
            });
            ui.spacing_mut().slider_width = ui.available_width();
            if ui
                .add(egui::Slider::new(&mut idx, 0..=HUNT_SPEEDS.len() - 1).show_value(false))
                .changed()
            {
                self.hunt_speed = HUNT_SPEEDS[idx];
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("Dá pra acompanhar", "Watchable", "Se puede seguir"))
                        .font(theme::regular(11.0))
                        .color(theme::FAINT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(t("Máxima", "Maximum", "Máxima"))
                            .font(theme::regular(11.0))
                            .color(theme::FAINT),
                    );
                });
            });
        });
    }

    fn panel_found(&mut self, ui: &mut egui::Ui, target: &TargetDef) {
        let lang = i18n::get();
        let title = if target.kind() == TargetKind::Wild && target.species == 0 {
            t("Shiny selvagem", "Wild shiny", "Shiny salvaje").to_string()
        } else {
            format!("{} shiny", target.display_name(lang))
        };
        if self.panel_header(
            ui,
            Some(t("Encontrado", "Found", "Encontrado")),
            &title,
            None,
            None,
            true,
        ) {
            self.close_hunter();
            return;
        }
        let species = if target.species != 0 {
            target.species
        } else {
            self.hunter.last_species
        };
        let ctx = ui.ctx().clone();
        let tex = self.target_sprite(&ctx, species, true);
        egui::Frame::none()
            .fill(theme::ACCENT_SOFT)
            .stroke(Stroke::new(1.5_f32, theme::ACCENT))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(18.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    sprite_box(ui, tex.as_ref(), 128.0);
                    ui.label(
                        RichText::new(t(
                            "É este que está no seu jogo agora",
                            "This is the one in your game now",
                            "Es el que está en tu juego ahora",
                        ))
                        .font(theme::semibold(14.0))
                        .color(theme::ACCENT),
                    );
                });
            });
        ui.columns(3, |cols| {
            stat_card(
                &mut cols[0],
                t("Tentativas", "Attempts", "Intentos"),
                &fmt_int(self.hunter.attempts),
                None,
                18.0,
            );
            stat_card(
                &mut cols[1],
                t("Tempo", "Time", "Tiempo"),
                &fmt_time(self.hunt_elapsed()),
                None,
                18.0,
            );
            stat_card(
                &mut cols[2],
                t("Chance", "Chance", "Prob."),
                &format!("{}%", fmt_dec(self.hunt_chance() * 100.0, 1)),
                None,
                18.0,
            );
        });
        ui.label(
            RichText::new(t(
                "Salve o estado e depois guarde o Pokémon no jogo (salvar pelo menu do jogo). Fechar o painel não muda nada no jogo.",
                "Save a state, then keep the Pokémon in the game (save from the game's menu). Closing the panel doesn't change the game.",
                "Guarda un estado y luego conserva el Pokémon en el juego (guarda desde el menú del juego). Cerrar el panel no cambia el juego.",
            ))
            .font(theme::regular(13.0))
            .color(theme::MUTED),
        );
    }

    /// Sprites normal e shiny lado a lado (sem sprite: um "?").
    fn sprite_pair(&mut self, ui: &mut egui::Ui, species: u16, side: f32) {
        let ctx = ui.ctx().clone();
        let normal = self.target_sprite(&ctx, species, false);
        let shiny = self.target_sprite(&ctx, species, true);
        ui.columns(2, |cols| {
            for (col, tex, label, hot) in [
                (0, &normal, t("Normal", "Normal", "Normal"), false),
                (1, &shiny, "Shiny", true),
            ] {
                let ui = &mut cols[col];
                let mut frame = egui::Frame::none()
                    .fill(theme::SURFACE)
                    .rounding(14.0)
                    .inner_margin(egui::Margin::same(10.0));
                if hot {
                    frame = frame.stroke(Stroke::new(1.0_f32, theme::ACCENT.gamma_multiply(0.5)));
                }
                frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        sprite_box(ui, tex.as_ref(), side);
                        let rich = RichText::new(label).font(if hot {
                            theme::semibold(12.0)
                        } else {
                            theme::regular(12.0)
                        });
                        ui.label(if hot {
                            rich.color(theme::ACCENT)
                        } else {
                            rich.color(theme::MUTED)
                        });
                    });
                });
            }
        });
    }

    /// Aviso de shiny encontrado no topo da área do jogo (não pausa nada).
    pub(crate) fn shiny_banner(&mut self, ui: &mut egui::Ui, area: Rect) {
        if !self.hunter.found || self.hunting || !self.shiny_banner {
            return;
        }
        let Some(profile) = self.profile else {
            return;
        };
        let target = profile.targets[self.selected_target.unwrap_or(0)];
        let name = if target.kind() == TargetKind::Wild && target.species == 0 {
            "Pokémon"
        } else {
            target.display_name(i18n::get())
        };
        let w = 640.0_f32.min(area.width() - 24.0);
        let title = format!(
            "{name} shiny {} {}",
            t("na tentativa", "on attempt", "en el intento"),
            fmt_int(self.hunter.attempts)
        );
        let sub = t(
            "A caça parou e o jogo é seu. Salve antes de qualquer coisa.",
            "The hunt stopped and the game is yours. Save before anything else.",
            "La caza se detuvo y el juego es tuyo. Guarda antes que nada.",
        );
        // Botões à direita; se o texto ficaria espremido, eles descem pra uma
        // linha própria (janela estreita / painel do Hunter aberto).
        let save_label = t("Salvar estado", "Save state", "Guardar estado");
        let buttons_w = 36.0
            + 8.0
            + ui.painter()
                .layout_no_wrap(save_label.to_owned(), theme::bold(14.0), theme::TEXT)
                .size()
                .x
            + 32.0;
        let text_left = 50.0;
        let side_w = w - text_left - 12.0 - buttons_w - 12.0;
        let stacked = side_w < 220.0;
        let text_w = if stacked {
            w - text_left - 12.0
        } else {
            side_w
        };
        let wrap = |text: String, font: egui::FontId, color: Color32| {
            ui.fonts(|f| f.layout(text, font, color, text_w))
        };
        let title_g = wrap(title, theme::bold(15.0), theme::TEXT);
        let sub_g = wrap(sub.to_owned(), theme::regular(13.0), theme::MUTED);
        let text_h = title_g.size().y + 3.0 + sub_g.size().y;
        let h = if stacked {
            12.0 + text_h + 10.0 + 40.0 + 12.0
        } else {
            (12.0 + text_h + 12.0).max(64.0)
        };
        let rect = Rect::from_min_size(
            pos2(area.center().x - w / 2.0, area.top() + 12.0),
            vec2(w, h),
        );
        let p = ui.painter();
        p.rect_filled(rect, 14.0, theme::SURFACE);
        p.rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, theme::ACCENT.gamma_multiply(0.6)),
        );
        let text_top = if stacked {
            rect.top() + 12.0
        } else {
            rect.center().y - text_h / 2.0
        };
        icons::paint(
            p,
            Rect::from_center_size(pos2(rect.left() + 28.0, text_top + 10.0), vec2(22.0, 22.0)),
            Icon::Sparkle,
            theme::ACCENT,
        );
        let sub_top = text_top + title_g.size().y + 3.0;
        p.galley(
            pos2(rect.left() + text_left, text_top),
            title_g,
            theme::TEXT,
        );
        p.galley(pos2(rect.left() + text_left, sub_top), sub_g, theme::MUTED);
        let row = if stacked {
            Rect::from_min_max(
                pos2(rect.left() + 12.0, rect.bottom() - 12.0 - 40.0),
                rect.right_bottom() - vec2(12.0, 12.0),
            )
        } else {
            rect.shrink2(vec2(12.0, 12.0))
        };
        let mut buttons = ui.child_ui(row, egui::Layout::right_to_left(egui::Align::Center));
        let (close, resp) = buttons.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
        if resp.hovered() {
            buttons
                .painter()
                .circle_filled(close.center(), 18.0, theme::SURFACE2);
        }
        icons::paint(
            buttons.painter(),
            Rect::from_center_size(close.center(), vec2(16.0, 16.0)),
            Icon::Close,
            theme::MUTED,
        );
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.shiny_banner = false;
        }
        if widgets::accent_button(&mut buttons, save_label, 40.0).clicked() {
            self.open_states_to_save();
        }
    }

    fn open_states_to_save(&mut self) {
        self.states_open = true;
        self.states_load_mode = false;
        self.states_confirm = None;
    }
}

/// Sprite (ou "?" quando não há) num quadrado de `side`.
fn sprite_box(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, side: f32) {
    match tex {
        Some(tex) => {
            ui.add(egui::Image::new(tex).fit_to_exact_size(vec2(side, side)));
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            ui.painter()
                .rect_stroke(rect.shrink(1.0), 12.0, Stroke::new(1.0_f32, theme::OFF));
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "?",
                theme::bold(side * 0.35),
                theme::FAINT,
            );
        }
    }
}

/// Selo de status ("Parado" cinza / "Rodando" azul).
fn status_chip(ui: &mut egui::Ui, label: &str, running: bool) {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::semibold(12.0), Color32::WHITE);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 37.0, 28.0), Sense::hover());
    let (bg, dot, fg) = if running {
        (theme::ACCENT_SOFT, theme::ACCENT, theme::ACCENT)
    } else {
        (theme::SURFACE, theme::OFF, theme::MUTED)
    };
    let p = ui.painter();
    p.rect_filled(rect, 999.0, bg);
    p.circle_filled(pos2(rect.left() + 15.0, rect.center().y), 4.0, dot);
    p.galley(
        pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
}

/// Cartão tracejado de passo ("1 Escolha o que caçar").
fn step_card(ui: &mut egui::Ui, n: &str, title: &str, sub: &str) {
    let w = ui.available_width();
    let text_w = w - 16.0 - 40.0 - 12.0 - 16.0;
    let sub_galley = ui.fonts(|f| {
        let mut job = egui::text::LayoutJob::simple(
            sub.to_owned(),
            theme::regular(12.0),
            theme::MUTED,
            text_w,
        );
        job.wrap.max_rows = 4;
        f.layout_job(job)
    });
    let h = (14.0 + 19.0 + 3.0 + sub_galley.size().y + 14.0).max(68.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter();
    let c = rect.shrink(1.0);
    let pts = [
        c.left_top(),
        c.right_top(),
        c.right_bottom(),
        c.left_bottom(),
        c.left_top(),
    ];
    p.extend(egui::Shape::dashed_line(
        &pts,
        Stroke::new(1.5_f32, theme::BORDER),
        6.0,
        4.0,
    ));
    let circle = pos2(rect.left() + 16.0 + 20.0, rect.center().y);
    p.circle_filled(circle, 20.0, theme::SURFACE);
    p.text(
        circle,
        Align2::CENTER_CENTER,
        n,
        theme::bold(16.0),
        theme::ACCENT,
    );
    let x = rect.left() + 16.0 + 40.0 + 12.0;
    let top = rect.center().y - (19.0 + 3.0 + sub_galley.size().y) / 2.0;
    p.text(
        pos2(x, top),
        Align2::LEFT_TOP,
        title,
        theme::bold(15.0),
        theme::TEXT,
    );
    p.galley(pos2(x, top + 22.0), sub_galley, theme::MUTED);
}

/// Botão de alvo na lista (até duas linhas de nome).
fn target_chip(ui: &mut egui::Ui, name: &str, w: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    let galley = ui.fonts(|f| {
        let mut job = egui::text::LayoutJob::simple(
            name.to_owned(),
            theme::semibold(13.0),
            theme::TEXT,
            w - 20.0,
        );
        job.wrap.max_rows = 2;
        f.layout_job(job)
    });
    let p = ui.painter();
    p.rect_filled(
        rect,
        10.0,
        if resp.hovered() {
            theme::SURFACE2
        } else {
            theme::SURFACE
        },
    );
    if resp.hovered() {
        p.rect_stroke(rect.shrink(0.75), 10.0, Stroke::new(1.5_f32, theme::ACCENT));
    }
    p.galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::TEXT,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Cartão de número ("Tentativas 1.284").
fn stat_card(ui: &mut egui::Ui, label: &str, value: &str, sub: Option<&str>, size: f32) {
    egui::Frame::none()
        .fill(theme::SURFACE)
        .rounding(14.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(label)
                    .font(theme::regular(12.0))
                    .color(theme::MUTED),
            );
            ui.label(
                RichText::new(value)
                    .font(theme::bold(size))
                    .color(theme::TEXT),
            );
            if let Some(s) = sub {
                ui.label(
                    RichText::new(s)
                        .font(theme::regular(12.0))
                        .color(theme::MUTED),
                );
            }
        });
}

/// Botão quadrado "?" (Como usar).
fn help_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 46.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(
        rect,
        12.0,
        if resp.hovered() {
            theme::SURFACE2
        } else {
            Color32::TRANSPARENT
        },
    );
    p.rect_stroke(rect.shrink(0.5), 12.0, Stroke::new(1.0_f32, theme::BORDER));
    p.circle_stroke(rect.center(), 10.0, Stroke::new(2.0_f32, theme::TEXT));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "?",
        theme::bold(13.0),
        theme::TEXT,
    );
    resp.on_hover_text(t("Como usar", "How to use", "Cómo usar"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
