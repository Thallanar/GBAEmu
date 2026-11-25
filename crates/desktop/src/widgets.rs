//! Componentes do design desenhados à mão (o egui padrão não tem o visual do
//! canvas): botão da barra de ferramentas, botão de destaque e com contorno.

use eframe::egui::{self, vec2, Align2, Color32, Rect, Response, Sense, Stroke};

use crate::icons::{self, Icon};
use crate::theme;

/// Botão da barra de ferramentas: ícone + nome (ou só o ícone, com o nome na
/// dica, quando `compact`). `active` = alternador ligado / painel aberto.
pub fn tool_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
    compact: bool,
) -> Response {
    let font = theme::semibold(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::WHITE);
    let icon_side = 18.0;
    let width = if compact {
        40.0
    } else {
        10.0 + icon_side + 7.0 + galley.size().x + 10.0
    };
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let fg = if !enabled {
            theme::OFF
        } else if active {
            theme::ACCENT
        } else if hovered {
            theme::TEXT
        } else {
            theme::TEXT_SOFT
        };
        let bg = if active {
            theme::ACCENT_SOFT
        } else if hovered {
            theme::SURFACE2
        } else {
            Color32::TRANSPARENT
        };
        let p = ui.painter();
        p.rect_filled(rect, 10.0, bg);
        let icon_x = if compact {
            rect.center().x - icon_side / 2.0
        } else {
            rect.left() + 10.0
        };
        let icon_rect = Rect::from_min_size(
            egui::pos2(icon_x, rect.center().y - icon_side / 2.0),
            vec2(icon_side, icon_side),
        );
        icons::paint(p, icon_rect, icon, fg);
        if !compact {
            let pos = egui::pos2(
                icon_rect.right() + 7.0,
                rect.center().y - galley.size().y / 2.0,
            );
            p.galley_with_override_text_color(pos, galley, fg);
        }
    }
    let resp = if compact {
        resp.on_hover_text(label)
    } else {
        resp
    };
    resp.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    })
}

/// Largura que [`tool_button`] ocupa com o nome visível.
pub fn tool_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), theme::semibold(13.0), Color32::WHITE);
    10.0 + 18.0 + 7.0 + galley.size().x + 10.0
}

/// Divisória vertical entre grupos da barra.
pub fn tool_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0_f32, egui::Color32::from_rgb(0x33, 0x33, 0x3D)),
    );
}

/// Botão cheio na cor de destaque (ação principal).
pub fn accent_button(ui: &mut egui::Ui, label: &str, height: f32) -> Response {
    filled_button(ui, label, height, theme::ACCENT, theme::ON_ACCENT, None)
}

/// Botão só com contorno (ação secundária).
pub fn outline_button(ui: &mut egui::Ui, label: &str, height: f32) -> Response {
    filled_button(
        ui,
        label,
        height,
        Color32::TRANSPARENT,
        theme::TEXT,
        Some(theme::BORDER),
    )
}

fn filled_button(
    ui: &mut egui::Ui,
    label: &str,
    height: f32,
    bg: Color32,
    fg: Color32,
    border: Option<Color32>,
) -> Response {
    let font = theme::bold(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
    let size = vec2(galley.size().x + 32.0, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if resp.hovered() && bg != Color32::TRANSPARENT {
            bg.gamma_multiply(0.9)
        } else if resp.hovered() {
            theme::SURFACE2
        } else {
            bg
        };
        p.rect_filled(rect, 10.0, fill);
        if let Some(b) = border {
            p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, b));
        }
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            theme::bold(14.0),
            fg,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Janela modal centralizada sobre um véu escuro que cobre (e bloqueia) o resto
/// do app. Devolve `true` se o usuário pediu pra fechar: clique fora ou Esc.
pub fn modal(
    ctx: &egui::Context,
    id: &str,
    size: egui::Vec2,
    esc_closes: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let screen = ctx.screen_rect();
    let mut close = false;
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let veil = ui.allocate_rect(screen, Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(158));
            let size = size.min(screen.size() - vec2(32.0, 32.0));
            let rect = Rect::from_center_size(screen.center(), size);
            ui.painter().rect_filled(rect, 20.0, theme::BG);
            ui.painter().rect_stroke(
                rect,
                20.0,
                Stroke::new(1.0_f32, Color32::from_rgb(0x33, 0x33, 0x3D)),
            );
            let outside = veil.clicked()
                && veil
                    .interact_pointer_pos()
                    .is_some_and(|p| !rect.contains(p));
            let mut child = ui.child_ui(rect, egui::Layout::top_down(egui::Align::Min));
            child.set_clip_rect(rect);
            add_contents(&mut child);
            close = outside || (esc_closes && ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        });
    close
}

/// Botão redondo de fechar (×) do canto das janelas.
pub fn close_button(ui: &mut egui::Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let bg = if resp.hovered() {
        theme::SURFACE2
    } else {
        theme::SURFACE
    };
    ui.painter().circle_filled(rect.center(), 20.0, bg);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
        Icon::Close,
        theme::TEXT,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Cartão (superfície arredondada) com conteúdo.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(theme::SURFACE)
        .rounding(14.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Título + subtítulo empilhados (subtítulo opcional, cinza).
pub fn titled(ui: &mut egui::Ui, title: &str, sub: Option<&str>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.label(
            egui::RichText::new(title)
                .font(theme::semibold(15.0))
                .color(theme::TEXT),
        );
        if let Some(sub) = sub {
            ui.label(
                egui::RichText::new(sub)
                    .font(theme::regular(13.0))
                    .color(theme::MUTED),
            );
        }
    });
}

/// Rótulo de seção em caixa alta (ex.: "TERCEIROS").
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(theme::semibold(12.0))
            .color(theme::MUTED),
    );
}

/// Switch 52×32 (pílula com pino). Alterna `on` ao clicar.
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(52.0, 32.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let p = ui.painter();
    let track = if *on {
        theme::ACCENT
    } else {
        Color32::from_rgb(0x4A, 0x4A, 0x55)
    };
    p.rect_filled(rect, 16.0, track);
    let x = egui::lerp(rect.left() + 16.0..=rect.right() - 16.0, t);
    let knob = if *on { theme::ON_ACCENT } else { theme::TEXT };
    p.circle_filled(egui::pos2(x, rect.center().y), 13.0, knob);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Seletor segmentado: escreve em `selected` o índice clicado. Devolve true se
/// mudou.
pub fn segmented(ui: &mut egui::Ui, selected: &mut usize, options: &[&str], width: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
    ui.painter().rect_filled(rect, 10.0, theme::BG);
    let inner = rect.shrink(4.0);
    let w = inner.width() / options.len() as f32;
    let mut changed = false;
    for (i, label) in options.iter().enumerate() {
        let r = Rect::from_min_size(inner.min + vec2(w * i as f32, 0.0), vec2(w, inner.height()));
        let resp = ui
            .interact(r, ui.id().with(("seg", label)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = *selected == i;
        if on {
            ui.painter().rect_filled(r, 7.0, theme::SURFACE2);
        }
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            theme::semibold(13.0),
            if on { theme::TEXT } else { theme::MUTED },
        );
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// Botão de opção (um entre vários): borda e texto azuis quando escolhido.
pub fn option_button(ui: &mut egui::Ui, label: &str, selected: bool, size: egui::Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let (bg, border, fg) = if selected {
        (theme::ACCENT_SOFT, theme::ACCENT, theme::ACCENT)
    } else if resp.hovered() {
        (
            Color32::from_rgb(0x30, 0x30, 0x3A),
            theme::BORDER,
            theme::TEXT,
        )
    } else {
        (theme::SURFACE2, theme::DIVIDER, theme::TEXT)
    };
    p.rect_filled(rect, 10.0, bg);
    p.rect_stroke(rect.shrink(0.75), 10.0, Stroke::new(1.5_f32, border));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::semibold(13.0),
        fg,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Item da navegação lateral (seção das Configurações).
pub fn nav_item(ui: &mut egui::Ui, label: &str, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, 10.0, theme::ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 10.0, theme::SURFACE);
    }
    p.text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::semibold(14.0),
        if active {
            theme::ACCENT
        } else {
            theme::TEXT_SOFT
        },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Texto clicável na cor de destaque (ação leve, tipo "Trocar pasta…").
pub fn text_link(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .font(theme::semibold(13.0))
                .color(theme::ACCENT),
        )
        .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Linha "título/subtítulo à esquerda, controle à direita" dentro de um cartão.
pub fn row<R>(
    ui: &mut egui::Ui,
    title: &str,
    sub: Option<&str>,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        titled(ui, title, sub);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control)
            .inner
    })
    .inner
}
