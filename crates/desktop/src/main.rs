//! AuroraGBA — frontend desktop (Linux/Windows).
//!
//! Roda 1 frame por update da UI e exibe o framebuffer da PPU numa textura
//! 240×160 escalada.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use auroragba_core::joypad::Button;
use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};
use auroragba_shiny::games::GameProfile;
use auroragba_shiny::gfx::RomGfx;
use auroragba_shiny::{CheckResult, Hunter};
use eframe::egui;

mod audio;

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
    /// Saída de áudio (None se não houver dispositivo).
    audio: Option<audio::AudioOut>,
    /// Tabelas de gráficos da ROM (pra decodificar o sprite do alvo). `None` se
    /// não localizadas (ROM não-Gen3 ou layout desconhecido).
    gfx: Option<RomGfx>,
    /// Cache de texturas de sprite por (espécie, shiny) — decodificar a cada
    /// frame seria desperdício.
    sprite_cache: HashMap<(u16, bool), Option<egui::TextureHandle>>,
    /// Instante em que a caça atual começou (pra tempo decorrido e taxa).
    hunt_started: Option<Instant>,
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
            audio: audio::AudioOut::new(),
            gfx: None,
            sprite_cache: HashMap::new(),
            hunt_started: None,
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
                self.hunt_started = None;
                // Localiza as tabelas de gráficos pra decodificar sprites do alvo.
                self.gfx = RomGfx::locate(&self.gba.bus.cartridge.rom);
                self.sprite_cache.clear();
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
            self.hunt_started = Some(Instant::now());
            log::info!("Caça iniciada.");
        }
    }

    /// Decodifica (com cache) o sprite do alvo da ROM e devolve a textura egui.
    /// `None` se a espécie é 0 (não preenchida) ou os gráficos não foram achados.
    fn target_sprite(
        &mut self,
        ctx: &egui::Context,
        species: u16,
        shiny: bool,
    ) -> Option<egui::TextureHandle> {
        if species == 0 {
            return None;
        }
        if let Some(cached) = self.sprite_cache.get(&(species, shiny)) {
            return cached.clone();
        }
        let handle = self.gfx.and_then(|gfx| {
            let sprite = gfx.decode_front(&self.gba.bus.cartridge.rom, species, shiny)?;
            let img = egui::ColorImage::from_rgba_unmultiplied(
                [sprite.width, sprite.height],
                &sprite.rgba,
            );
            Some(ctx.load_texture(
                format!("mon-{species}-{shiny}"),
                img,
                egui::TextureOptions::NEAREST,
            ))
        });
        self.sprite_cache.insert((species, shiny), handle.clone());
        handle
    }

    /// Desenha o painel lateral do Shiny Hunter: sprite do alvo + estatísticas
    /// da caça em tempo real + controles.
    fn shiny_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("✨ Shiny Hunter");
        let Some(profile) = self.profile else {
            ui.label("Jogo não reconhecido.");
            ui.label("(carregue uma ROM Gen 3 suportada)");
            return;
        };
        ui.label(profile.name);

        // Seletor de alvo.
        let current = profile.targets[self.selected_target].name;
        egui::ComboBox::from_label("Alvo")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (i, t) in profile.targets.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_target, i, t.name);
                }
            });
        let target = profile.targets[self.selected_target];

        // Sprite do alvo: normal + shiny lado a lado, pra comparar a cor que
        // estamos caçando. Quando o shiny aparece, destacamos a coluna dele.
        ui.separator();
        let ctx = ui.ctx().clone();
        let normal_tex = self.target_sprite(&ctx, target.species, false);
        let shiny_tex = self.target_sprite(&ctx, target.species, true);
        let found = self.hunter.found;
        ui.horizontal(|ui| {
            // Distribui as duas colunas igualmente na largura do painel.
            let col_w = (ui.available_width() - 8.0) / 2.0;
            let draw = |ui: &mut egui::Ui, tex: &Option<egui::TextureHandle>, label: &str, hot: bool| {
                ui.allocate_ui(egui::vec2(col_w, 130.0), |ui| {
                    ui.vertical_centered(|ui| {
                        match tex {
                            Some(tex) => {
                                ui.add(
                                    egui::Image::new(tex)
                                        .fit_to_exact_size(egui::vec2(96.0, 96.0)),
                                );
                            }
                            None => {
                                ui.add_space(24.0);
                                ui.label(egui::RichText::new("?").size(40.0).weak());
                                ui.add_space(24.0);
                            }
                        }
                        let rich = egui::RichText::new(label).small();
                        ui.label(if hot { rich.strong().color(egui::Color32::GOLD) } else { rich.weak() });
                    });
                });
            };
            draw(ui, &normal_tex, "Normal", false);
            draw(ui, &shiny_tex, "✨ Shiny", found);
        });

        // Contador grande.
        ui.separator();
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(self.hunter.attempts.to_string())
                    .size(30.0)
                    .strong(),
            );
            ui.label("tentativas");
        });

        // Tempo decorrido + taxa.
        if let Some(start) = self.hunt_started {
            let secs = start.elapsed().as_secs_f64();
            let (m, s) = (secs as u64 / 60, secs as u64 % 60);
            let rate = if secs > 0.5 {
                self.hunter.attempts as f64 / secs
            } else {
                0.0
            };
            ui.label(format!("⏱ {m:02}:{s:02}   ·   {rate:.1}/s"));
        }

        // Probabilidade acumulada de já ter achado pelo menos 1 shiny.
        let p = 1.0 - (1.0 - 1.0 / 8192.0_f64).powi(self.hunter.attempts as i32);
        ui.label(format!("📊 Chance acumulada: {:.1}%", p * 100.0));

        // Quão perto chegou (menor valor shiny visto).
        if self.hunter.best_shiny_value != 0xFFFF {
            ui.label(format!(
                "🔥 Mais perto: {} (tentativa #{})",
                self.hunter.best_shiny_value, self.hunter.best_attempt
            ));
        }

        // Último encontro.
        if self.hunter.last_pid != 0 {
            ui.separator();
            ui.label(format!("Último PID: {:08X}", self.hunter.last_pid));
            ui.label(format!(
                "Valor shiny: {} (shiny se < 8)",
                self.hunter.last_shiny_value
            ));
            ui.label(format!("Espécie lida: {}", self.hunter.last_species));
        }

        // Controles.
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Velocidade:");
            ui.add(
                egui::Slider::new(&mut self.hunt_speed, 1..=2000)
                    .logarithmic(true)
                    .suffix(" fr/upd"),
            );
        });
        if self.hunting {
            if ui.button("⏹ Parar caça").clicked() {
                self.hunting = false;
            }
        } else if ui.button("▶ Iniciar caça").clicked() {
            self.start_hunt();
        }

        if self.hunter.found {
            ui.separator();
            ui.vertical_centered(|ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 105, 180),
                    egui::RichText::new("✨ SHINY ENCONTRADO! ✨")
                        .size(18.0)
                        .strong(),
                );
            });
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
        // Descarta o áudio gerado durante a caça (não toca; evita crescer o buffer).
        self.gba.bus.apu.buffer.clear();
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
            self.poll_input(ctx);
            match &mut self.audio {
                Some(audio) => {
                    // Pacing pelo áudio: roda frames só até repor o buffer-alvo
                    // (no máx. 4 por update, pra não travar se a UI ficar lenta).
                    // Como o áudio é consumido em tempo real, isso ancora a
                    // emulação ao tempo real e corrige a "aceleração".
                    let target = audio.target();
                    let mut ran = 0;
                    while audio.queued() < target && ran < 4 {
                        self.gba.run_frame();
                        let samples = self.gba.bus.apu.drain();
                        audio.push(&samples, auroragba_core::apu::OUTPUT_RATE);
                        self.frame_count += 1;
                        ran += 1;
                    }
                }
                None => {
                    // Sem áudio: 1 frame por update (sincroniza pelo vsync da UI).
                    self.gba.run_frame();
                    self.gba.bus.apu.buffer.clear();
                    self.frame_count += 1;
                }
            }
            self.refresh_texture();
            ctx.request_repaint();

            // Persiste o save no máximo ~1×/s (um save no jogo gera milhares de
            // escritas byte-a-byte no Flash; não faz sentido tocar o disco a cada).
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
                ui.separator();
                ui.label(format!("Scale: {:.0}x", self.scale));
                ui.add(egui::Slider::new(&mut self.scale, 1.0..=6.0).show_value(false));
            });
        });

        // Painel do Shiny Hunter (só quando o jogo é reconhecido).
        if self.profile.is_some() {
            egui::SidePanel::right("shiny_panel")
                .min_width(230.0)
                .show(ctx, |ui| self.shiny_panel(ui));
        }

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
