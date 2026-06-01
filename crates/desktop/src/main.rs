//! AuroraGBA — frontend desktop (Linux/Windows).
//!
//! Roda 1 frame por update da UI e exibe o framebuffer da PPU numa textura
//! 240×160 escalada.

use std::path::PathBuf;

use auroragba_core::joypad::Button;
use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};
use eframe::egui;

/// Mapeamento teclado → botões do GBA.
const KEY_MAP: &[(egui::Key, Button)] = &[
    (egui::Key::Z, Button::A),
    (egui::Key::X, Button::B),
    (egui::Key::Enter, Button::Start),
    (egui::Key::Backspace, Button::Select),
    (egui::Key::ArrowUp, Button::Up),
    (egui::Key::ArrowDown, Button::Down),
    (egui::Key::ArrowLeft, Button::Left),
    (egui::Key::ArrowRight, Button::Right),
    (egui::Key::A, Button::L),
    (egui::Key::S, Button::R),
];

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_title("AuroraGBA"),
        ..Default::default()
    };

    eframe::run_native(
        "AuroraGBA",
        options,
        Box::new(|cc| Box::new(AuroraApp::new(cc))),
    )
}

struct AuroraApp {
    gba: Gba,
    rom_path: Option<PathBuf>,
    texture: egui::TextureHandle,
    running: bool,
    scale: f32,
}

impl AuroraApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let image = egui::ColorImage::new([SCREEN_WIDTH, SCREEN_HEIGHT], egui::Color32::BLACK);
        let texture = cc
            .egui_ctx
            .load_texture("gba-framebuffer", image, egui::TextureOptions::NEAREST);

        Self {
            gba: Gba::new(),
            rom_path: None,
            texture,
            running: false,
            scale: 3.0,
        }
    }

    fn open_rom(&mut self, path: PathBuf) {
        match std::fs::read(&path) {
            Ok(rom) => {
                self.gba = Gba::new();
                self.gba.load_rom(rom);
                // Direct boot: estado pós-BIOS (modo System, SPs configurados),
                // entrada na ROM em 0x08000000. Os SWI são tratados por HLE.
                self.gba.cpu.setup_direct_boot();
                self.gba.cpu.regs.set_pc(0x0800_0000);
                self.rom_path = Some(path);
                self.running = true;
            }
            Err(e) => log::error!("Falha ao abrir ROM: {e}"),
        }
    }

    /// Lê o teclado e atualiza o estado dos botões do GBA.
    fn poll_input(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            for (key, button) in KEY_MAP {
                self.gba.bus.io.joypad.set_button(*button, i.key_down(*key));
            }
        });
    }

    /// Copia o framebuffer da PPU (RGBA8) para a textura egui.
    fn refresh_texture(&mut self) {
        let pixels: &[u8] = &*self.gba.bus.ppu.framebuffer;
        let mut img = egui::ColorImage::new(
            [SCREEN_WIDTH, SCREEN_HEIGHT],
            egui::Color32::TRANSPARENT,
        );
        for (i, px) in img.pixels.iter_mut().enumerate() {
            let off = i * 4;
            *px = egui::Color32::from_rgba_unmultiplied(
                pixels[off],
                pixels[off + 1],
                pixels[off + 2],
                pixels[off + 3],
            );
        }
        self.texture.set(img, egui::TextureOptions::NEAREST);
    }
}

impl eframe::App for AuroraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Avança 1 frame por update se rodando.
        if self.running {
            self.poll_input(ctx);
            self.gba.run_frame();
            self.refresh_texture();
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Arquivo", |ui| {
                    if ui.button("Abrir ROM…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("GBA ROM", &["gba"])
                            .pick_file()
                        {
                            self.open_rom(path);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Sair").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Emulação", |ui| {
                    if ui
                        .button(if self.running { "Pausar" } else { "Retomar" })
                        .clicked()
                    {
                        self.running = !self.running;
                        ui.close_menu();
                    }
                    if ui.button("Reset").clicked() {
                        if let Some(p) = self.rom_path.clone() {
                            self.open_rom(p);
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("Shiny Hunter", |ui| {
                    ui.label("(em breve)");
                });

                ui.separator();
                ui.label(format!("Scale: {:.0}x", self.scale));
                ui.add(egui::Slider::new(&mut self.scale, 1.0..=6.0).show_value(false));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let size = egui::vec2(
                    SCREEN_WIDTH as f32 * self.scale,
                    SCREEN_HEIGHT as f32 * self.scale,
                );
                ui.add(egui::Image::new(&self.texture).fit_to_exact_size(size));

                if let Some(p) = &self.rom_path {
                    ui.label(format!("ROM: {}", p.display()));
                } else {
                    ui.label("Nenhuma ROM carregada. Arquivo → Abrir ROM…");
                }

                let s = &self.gba.cpu.stats;
                ui.label(format!(
                    "ARM: {} · THUMB: {} · unimpl: {}",
                    s.arm_executed,
                    s.thumb_executed,
                    s.arm_unimplemented + s.thumb_unimplemented
                ));
            });
        });
    }
}
