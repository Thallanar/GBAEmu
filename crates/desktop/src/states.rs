//! Estados: janela com os 8 slots em grade, cada um com miniatura e data (como
//! no Android). A miniatura é um PNG ao lado do estado (`<rom>.ssN.png`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use auroragba_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use chrono::{DateTime, Local};
use eframe::egui::{self, vec2, Align2, Color32, RichText, Sense, Stroke};

use crate::i18n::{self, t, Lang};
use crate::{theme, widgets, AuroraApp, SAVE_SLOTS};

/// Caminho da miniatura de um estado: o arquivo do estado + `.png`.
pub fn thumb_path(state: &Path) -> PathBuf {
    let mut s = state.as_os_str().to_owned();
    s.push(".png");
    PathBuf::from(s)
}

/// "Hoje, 14:32", "Ontem, 21:10", "12/11, 09:45" (no idioma da interface).
fn slot_date(when: SystemTime) -> String {
    let dt: DateTime<Local> = when.into();
    let day = dt.date_naive();
    let today = Local::now().date_naive();
    let hm = dt.format("%H:%M");
    if day == today {
        format!("{}, {hm}", t("Hoje", "Today", "Hoy"))
    } else if Some(day) == today.pred_opt() {
        format!("{}, {hm}", t("Ontem", "Yesterday", "Ayer"))
    } else if i18n::get() == Lang::En {
        dt.format("%b %-d, %H:%M").to_string()
    } else {
        dt.format("%d/%m, %H:%M").to_string()
    }
}

impl AuroraApp {
    /// Grava a miniatura (o framebuffer atual) ao lado do estado do `slot`.
    pub(crate) fn save_slot_thumb(&mut self, slot: usize) {
        let Some(path) = self.slot_path(slot) else {
            return;
        };
        let png = crate::png::encode_rgba(
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
            &self.gba.bus.ppu.framebuffer[..],
        );
        if let Err(e) = std::fs::write(thumb_path(&path), png) {
            log::warn!("miniatura do estado não gravada: {e}");
        }
        self.thumb_cache.remove(&slot);
    }

    /// Data do estado e a miniatura (textura, cacheada pela data do arquivo).
    fn slot_info(
        &mut self,
        ctx: &egui::Context,
        slot: usize,
    ) -> Option<(SystemTime, Option<egui::TextureHandle>)> {
        let path = self.slot_path(slot)?;
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        if let Some((at, tex)) = self.thumb_cache.get(&slot) {
            if *at == modified {
                return Some((modified, tex.clone()));
            }
        }
        let tex = image::open(thumb_path(&path)).ok().map(|img| {
            let img = img.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            ctx.load_texture(
                format!("slot-thumb-{slot}"),
                color,
                egui::TextureOptions::NEAREST,
            )
        });
        self.thumb_cache.insert(slot, (modified, tex.clone()));
        Some((modified, tex))
    }

    /// Desenha a janela de Estados (se aberta).
    pub(crate) fn states_window(&mut self, ctx: &egui::Context) {
        if !self.states_open {
            return;
        }
        let infos: Vec<_> = (0..SAVE_SLOTS).map(|i| self.slot_info(ctx, i)).collect();
        let mut clicked: Option<usize> = None;
        let mut confirmed = false;
        let mut close = false;
        let esc = self.states_confirm.is_none();

        let veil_close = widgets::modal(ctx, "states", vec2(820.0, 510.0), esc, |ui| {
            let inner = ui.max_rect().shrink2(vec2(24.0, 20.0));
            let mut ui = ui.child_ui(inner, egui::Layout::top_down(egui::Align::Min));
            ui.spacing_mut().item_spacing.y = 14.0;

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t("Estados", "Save states", "Estados"))
                        .font(theme::bold(22.0))
                        .color(theme::TEXT),
                );
                ui.add_space(12.0);
                let mut mode = usize::from(self.states_load_mode);
                let opts = [
                    t("Salvar", "Save", "Guardar"),
                    t("Carregar", "Load", "Cargar"),
                ];
                if widgets::segmented(ui, &mut mode, &opts, 260.0) {
                    self.states_load_mode = mode == 1;
                    self.states_confirm = None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::close_button(ui).clicked() {
                        close = true;
                    }
                });
            });

            // Linha de ajuda, ou a confirmação de sobrescrever.
            match self.states_confirm {
                Some(slot) => {
                    ui.horizontal(|ui| {
                        let when = infos[slot]
                            .as_ref()
                            .map(|(m, _)| slot_date(*m))
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!(
                                "{} {} ({when})?",
                                t(
                                    "Substituir o slot",
                                    "Overwrite slot",
                                    "¿Reemplazar la ranura"
                                ),
                                slot + 1
                            ))
                            .font(theme::semibold(14.0))
                            .color(theme::TEXT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::accent_button(
                                ui,
                                t("Substituir", "Overwrite", "Reemplazar"),
                                36.0,
                            )
                            .clicked()
                            {
                                confirmed = true;
                            }
                            if widgets::outline_button(
                                ui,
                                t("Cancelar", "Cancel", "Cancelar"),
                                36.0,
                            )
                            .clicked()
                            {
                                self.states_confirm = None;
                            }
                        });
                    });
                }
                None => {
                    ui.label(
                        RichText::new(if self.states_load_mode {
                            t(
                                "Clique num slot para voltar àquele momento. O progresso desde o último save do jogo se perde.",
                                "Click a slot to go back to that moment. Progress since the game's last save is lost.",
                                "Haz clic en una ranura para volver a ese momento. Se pierde el progreso desde la última partida guardada.",
                            )
                        } else {
                            t(
                                "Clique num slot para salvar. Num slot ocupado, o app pede confirmação antes de sobrescrever.",
                                "Click a slot to save. On a used slot, the app asks before overwriting.",
                                "Haz clic en una ranura para guardar. En una ranura ocupada, la app pide confirmación antes de reemplazar.",
                            )
                        })
                        .font(theme::regular(13.0))
                        .color(theme::MUTED),
                    );
                }
            }

            // Grade 4×2.
            let gap = 14.0;
            let w = (ui.available_width() - 3.0 * gap) / 4.0;
            let thumb_h = (w - 16.0) * 2.0 / 3.0;
            let h = 8.0 + thumb_h + 10.0 + 20.0 + 8.0;
            for row in 0..2 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (slot, info) in infos.iter().enumerate().skip(row * 4).take(4) {
                        let enabled = !self.states_load_mode || info.is_some();
                        let sense = if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        };
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), sense);
                        let p = ui.painter();
                        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
                        let hot = enabled && (resp.hovered() || self.states_confirm == Some(slot));
                        p.rect_filled(rect, 14.0, dim(theme::SURFACE));
                        if hot {
                            p.rect_stroke(
                                rect.shrink(1.0),
                                14.0,
                                Stroke::new(2.0_f32, theme::ACCENT),
                            );
                        }
                        let thumb = egui::Rect::from_min_size(
                            rect.min + vec2(8.0, 8.0),
                            vec2(w - 16.0, thumb_h),
                        );
                        match info {
                            Some((_, Some(tex))) => {
                                egui::Image::new(tex)
                                    .rounding(8.0)
                                    .tint(dim(Color32::WHITE))
                                    .paint_at(ui, thumb);
                            }
                            Some((_, None)) => {
                                p.rect_filled(thumb, 8.0, Color32::from_rgb(0x1E, 0x2A, 0x36));
                            }
                            None => {
                                let c = thumb.shrink(1.0);
                                let pts = [
                                    c.left_top(),
                                    c.right_top(),
                                    c.right_bottom(),
                                    c.left_bottom(),
                                    c.left_top(),
                                ];
                                p.extend(egui::Shape::dashed_line(
                                    &pts,
                                    Stroke::new(1.5_f32, dim(theme::BORDER)),
                                    6.0,
                                    4.0,
                                ));
                                p.text(
                                    thumb.center(),
                                    Align2::CENTER_CENTER,
                                    t("Vazio", "Empty", "Vacía"),
                                    theme::regular(13.0),
                                    dim(theme::FAINT),
                                );
                            }
                        }
                        if slot == self.current_slot {
                            let badge = egui::Rect::from_min_size(
                                thumb.min + vec2(6.0, 6.0),
                                vec2(52.0, 18.0),
                            );
                            p.rect_filled(badge, 6.0, theme::ACCENT);
                            p.text(
                                badge.center(),
                                Align2::CENTER_CENTER,
                                "F5 / F9",
                                theme::bold(11.0),
                                theme::ON_ACCENT,
                            );
                        }
                        let y = thumb.bottom() + 10.0 + 10.0;
                        p.text(
                            egui::pos2(rect.left() + 10.0, y),
                            Align2::LEFT_CENTER,
                            format!("{} {}", t("Slot", "Slot", "Ranura"), slot + 1),
                            theme::semibold(14.0),
                            dim(theme::TEXT),
                        );
                        if let Some((m, _)) = info {
                            p.text(
                                egui::pos2(rect.right() - 10.0, y),
                                Align2::RIGHT_CENTER,
                                slot_date(*m),
                                theme::regular(12.0),
                                theme::MUTED,
                            );
                        }
                        if resp
                            .on_hover_cursor(if enabled {
                                egui::CursorIcon::PointingHand
                            } else {
                                egui::CursorIcon::NotAllowed
                            })
                            .clicked()
                        {
                            clicked = Some(slot);
                        }
                    }
                });
            }

            ui.label(
                RichText::new(t(
                    "O slot marcado com F5 / F9 é o dos atalhos: segue o último que você usou aqui.",
                    "The slot marked F5 / F9 is the shortcuts' slot: it follows the last one you used here.",
                    "La ranura marcada con F5 / F9 es la de los atajos: sigue la última que usaste aquí.",
                ))
                .font(theme::regular(12.0))
                .color(theme::FAINT),
            );
        });

        if let Some(slot) = clicked {
            if self.states_load_mode {
                self.current_slot = slot;
                self.load_state_slot(slot);
                close = true;
            } else if infos[slot].is_some() {
                self.states_confirm = Some(slot);
            } else {
                self.current_slot = slot;
                self.save_state_slot(slot);
                close = true;
            }
        }
        if confirmed {
            if let Some(slot) = self.states_confirm.take() {
                self.current_slot = slot;
                self.save_state_slot(slot);
                close = true;
            }
        }
        if close || veil_close {
            self.states_open = false;
            self.states_confirm = None;
        }
    }
}
