//! AuroraGBA — frontend desktop (Linux/Windows).

use eframe::egui;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("AuroraGBA"),
        ..Default::default()
    };

    eframe::run_native(
        "AuroraGBA",
        options,
        Box::new(|_cc| Box::new(AuroraApp::default())),
    )
}

#[derive(Default)]
struct AuroraApp {
    rom_path: Option<String>,
}

impl eframe::App for AuroraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Arquivo", |ui| {
                    if ui.button("Abrir ROM…").clicked() {
                        // TODO: file picker
                        ui.close_menu();
                    }
                    if ui.button("Sair").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Shiny Hunter", |ui| {
                    if ui.button("Iniciar caça").clicked() {
                        ui.close_menu();
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("🌌 AuroraGBA");
            ui.label("Emulador de Game Boy Advance — versão 0.1.0");
            if let Some(path) = &self.rom_path {
                ui.label(format!("ROM: {path}"));
            } else {
                ui.label("Nenhuma ROM carregada.");
            }
        });
    }
}
