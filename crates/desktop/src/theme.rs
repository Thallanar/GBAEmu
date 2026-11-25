//! Visual do app: paleta (a mesma do canvas de design e do Android), fonte
//! Inter embutida em três pesos e o tema escuro do egui.

use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke};

/// Fundo da área do jogo (mais escuro que as superfícies).
pub const GAME_BG: Color32 = Color32::from_rgb(0x0C, 0x0C, 0x0F);
pub const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x16);
pub const STATUS_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x18);
pub const SURFACE: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x22);
pub const SURFACE2: Color32 = Color32::from_rgb(0x26, 0x26, 0x2E);
pub const LINE: Color32 = Color32::from_rgb(0x26, 0x26, 0x2E);
pub const BORDER: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x45);
pub const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF5);
pub const TEXT_SOFT: Color32 = Color32::from_rgb(0xD6, 0xD6, 0xDD);
pub const MUTED: Color32 = Color32::from_rgb(0xA3, 0xA3, 0xAD);
pub const FAINT: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x96);
pub const OFF: Color32 = Color32::from_rgb(0x5A, 0x5A, 0x66);
pub const ACCENT: Color32 = Color32::from_rgb(0x78, 0xC8, 0xFF);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0B, 0x22, 0x33);
/// Destaque suave (fundo de item ativo): o azul a ~16% sobre o escuro.
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0x13, 0x20, 0x29, 0x29);

const SEMIBOLD: &str = "inter-semibold";
const BOLD: &str = "inter-bold";

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// Instala fontes e visual. Chamado uma vez na criação do app.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_visuals(visuals());
    ctx.style_mut(|s| {
        use egui::TextStyle;
        s.text_styles = [
            (TextStyle::Heading, bold(20.0)),
            (TextStyle::Body, regular(14.0)),
            (TextStyle::Button, semibold(14.0)),
            (TextStyle::Small, regular(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 220.0;
    });
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 3] = [
        ("inter", include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        (
            SEMIBOLD,
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
        ),
        (BOLD, include_bytes!("../assets/fonts/Inter-Bold.ttf")),
    ];
    for (name, bytes) in faces {
        fonts
            .font_data
            .insert(name.to_owned(), egui::FontData::from_static(bytes));
    }
    // A Inter na frente; as fontes padrão do egui (com símbolos e emoji) seguem
    // como reserva pros glifos que ela não tem.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "inter".to_owned());
    for name in [SEMIBOLD, BOLD] {
        let mut list = vec![name.to_owned()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), list);
    }
    ctx.set_fonts(fonts);
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x33, 0x33, 0x3D));
    v.window_rounding = Rounding::same(16.0);
    v.menu_rounding = Rounding::same(10.0);
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = SURFACE;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(0x2B, 0x4A, 0x60);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.slider_trailing_fill = true;

    let r = Rounding::same(10.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BG;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_SOFT);
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = SURFACE2;
    w.inactive.weak_bg_fill = SURFACE2;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = r;

    w.hovered.bg_fill = Color32::from_rgb(0x30, 0x30, 0x3A);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x30, 0x30, 0x3A);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.hovered.rounding = r;

    w.active.bg_fill = Color32::from_rgb(0x38, 0x38, 0x44);
    w.active.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x44);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.active.rounding = r;

    w.open = w.active;
    v
}
