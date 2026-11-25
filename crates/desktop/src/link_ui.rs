//! Janela do Link (cabo): parceiros achados na rede, hospedar ou conectar pelo
//! endereço; durante a conexão, um aviso com Cancelar; conectado, o papel e
//! Desconectar. A conexão roda em thread de fundo ([`crate::link::PendingLink`]),
//! então a UI nunca trava esperando o parceiro.

use eframe::egui::{self, vec2, RichText};

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::{discovery, host_name, link, theme, widgets, AuroraApp, LinkAction};

impl AuroraApp {
    pub(crate) fn link_window(&mut self, ctx: &egui::Context) {
        if !self.show_link {
            return;
        }
        // Espelha o estado em valores `Copy` pra não segurar `&self` enquanto a
        // UI roda; as mutações acontecem depois, via `action`.
        let connected_id = self.link.as_ref().map(|s| s.id);
        let pending_hosting = self.link_pending.as_ref().map(|p| p.hosting);
        let peers = self
            .discovery
            .as_ref()
            .map(|d| d.peers())
            .unwrap_or_default();
        let discovery_on = self.discovery.is_some();
        let mut action: Option<LinkAction> = None;
        let mut close = false;

        let height = if connected_id.is_some() || pending_hosting.is_some() {
            230.0
        } else {
            370.0 + 60.0 * peers.len() as f32
        };
        let veil_close = widgets::modal(ctx, "link", vec2(720.0, height), true, |ui| {
            let inner = ui.max_rect().shrink2(vec2(24.0, 20.0));
            let mut ui = ui.child_ui(inner, egui::Layout::top_down(egui::Align::Min));
            ui.spacing_mut().item_spacing.y = 14.0;

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        RichText::new(t("Link (cabo)", "Link (cable)", "Link (cable)"))
                            .font(theme::bold(22.0))
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new(t(
                            "Troca e batalha com outro AuroraGBA na mesma rede",
                            "Trade and battle with another AuroraGBA on the same network",
                            "Intercambia y combate con otro AuroraGBA en la misma red",
                        ))
                        .font(theme::regular(13.0))
                        .color(theme::MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::close_button(ui).clicked() {
                        close = true;
                    }
                });
            });

            if let Some(id) = connected_id {
                banner(&mut ui, |ui| {
                    widgets::titled(
                        ui,
                        t("Conectado", "Connected", "Conectado"),
                        Some(if id == 0 {
                            t(
                                "Você hospeda (gera o relógio do cabo). Agora é só usar o link dentro do jogo.",
                                "You're the host (you drive the cable clock). Now just use the link inside the game.",
                                "Eres el anfitrión (generas el reloj del cable). Ahora solo usa el link dentro del juego.",
                            )
                        } else {
                            t(
                                "Você é o convidado (o outro lado gera o relógio). Agora é só usar o link dentro do jogo.",
                                "You're the guest (the other side drives the clock). Now just use the link inside the game.",
                                "Eres el invitado (el otro lado genera el reloj). Ahora solo usa el link dentro del juego.",
                            )
                        }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::danger_button(
                            ui,
                            t("Desconectar", "Disconnect", "Desconectar"),
                            40.0,
                        )
                        .clicked()
                        {
                            action = Some(LinkAction::Disconnect);
                        }
                    });
                });
                return;
            }

            if let Some(hosting) = pending_hosting {
                banner(&mut ui, |ui| {
                    ui.add(egui::Spinner::new().size(20.0).color(theme::ACCENT));
                    let sub =
                        if hosting {
                            format!(
                            "{} {}. {}",
                            t("Ouvindo na porta", "Listening on port", "Escuchando en el puerto"),
                            self.link_port.trim(),
                            t(
                                "Você aparece na lista do parceiro; se não, passe o seu IP.",
                                "You show up in your partner's list; if not, give them your IP.",
                                "Apareces en la lista del compañero; si no, pásale tu IP.",
                            )
                        )
                        } else {
                            self.link_addr.trim().to_string()
                        };
                    widgets::titled(
                        ui,
                        if hosting {
                            t(
                                "Esperando o parceiro conectar…",
                                "Waiting for your partner…",
                                "Esperando al compañero…",
                            )
                        } else {
                            t("Conectando…", "Connecting…", "Conectando…")
                        },
                        Some(&sub),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::outline_button(ui, t("Cancelar", "Cancel", "Cancelar"), 40.0)
                            .clicked()
                        {
                            action = Some(LinkAction::Cancel);
                        }
                    });
                });
                return;
            }

            // Ocioso: parceiros na rede + hospedar / conectar pelo endereço.
            widgets::section_label(
                &mut ui,
                t(
                    "Parceiros na rede",
                    "Partners on the network",
                    "Compañeros en la red",
                ),
            );
            egui::Frame::none()
                .fill(theme::SURFACE)
                .rounding(14.0)
                .show(&mut ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for peer in &peers {
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 60.0),
                            egui::Sense::click(),
                        );
                        let p = ui.painter();
                        if resp.hovered() {
                            p.rect_filled(rect, 14.0, theme::SURFACE2);
                        }
                        icons::paint(
                            p,
                            egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 26.0, rect.center().y),
                                vec2(20.0, 20.0),
                            ),
                            Icon::Link,
                            theme::ACCENT,
                        );
                        p.text(
                            egui::pos2(rect.left() + 48.0, rect.center().y - 9.0),
                            egui::Align2::LEFT_CENTER,
                            &peer.name,
                            theme::semibold(15.0),
                            theme::TEXT,
                        );
                        p.text(
                            egui::pos2(rect.left() + 48.0, rect.center().y + 10.0),
                            egui::Align2::LEFT_CENTER,
                            peer.addr.to_string(),
                            theme::regular(12.0),
                            theme::MUTED,
                        );
                        p.text(
                            egui::pos2(rect.right() - 16.0, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            t("Conectar", "Connect", "Conectar"),
                            theme::bold(13.0),
                            theme::ACCENT,
                        );
                        p.hline(
                            rect.x_range(),
                            rect.bottom(),
                            egui::Stroke::new(1.0_f32, theme::DIVIDER),
                        );
                        if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            action = Some(LinkAction::JoinAddr(peer.addr.to_string()));
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let msg = if !discovery_on {
                            t(
                                "Busca na rede indisponível: use o endereço abaixo.",
                                "Network search unavailable: use the address below.",
                                "Búsqueda en la red no disponible: usa la dirección de abajo.",
                            )
                        } else {
                            t(
                                "Procurando na rede local…",
                                "Looking on the local network…",
                                "Buscando en la red local…",
                            )
                        };
                        ui.add_sized(
                            vec2(ui.available_width() - 16.0, 40.0),
                            egui::Label::new(
                                RichText::new(msg)
                                    .font(theme::regular(12.0))
                                    .color(theme::FAINT),
                            ),
                        );
                    });
                });

            ui.columns(2, |cols| {
                widgets::card(&mut cols[0], |ui| {
                    widgets::titled(
                        ui,
                        t("Hospedar", "Host", "Hospedar"),
                        Some(t(
                            "Você aparece pros outros na lista acima e gera o relógio do cabo.",
                            "You show up in others' list above and drive the cable clock.",
                            "Apareces en la lista de los demás y generas el reloj del cable.",
                        )),
                    );
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(t("Porta", "Port", "Puerto")).color(theme::MUTED));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.link_port)
                                .desired_width(70.0)
                                .margin(vec2(8.0, 7.0)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::accent_button(ui, t("Hospedar", "Host", "Hospedar"), 36.0)
                                .clicked()
                            {
                                action = Some(LinkAction::Host);
                            }
                        });
                    });
                });
                widgets::card(&mut cols[1], |ui| {
                    widgets::titled(
                        ui,
                        t(
                            "Conectar pelo endereço",
                            "Connect by address",
                            "Conectar por dirección",
                        ),
                        Some(t(
                            "Quando o parceiro não aparece na lista.",
                            "When your partner isn't in the list.",
                            "Cuando el compañero no aparece en la lista.",
                        )),
                    );
                    ui.horizontal(|ui| {
                        let button_w = 140.0;
                        ui.add(
                            egui::TextEdit::singleline(&mut self.link_addr)
                                .desired_width(ui.available_width() - button_w)
                                .margin(vec2(8.0, 7.0)),
                        );
                        if widgets::outline_button(ui, t("Conectar", "Connect", "Conectar"), 36.0)
                            .clicked()
                        {
                            action = Some(LinkAction::Join);
                        }
                    });
                });
            });
            ui.label(
                RichText::new(t(
                    "Os dois lados precisam estar na mesma rede.",
                    "Both sides need to be on the same network.",
                    "Ambos lados deben estar en la misma red.",
                ))
                .font(theme::regular(12.0))
                .color(theme::FAINT),
            );
        });
        if close || veil_close {
            self.show_link = false;
        }

        match action {
            Some(LinkAction::Host) => match self.link_port.trim().parse::<u16>() {
                Ok(port) => {
                    self.link_pending = Some(link::PendingLink::host(port));
                    // Anuncia o host na LAN pra os parceiros nos acharem sem IP
                    // (best-effort: se o broadcast falhar, o manual ainda vale).
                    self.link_advertiser = discovery::Advertiser::start(port, host_name())
                        .map_err(|e| log::info!("não consegui anunciar o link na LAN: {e}"))
                        .ok();
                }
                Err(_) => self.set_status(t("Porta inválida", "Invalid port", "Puerto no válido")),
            },
            Some(LinkAction::Join) => {
                let addr = self.link_addr.trim().to_string();
                if addr.is_empty() {
                    self.set_status(t(
                        "Digite o endereço do parceiro",
                        "Type your partner's address",
                        "Escribe la dirección del compañero",
                    ));
                } else {
                    self.link_pending = Some(link::PendingLink::join(addr));
                }
            }
            Some(LinkAction::JoinAddr(addr)) => {
                self.link_addr = addr.clone();
                self.link_pending = Some(link::PendingLink::join(addr));
            }
            Some(LinkAction::Cancel) => {
                if let Some(p) = &self.link_pending {
                    p.cancel();
                }
                self.link_pending = None;
                self.link_advertiser = None; // para de anunciar
                self.set_status(t(
                    "Conexão de link cancelada",
                    "Link connection cancelled",
                    "Conexión de link cancelada",
                ));
            }
            Some(LinkAction::Disconnect) => {
                self.link = None;
                self.gba.link_configure(false, 0);
                self.set_status(t("Link encerrado", "Link closed", "Link cerrado"));
            }
            None => {}
        }
    }
}

/// Faixa azul de status (conectado / conectando), com conteúdo em linha.
fn banner(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(theme::ACCENT_SOFT)
        .stroke(egui::Stroke::new(
            1.0_f32,
            theme::ACCENT.gamma_multiply(0.45),
        ))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(add);
        });
}
