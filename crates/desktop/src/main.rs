//! AuroraGBA — frontend desktop (Linux/Windows).
//!
//! Roda 1 frame por update da UI e exibe o framebuffer da PPU numa textura
//! 240×160 escalada.

use std::path::PathBuf;

use auroragba_core::joypad::Button;
use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};
use auroragba_shiny::games::GameProfile;
use auroragba_shiny::{CheckResult, Hunter};
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
    /// Contador de frames, usado pra limitar a frequência de gravação do save.
    frame_count: u64,
    /// Perfil do jogo detectado pelo header (None = não reconhecido / sem ROM).
    profile: Option<&'static GameProfile>,
    /// Índice do alvo selecionado dentro de `profile.targets`.
    selected_target: usize,
    /// Caça em andamento?
    hunting: bool,
    /// Estado do Shiny Hunter.
    hunter: Hunter,
    /// Velocidade da caça: frames de emulação por update da UI. 1 = tempo real
    /// (assistível, pra validar que está navegando certo); valores altos = caça
    /// rápida (mas vira um borrão).
    hunt_speed: u32,
}

impl AuroraApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let image = egui::ColorImage::new([SCREEN_WIDTH, SCREEN_HEIGHT], egui::Color32::BLACK);
        let texture =
            cc.egui_ctx
                .load_texture("gba-framebuffer", image, egui::TextureOptions::NEAREST);

        Self {
            gba: Gba::new(),
            rom_path: None,
            texture,
            running: false,
            scale: 3.0,
            frame_count: 0,
            profile: None,
            selected_target: 0,
            hunting: false,
            hunter: Hunter::new(),
            hunt_speed: 1, // começa em tempo real pra dar pra ver/validar
        }
    }

    fn open_rom(&mut self, path: PathBuf) {
        // Grava o save do jogo anterior antes de trocar de ROM.
        self.flush_save();

        match std::fs::read(&path) {
            Ok(rom) => {
                self.gba = Gba::new();
                self.gba.load_rom(rom);
                // Direct boot: estado pós-BIOS (modo System, SPs configurados),
                // entrada na ROM em 0x08000000. Os SWI são tratados por HLE.
                self.gba.cpu.setup_direct_boot();
                self.gba.cpu.regs.set_pc(0x0800_0000);
                self.rom_path = Some(path.clone());
                self.running = true;
                self.load_save(&path);

                // Identifica o jogo pelo game code do header pra habilitar o
                // Shiny Hunter com os endereços certos.
                let code = self.gba.bus.cartridge.game_code();
                self.profile = auroragba_shiny::games::detect(&code);
                self.selected_target = 0;
                self.hunting = false;
                self.hunter = Hunter::new();
                match self.profile {
                    Some(p) => log::info!("Jogo reconhecido: {} ({code})", p.name),
                    None => log::info!("Jogo não reconhecido pelo Shiny Hunter (code={code})"),
                }
            }
            Err(e) => log::error!("Falha ao abrir ROM: {e}"),
        }
    }

    /// Caminho do arquivo de save: a ROM com extensão `.sav`.
    fn save_path(&self) -> Option<PathBuf> {
        self.rom_path.as_ref().map(|p| p.with_extension("sav"))
    }

    /// Carrega `<rom>.sav` na memória de backup, se existir e o jogo salvar.
    fn load_save(&mut self, rom_path: &std::path::Path) {
        if !self.gba.bus.cartridge.has_save() {
            return;
        }
        let sav = rom_path.with_extension("sav");
        match std::fs::read(&sav) {
            Ok(bytes) => {
                if self.gba.bus.cartridge.load_backup(&bytes) {
                    log::info!("Save carregado: {}", sav.display());
                } else {
                    log::warn!("Save ignorado (tamanho incompatível): {}", sav.display());
                }
            }
            Err(_) => log::info!("Sem save prévio em {}", sav.display()),
        }
    }

    /// Grava o backup em disco se houve alteração desde a última gravação.
    fn flush_save(&mut self) {
        if !self.gba.bus.cartridge.dirty {
            return;
        }
        if let Some(path) = self.save_path() {
            match std::fs::write(&path, self.gba.bus.cartridge.backup_bytes()) {
                Ok(()) => {
                    self.gba.bus.cartridge.dirty = false;
                    log::info!("Save gravado: {}", path.display());
                }
                Err(e) => log::error!("Falha ao gravar save: {e}"),
            }
        }
    }

    /// Inicia a caça com o alvo selecionado. O jogador deve estar **parado na
    /// frente do alvo** com o save carregado; a primeira tentativa amassa A até
    /// a batalha, e as seguintes resetam sozinhas.
    fn start_hunt(&mut self) {
        if self.profile.is_some() {
            self.hunter = Hunter::new();
            self.hunting = true;
            self.running = false;
            log::info!("Caça iniciada.");
        }
    }

    /// Um passo da caça (lote de frames). Para e pausa ao achar o shiny.
    fn hunt_step(&mut self) {
        let Some(profile) = self.profile else {
            self.hunting = false;
            return;
        };
        let target = &profile.targets[self.selected_target];
        // `hunt_speed` frames por update (1 = tempo real, assistível). Timeout de
        // 1 min de tempo emulado por tentativa antes de resetar por segurança.
        let batch = self.hunt_speed.max(1);
        let result = self
            .hunter
            .tick(&mut self.gba, profile, target, batch, 60 * 60);
        if result == CheckResult::Shiny {
            // Achou! Devolve o controle no momento pós-seleção: o jogo entra na
            // batalha sozinho e o inicial shiny aparece (com os sparkles). O
            // jogador assiste/joga a partir daí (pode apertar Z=A pra avançar).
            self.hunting = false;
            self.running = true;
            log::info!(
                "✨ Shiny encontrado em {} tentativas! Controle devolvido pra você ver a batalha.",
                self.hunter.attempts
            );
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
        let mut img =
            egui::ColorImage::new([SCREEN_WIDTH, SCREEN_HEIGHT], egui::Color32::TRANSPARENT);
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
        if self.hunting {
            // Modo caça: o Hunter dirige a emulação (amassa A/Start, reseta entre
            // tentativas). Roda um lote de frames por update pra não travar a UI.
            self.hunt_step();
            self.refresh_texture();
            ctx.request_repaint();
        } else if self.running {
            // Modo jogo normal: 1 frame por update.
            self.poll_input(ctx);
            self.gba.run_frame();
            self.refresh_texture();
            ctx.request_repaint();

            // Persiste o save no máximo ~1×/s (um save no jogo gera milhares de
            // escritas byte-a-byte no Flash; não faz sentido tocar o disco a cada).
            self.frame_count += 1;
            if self.frame_count.is_multiple_of(60) {
                self.flush_save();
            }
        }

        // Grava o save ao fechar a janela.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush_save();
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
                ui.menu_button("Shiny Hunter", |ui| match self.profile {
                    Some(profile) => {
                        ui.label(format!("Jogo: {}", profile.name));

                        let current = profile.targets[self.selected_target].name;
                        egui::ComboBox::from_label("Alvo")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (i, t) in profile.targets.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected_target, i, t.name);
                                }
                            });

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Velocidade:");
                            ui.add(
                                egui::Slider::new(&mut self.hunt_speed, 1..=2000)
                                    .logarithmic(true)
                                    .suffix(" fr/upd"),
                            );
                        });
                        if self.hunt_speed == 1 {
                            ui.label("(tempo real — dá pra ver navegando)");
                        }

                        if self.hunting {
                            if ui.button("⏹ Parar caça").clicked() {
                                self.hunting = false;
                            }
                        } else if ui.button("▶ Iniciar caça").clicked() {
                            self.start_hunt();
                        }

                        ui.label(format!("Tentativas: {}", self.hunter.attempts));
                        if self.hunter.last_pid != 0 {
                            ui.label(format!(
                                "Último PID: {:08X}  (valor shiny: {})",
                                self.hunter.last_pid, self.hunter.last_shiny_value
                            ));
                            ui.label(format!(
                                "Espécie lida (índice interno): {}",
                                self.hunter.last_species
                            ));
                        }
                        if self.hunter.found {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 105, 180),
                                "✨ SHINY ENCONTRADO!",
                            );
                        }
                    }
                    None => {
                        ui.label("Jogo não reconhecido.");
                        ui.label("(carregue uma ROM Gen 3 suportada)");
                    }
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
