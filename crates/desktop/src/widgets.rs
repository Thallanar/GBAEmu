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
