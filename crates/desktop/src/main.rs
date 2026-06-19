//! AuroraGBA — frontend desktop (Linux/Windows).
//!
//! Roda 1 frame por update da UI e exibe o framebuffer da PPU numa textura
//! 240×160 escalada.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auroragba_core::joypad::Button as GbaButton;
use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};
use auroragba_shiny::games::GameProfile;
use auroragba_shiny::gfx::RomGfx;
use auroragba_shiny::{CheckResult, Hunter};
use eframe::egui;

mod audio;
mod discovery;
mod library;
mod link;
mod png;

/// Quantidade de slots de save state em disco (`<rom>.ss1`..`.ss8`).
const SAVE_SLOTS: usize = 8;

/// Rewind: a cada quantos frames tiramos um snapshot e quantos guardamos no anel.
/// Cada snapshot é o estado serializado inteiro (centenas de KB), então isto é uma
/// troca de RAM por duração: ~150 snapshots a cada 4 frames ≈ 10 s de rewind a
/// 60 fps, custando algumas dezenas de MB de RAM.
const REWIND_INTERVAL_FRAMES: u64 = 4;
const REWIND_MAX_SNAPSHOTS: usize = 150;

/// Fast-forward: em vez de um número fixo de frames por update, roda frames até
/// gastar este orçamento de tempo de parede. Com vsync, cada update tem ~16 ms;
/// gastar ~12 ms emulando (deixando ~4 ms pra UI) extrai o máximo de throughput
/// da janela em vez de rodar só uns poucos frames e ficar bloqueado no vsync.
const FAST_FORWARD_BUDGET: Duration = Duration::from_millis(12);
/// Teto de segurança de frames por update no fast-forward (evita travar a UI se
/// um frame for muito barato e o orçamento nunca estourar).
const FAST_FORWARD_MAX_FRAMES: u32 = 200;

/// Por quanto tempo a mensagem de status fica visível.
const STATUS_DURATION: Duration = Duration::from_secs(3);

/// Os 10 botões do GBA na ordem dos bits de KEYINPUT (= valor do enum). Os
/// arrays de [`InputConfig`] são indexados por `botão as usize`, então esta ordem
/// precisa casar com a do enum [`GbaButton`].
const GBA_BUTTONS: [GbaButton; 10] = [
    GbaButton::A,
    GbaButton::B,
    GbaButton::Select,
    GbaButton::Start,
    GbaButton::Right,
    GbaButton::Left,
    GbaButton::Up,
    GbaButton::Down,
    GbaButton::R,
    GbaButton::L,
];
/// Nomes dos botões na mesma ordem de [`GBA_BUTTONS`] (rótulos + chaves de
/// persistência).
const GBA_NAMES: [&str; 10] = [
    "A", "B", "Select", "Start", "Right", "Left", "Up", "Down", "R", "L",
];

/// Configuração de input: uma tecla de teclado e (opcionalmente) um botão de
/// gamepad por botão do GBA. Indexada por `botão as usize` (ver [`GBA_BUTTONS`]).
#[derive(Clone)]
struct InputConfig {
    keys: [egui::Key; 10],
    pads: [Option<gilrs::Button>; 10],
}

impl Default for InputConfig {
    fn default() -> Self {
        use egui::Key;
        use gilrs::Button as P;
        // Ordem: A, B, Select, Start, Right, Left, Up, Down, R, L.
        Self {
            keys: [
                Key::Z,
                Key::X,
                Key::Backspace,
                Key::Enter,
                Key::ArrowRight,
                Key::ArrowLeft,
                Key::ArrowUp,
                Key::ArrowDown,
                Key::S,
                Key::A,
            ],
            pads: [
                Some(P::East),
                Some(P::South),
                Some(P::Select),
                Some(P::Start),
                Some(P::DPadRight),
                Some(P::DPadLeft),
                Some(P::DPadUp),
                Some(P::DPadDown),
                Some(P::RightTrigger),
                Some(P::LeftTrigger),
            ],
        }
    }
}

impl InputConfig {
    /// Serializa num texto simples de linhas `key.<Botão>=<Tecla>` /
    /// `pad.<Botão>=<BotãoDoPad>` (`-` = sem binding), pra guardar no storage.
    fn serialize(&self) -> String {
        let mut s = String::new();
        for (i, name) in GBA_NAMES.iter().enumerate() {
            s.push_str(&format!("key.{}={}\n", name, self.keys[i].name()));
            let pad = self.pads[i].map(pad_name).unwrap_or("-");
            s.push_str(&format!("pad.{name}={pad}\n"));
        }
        s
    }

    /// Lê o formato de [`serialize`](Self::serialize), partindo do padrão e
    /// sobrescrevendo o que reconhecer (linhas inválidas são ignoradas).
    fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        for line in text.lines() {
            let Some((lhs, val)) = line.split_once('=') else {
                continue;
            };
            let Some((kind, name)) = lhs.split_once('.') else {
                continue;
            };
            let Some(idx) = GBA_NAMES.iter().position(|n| *n == name) else {
                continue;
            };
            match kind {
                "key" => {
                    if let Some(key) = egui::Key::from_name(val) {
                        cfg.keys[idx] = key;
                    }
                }
                "pad" => cfg.pads[idx] = pad_from_name(val),
                _ => {}
            }
        }
        cfg
    }
}

/// O que está sendo remapeado agora (índice do botão do GBA).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rebind {
    Key(usize),
    Pad(usize),
}

/// Tamanho da capa (caixa) no grid da biblioteca.
const COVER_W: f32 = 110.0;
const COVER_H: f32 = 124.0;

/// Desenha uma célula da biblioteca (capa + título) e devolve `true` se clicada.
fn cover_cell(ui: &mut egui::Ui, entry: &library::RomEntry) -> bool {
    let mut clicked = false;
    ui.allocate_ui(egui::vec2(COVER_W + 8.0, COVER_H + 30.0), |ui| {
        ui.vertical_centered(|ui| {
            let resp = match &entry.cover {
                Some(tex) => {
                    // Preserva o aspecto da capa dentro da caixa (box art é
                    // retrato, screenshot é paisagem).
                    let ts = tex.size_vec2();
                    let scale = (COVER_W / ts.x).min(COVER_H / ts.y);
                    let img = egui::Image::new(tex).fit_to_exact_size(ts * scale);
                    ui.add(egui::ImageButton::new(img))
                }
                None => {
                    // Placeholder enquanto o worker gera a capa.
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(COVER_W, COVER_H), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 4.0, egui::Color32::from_gray(40));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "…",
                        egui::FontId::proportional(24.0),
                        egui::Color32::GRAY,
                    );
                    resp
                }
            };
            clicked = resp.clicked();
            let label = if entry.title.is_empty() {
                entry.code.as_str()
            } else {
                entry.title.as_str()
            };
            ui.label(egui::RichText::new(label).small())
                .on_hover_text(entry.path.display().to_string());
        });
    });
    clicked
}

/// Nome estável de um botão de gamepad (pra UI e persistência).
fn pad_name(b: gilrs::Button) -> &'static str {
    use gilrs::Button::*;
    match b {
        South => "South",
        East => "East",
        North => "North",
        West => "West",
        C => "C",
        Z => "Z",
        LeftTrigger => "LeftTrigger",
        LeftTrigger2 => "LeftTrigger2",
        RightTrigger => "RightTrigger",
        RightTrigger2 => "RightTrigger2",
        Select => "Select",
        Start => "Start",
        Mode => "Mode",
        LeftThumb => "LeftThumb",
        RightThumb => "RightThumb",
        DPadUp => "DPadUp",
        DPadDown => "DPadDown",
        DPadLeft => "DPadLeft",
        DPadRight => "DPadRight",
        Unknown => "Unknown",
    }
}

/// Inverso de [`pad_name`].
fn pad_from_name(s: &str) -> Option<gilrs::Button> {
    use gilrs::Button::*;
    Some(match s {
        "South" => South,
        "East" => East,
        "North" => North,
        "West" => West,
        "C" => C,
        "Z" => Z,
        "LeftTrigger" => LeftTrigger,
        "LeftTrigger2" => LeftTrigger2,
        "RightTrigger" => RightTrigger,
        "RightTrigger2" => RightTrigger2,
        "Select" => Select,
        "Start" => Start,
        "Mode" => Mode,
        "LeftThumb" => LeftThumb,
        "RightThumb" => RightThumb,
        "DPadUp" => DPadUp,
        "DPadDown" => DPadDown,
        "DPadLeft" => DPadLeft,
        "DPadRight" => DPadRight,
        _ => return None,
    })
}

/// Detector automático do byte do cursor do menu do inicial (ferramenta de
/// debug). Enquanto ativo, observa cada byte da IWRAM e, por offset, guarda um
/// bitmask: bits 0/1/2 marcam que o byte já valeu 0/1/2, e o bit 3 marca que ele
/// já passou de 2. O cursor do inicial é uma variável **discreta 0..2**: passa
/// por 0, 1 e 2 (mover ◄/► pelos três Poké Balls) e **nunca** excede 2. Isso
/// exclui contadores e bytes de animação (que estouram 2 em algum frame), sem
/// precisar digitar valor nenhum.
#[derive(Default)]
struct CursorFinder {
    /// Por offset da IWRAM: bits 0/1/2 = viu o valor; bit 3 = viu valor > 2.
    /// Vazio = não está rastreando.
    seen: Vec<u8>,
}

const SEEN_OVER_2: u8 = 0b1000;
const SEEN_ALL_012: u8 = 0b0111;

impl CursorFinder {
    /// (Re)inicia o rastreamento, zerando o histórico.
    fn start(&mut self) {
        self.seen = vec![0u8; 0x8000];
    }

    fn tracking(&self) -> bool {
        !self.seen.is_empty()
    }

    /// Registra os valores de cada byte neste frame. Chamado 1×/frame.
    fn observe(&mut self, iwram: &[u8]) {
        if self.seen.is_empty() {
            return;
        }
        for (slot, &v) in self.seen.iter_mut().zip(iwram.iter()) {
            *slot |= if v <= 2 { 1 << v } else { SEEN_OVER_2 };
        }
    }

    /// Offsets que mostraram 0, 1 e 2 e **nunca** passaram de 2 — candidatos
    /// fortes a cursor (discreto, clampado em 0..2).
    fn candidates(&self) -> Vec<u16> {
        self.seen
            .iter()
            .enumerate()
            .filter(|(_, &m)| m == SEEN_ALL_012)
            .map(|(i, _)| i as u16)
            .collect()
    }
}

/// Localizador (debug) do **bit** de estado de batalha (ex.: `gMain.inBattle`) na
/// IWRAM. O flag costuma ser um bitfield (divide o byte com outros bits), então
/// procuramos um BIT — não o byte inteiro — que seja 1 em **todo** snapshot de
/// batalha e **nunca** num snapshot de overworld:
///   - `battle_and[i]`: AND dos snapshots de batalha (bits 1 em toda batalha);
///   - `over_or[i]`: OR dos snapshots de overworld (bits já vistos 1 fora dela).
///
/// Candidato em `i` = `battle_and[i] & !over_or[i]` (bits que sobram). Usa
/// **snapshots explícitos** (não acumulação por frame): assim os frames de
/// transição do início da batalha — em que `inBattle` já é 1 mas a tela ainda
/// parece overworld — não envenenam o OR. Mais snapshots de cada lado afunilam.
#[derive(Default)]
struct BattleStateFinder {
    battle_and: Vec<u8>,
    over_or: Vec<u8>,
    battle_snaps: u32,
    over_snaps: u32,
}

impl BattleStateFinder {
    const IWRAM_LEN: usize = 0x8000;

    fn start(&mut self) {
        self.battle_and = vec![0xFFu8; Self::IWRAM_LEN];
        self.over_or = vec![0x00u8; Self::IWRAM_LEN];
        self.battle_snaps = 0;
        self.over_snaps = 0;
    }

    fn tracking(&self) -> bool {
        !self.battle_and.is_empty()
    }

    /// Dobra o estado atual da IWRAM num snapshot de batalha (AND) ou de overworld
    /// (OR), conforme o botão clicado.
    fn snapshot(&mut self, iwram: &[u8], in_battle: bool) {
        if self.battle_and.is_empty() {
            self.start();
        }
        if in_battle {
            for (acc, &v) in self.battle_and.iter_mut().zip(iwram.iter()) {
                *acc &= v;
            }
            self.battle_snaps += 1;
        } else {
            for (acc, &v) in self.over_or.iter_mut().zip(iwram.iter()) {
                *acc |= v;
            }
            self.over_snaps += 1;
        }
    }

    /// `(offset, máscara de bits candidatos)` — bits 1 em toda batalha e nunca no
    /// overworld. Só faz sentido com ≥1 snapshot de cada lado.
    fn candidates(&self) -> Vec<(u16, u8)> {
        if self.battle_snaps == 0 || self.over_snaps == 0 {
            return Vec::new();
        }
        self.battle_and
            .iter()
            .zip(self.over_or.iter())
            .enumerate()
            .filter_map(|(i, (&b, &o))| {
                let mask = b & !o;
                (mask != 0).then_some((i as u16, mask))
            })
            .collect()
    }
}

/// Mostra o painel de busca de RAM (debug) pra achar endereços por versão (ex.:
/// o cursor do menu do inicial) e o readout de estado de batalha. Fica desligado
/// por padrão pra não poluir a UI; vire pra `true` quando precisar mapear um jogo
/// novo (cursor do inicial, flag de batalha).
const SHOW_RAM_FINDER: bool = false;

fn main() -> eframe::Result<()> {
    env_logger::init();

    // Fase Link (etapa b): `--link-host [porta]` hospeda (parent) e
    // `--link-join <ip:porta>` entra (child). A sessão é estabelecida antes
    // da janela abrir — o host bloqueia até o parceiro chegar.
    let link_session = parse_link_args();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_title("AuroraGBA"),
        ..Default::default()
    };

    eframe::run_native(
        "AuroraGBA",
        options,
        Box::new(|cc| Box::new(AuroraApp::new(cc, link_session))),
    )
}

/// Lê as flags de link da linha de comando e estabelece a sessão (ou None).
fn parse_link_args() -> Option<link::LinkSession> {
    let args: Vec<String> = std::env::args().collect();
    let pos = |flag: &str| args.iter().position(|a| a == flag);
    if let Some(i) = pos("--link-host") {
        let port: u16 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(7777);
        eprintln!("link: aguardando o parceiro na porta {port}…");
        match link::PendingLink::host(port).wait() {
            Ok(s) => {
                eprintln!("link: parceiro conectado (somos o parent)");
                Some(s)
            }
            Err(e) => {
                eprintln!("link: falhou ({e}); seguindo sem link");
                None
            }
        }
    } else if let Some(i) = pos("--link-join") {
        let addr = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "127.0.0.1:7777".into());
        eprintln!("link: conectando em {addr}…");
        match link::PendingLink::join(addr).wait() {
            Ok(s) => {
                eprintln!("link: conectado (somos o child)");
                Some(s)
            }
            Err(e) => {
                eprintln!("link: falhou ({e}); seguindo sem link");
                None
            }
        }
    } else {
        None
    }
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
    /// Detector (debug) do byte do cursor do inicial na RAM. Ver [`CursorFinder`].
    cursor_finder: CursorFinder,
    /// Localizador (debug) do bit de estado de batalha. Ver [`BattleStateFinder`].
    battle_finder: BattleStateFinder,
    /// Slot de save state atual (0-indexado) usado por F5/F9.
    current_slot: usize,
    /// Anel de estados serializados pro rewind (o mais recente no fim).
    rewind: VecDeque<Vec<u8>>,
    /// Mensagem efêmera de status (texto + instante em que foi mostrada).
    status: Option<(String, Instant)>,
    /// Medição de velocidade: fps emulado calculado a cada ~1 s.
    fps: f64,
    /// Marca da última amostragem de fps (instante + `frame_count` na hora).
    fps_sample: (Instant, u64),
    /// Mapeamento de teclado/gamepad → botões do GBA (persistido no storage).
    input: InputConfig,
    /// Contexto do gilrs pra ler gamepads (None se indisponível no host).
    gilrs: Option<gilrs::Gilrs>,
    /// Janela de configuração de controles aberta?
    show_input_config: bool,
    /// Binding em remapeamento (aguardando a próxima tecla/botão). None = nenhum.
    rebinding: Option<Rebind>,
    /// Biblioteca de ROMs (varredura de pasta + capas).
    library: library::Library,
    /// Janela da biblioteca aberta?
    show_library: bool,
    /// Sessão de lockstep do link cable (Fase Link, etapa b). `None` = solo.
    link: Option<link::LinkSession>,
    /// Conexão de link em andamento numa thread de fundo. `None` = sem tentativa.
    link_pending: Option<link::PendingLink>,
    /// Janela de Link aberta?
    show_link: bool,
    /// Campo de texto da porta pra hospedar.
    link_port: String,
    /// Campo de texto do endereço ("ip:porta") pra conectar.
    link_addr: String,
    /// Escuta de hosts na LAN (descoberta UDP). `None` = indisponível.
    discovery: Option<discovery::Browser>,
    /// Anúncio do nosso host na LAN enquanto hospedamos. `None` = não anunciando.
    link_advertiser: Option<discovery::Advertiser>,
}

/// Ação escolhida no painel de Link, aplicada depois de fechar a UI (pra não
/// segurar `&self` enquanto se mexe em `self.link`/`self.link_pending`).
enum LinkAction {
    Host,
    Join,
    /// Conectar num host descoberto na LAN (endereço já pronto).
    JoinAddr(String),
    Cancel,
    Disconnect,
}

/// Nome amigável da máquina pra anunciar na LAN. Sem dependência: tenta as
/// variáveis de ambiente usuais e cai num genérico.
fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AuroraGBA".to_string())
}

impl AuroraApp {
    fn new(cc: &eframe::CreationContext<'_>, link_session: Option<link::LinkSession>) -> Self {
        let image = egui::ColorImage::new([SCREEN_WIDTH, SCREEN_HEIGHT], egui::Color32::BLACK);
        let texture =
            cc.egui_ctx
                .load_texture("gba-framebuffer", image, egui::TextureOptions::NEAREST);

        // Pasta de cache das capas: <dados-do-app>/covers (ou ./covers se o
        // diretório de dados não estiver disponível).
        let cache_dir = eframe::storage_dir("AuroraGBA")
            .unwrap_or_else(|| PathBuf::from("."))
            .join("covers");
        let saved_lib_dir = cc
            .storage
            .and_then(|s| s.get_string("library_dir"))
            .map(PathBuf::from);

        let mut app = Self {
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
            cursor_finder: CursorFinder::default(),
            battle_finder: BattleStateFinder::default(),
            current_slot: 0,
            rewind: VecDeque::new(),
            status: None,
            fps: 0.0,
            fps_sample: (Instant::now(), 0),
            input: cc
                .storage
                .and_then(|s| s.get_string("input_config"))
                .map(|t| InputConfig::parse(&t))
                .unwrap_or_default(),
            gilrs: match gilrs::Gilrs::new() {
                Ok(g) => Some(g),
                Err(e) => {
                    log::warn!("gilrs indisponível (sem gamepad): {e}");
                    None
                }
            },
            show_input_config: false,
            rebinding: None,
            library: library::Library::new(cache_dir),
            show_library: false,
            link: link_session,
            link_pending: None,
            show_link: false,
            link_port: link::DEFAULT_PORT.to_string(),
            link_addr: format!("127.0.0.1:{}", link::DEFAULT_PORT),
            discovery: match discovery::Browser::start() {
                Ok(b) => Some(b),
                Err(e) => {
                    log::info!("descoberta de link na LAN indisponível: {e}");
                    None
                }
            },
            link_advertiser: None,
        };

        if let Some(s) = &app.link {
            let papel = if s.id == 0 { "parent" } else { "child" };
            app.set_status(format!("🔗 link ativo — somos o {papel}"));
        }

        // Restaura a última pasta de ROMs e já mostra a biblioteca no boot.
        if let Some(dir) = saved_lib_dir {
            if dir.is_dir() {
                app.library.scan(dir);
                app.show_library = true;
            }
        }
        app
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

                // A sessão de link sobrevive à troca de ROM — o Gba é novo,
                // então re-aplica a configuração (papel/ID na mesa).
                if let Some(s) = &self.link {
                    self.gba.link_configure(true, s.id);
                }

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
                self.rewind.clear();
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

    /// Mostra uma mensagem efêmera de status no rodapé (some após
    /// [`STATUS_DURATION`]).
    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// Caminho do arquivo do slot de save state `slot` (0-indexado): a ROM com
    /// extensão `.ssN` (N = slot + 1).
    fn slot_path(&self, slot: usize) -> Option<PathBuf> {
        self.rom_path
            .as_ref()
            .map(|p| p.with_extension(format!("ss{}", slot + 1)))
    }

    /// Salva o estado atual no slot informado (arquivo `.ssN` ao lado da ROM).
    fn save_state_slot(&mut self, slot: usize) {
        if self.rom_path.is_none() {
            self.set_status("Carregue uma ROM antes de salvar estado");
            return;
        }
        let blob = self.gba.save_state();
        match self.slot_path(slot) {
            Some(path) => match std::fs::write(&path, &blob) {
                Ok(()) => self.set_status(format!("Estado salvo no slot {}", slot + 1)),
                Err(e) => self.set_status(format!("Falha ao salvar estado: {e}")),
            },
            None => self.set_status("Carregue uma ROM antes de salvar estado"),
        }
    }

    /// Carrega o estado do slot informado por cima do jogo atual. Recusa estados
    /// de outro jogo (o cabeçalho do save state guarda o game code).
    fn load_state_slot(&mut self, slot: usize) {
        let Some(path) = self.slot_path(slot) else {
            self.set_status("Carregue uma ROM antes de carregar estado");
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => match self.gba.load_state(&bytes) {
                Ok(()) => {
                    // O estado restaurado é um novo "agora": o anel de rewind
                    // antigo não bate mais com esta linha do tempo.
                    self.rewind.clear();
                    self.running = true;
                    self.set_status(format!("Estado carregado do slot {}", slot + 1));
                }
                Err(e) => self.set_status(format!("Falha ao carregar estado: {e}")),
            },
            Err(_) => self.set_status(format!("Slot {} vazio", slot + 1)),
        }
    }

    /// True se o slot tem um arquivo de save state em disco.
    fn slot_exists(&self, slot: usize) -> bool {
        self.slot_path(slot).is_some_and(|p| p.exists())
    }

    /// Empilha um snapshot do estado atual no anel de rewind, descartando o mais
    /// antigo se passar da capacidade.
    fn push_rewind_snapshot(&mut self) {
        self.rewind.push_back(self.gba.save_state());
        if self.rewind.len() > REWIND_MAX_SNAPSHOTS {
            self.rewind.pop_front();
        }
    }

    /// Volta um snapshot no tempo (consome o mais recente do anel). No-op se o
    /// anel está vazio (já voltou tudo que tinha guardado).
    fn rewind_step(&mut self) {
        if let Some(snap) = self.rewind.pop_back() {
            // `load_state` só falha por incompatibilidade de jogo/versão, o que
            // não acontece com snapshots que nós mesmos geramos agora.
            let _ = self.gba.load_state(&snap);
        }
    }

    /// Salva um PNG do framebuffer atual em `<dir-da-rom>/screenshots/`.
    fn screenshot(&mut self) {
        let Some(rom) = self.rom_path.clone() else {
            self.set_status("Carregue uma ROM antes de capturar a tela");
            return;
        };
        let dir = rom
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("screenshots");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.set_status(format!("Falha ao criar pasta de screenshots: {e}"));
            return;
        }
        let stem = rom.file_stem().and_then(|s| s.to_str()).unwrap_or("rom");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("{stem}-{ts}.png"));
        let bytes = png::encode_rgba(
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
            &self.gba.bus.ppu.framebuffer[..],
        );
        match std::fs::write(&path, bytes) {
            Ok(()) => self.set_status(format!("Screenshot: {}", path.display())),
            Err(e) => self.set_status(format!("Falha ao salvar screenshot: {e}")),
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
            let draw =
                |ui: &mut egui::Ui, tex: &Option<egui::TextureHandle>, label: &str, hot: bool| {
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
                            ui.label(if hot {
                                rich.strong().color(egui::Color32::GOLD)
                            } else {
                                rich.weak()
                            });
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

        if SHOW_RAM_FINDER {
            self.cursor_finder_ui(ui);
            self.battle_state_finder_ui(ui);
        }
    }

    /// Ferramenta de debug pra achar o endereço do cursor do menu do inicial na
    /// RAM, **automaticamente**: com o jogo em modo manual e a bag aberta, basta
    /// clicar "Detectar" e mover ◄/► pelos três Poké Balls. A ferramenta acha o
    /// byte que passou por 0, 1 e 2. Esse endereço vai pro perfil do jogo pra
    /// caça em malha fechada.
    fn cursor_finder_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        egui::CollapsingHeader::new("🔎 Achar cursor do inicial (debug)").show(ui, |ui| {
            ui.label(
                egui::RichText::new(
                    "Com a bag aberta, clique Detectar e mova ◄ e ► passando por \
                     TODOS os 3 Poké Balls (esquerda, centro, direita) — até o nome \
                     no canto mudar entre os três. O endereço aparece sozinho.",
                )
                .small()
                .weak(),
            );
            if ui.button("Detectar (resetar)").clicked() {
                self.cursor_finder.start();
            }
            if self.cursor_finder.tracking() {
                let cands = self.cursor_finder.candidates();
                ui.label(format!("candidatos (viram 0,1,2): {}", cands.len()));
                for &off in cands.iter().take(16) {
                    let addr = 0x0300_0000u32 + off as u32;
                    let val = self.gba.bus.iwram[off as usize];
                    // Se o candidato for `gTasks[i].data[0]`, a função da task fica
                    // 8 bytes antes (offset 0 da struct Task) — mostrá-la permite
                    // cravar a detecção do menu aberto.
                    let func = if off >= 8 {
                        let b = off as usize - 8;
                        u32::from_le_bytes([
                            self.gba.bus.iwram[b],
                            self.gba.bus.iwram[b + 1],
                            self.gba.bus.iwram[b + 2],
                            self.gba.bus.iwram[b + 3],
                        ])
                    } else {
                        0
                    };
                    ui.monospace(format!("0x{addr:08X} = {val}   (func: 0x{func:08X})"));
                }
            }
        });
    }

    /// Localizador (debug) do bit de estado de batalha (`gMain.inBattle` & cia).
    /// O `gEnemyParty` fica sujo depois da fuga, então a detecção de overworld do
    /// Marco 2 vai depender desse flag. Por snapshots explícitos: tire um no
    /// overworld (andando) e um na batalha (com o menu FIGHT/RUN na tela). Repita
    /// alguns de cada — bem dentro de cada estado, nunca na transição — até
    /// sobrar 1 candidato.
    fn battle_state_finder_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("🔎 Achar flag de batalha (debug)").show(ui, |ui| {
            ui.label(
                egui::RichText::new(
                    "Reset → ande no overworld e clique 'Snapshot overworld' → \
                     entre numa batalha e, com o menu na tela, clique 'Snapshot \
                     batalha' → fuja. Repita 2-3× de cada lado até sobrar 1.",
                )
                .small()
                .weak(),
            );
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    self.battle_finder.start();
                }
                if ui.button("Snapshot overworld").clicked() {
                    self.battle_finder.snapshot(&self.gba.bus.iwram[..], false);
                }
                if ui.button("Snapshot batalha").clicked() {
                    self.battle_finder.snapshot(&self.gba.bus.iwram[..], true);
                }
            });
            // Suspeito forte: gMain (0x030022C0) + 0x439 = byte do bitfield onde
            // mora `inBattle` (bit 0x02). Olhe esta linha: no overworld o bit deve
            // ser 0, na batalha 0x02. (Se não bater, o gMain pode ser de outra
            // revisão — aí use a lista filtrada abaixo.)
            let suspect = 0x0000_26F9usize;
            let sval = self.gba.bus.iwram[suspect];
            ui.monospace(format!(
                "suspeito 0x030026F9 = {sval:#04X}  (inBattle bit 0x02 = {:#04X})",
                sval & 0x02
            ));
            if self.battle_finder.tracking() {
                ui.label(format!(
                    "snapshots: overworld={}, batalha={}",
                    self.battle_finder.over_snaps, self.battle_finder.battle_snaps
                ));
                let cands = self.battle_finder.candidates();
                // Filtra pro entorno do gMain (≥0x2000): corta o ruído dos buffers
                // de batalha na RAM baixa, que também são "0 no overworld, 1 na luta".
                let near_gmain: Vec<_> = cands.iter().filter(|(off, _)| *off >= 0x2000).collect();
                ui.label(format!(
                    "candidatos: {} total, {} perto do gMain (≥0x2000)",
                    cands.len(),
                    near_gmain.len()
                ));
                for &&(off, mask) in near_gmain.iter().take(24) {
                    let addr = 0x0300_0000u32 + off as u32;
                    let val = self.gba.bus.iwram[off as usize];
                    ui.monospace(format!("0x{addr:08X}  bit={mask:#04X}  (byte={val:#04X})"));
                }
            }
        });
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

    /// Lê teclado + gamepad e atualiza o estado dos botões do GBA. Um botão fica
    /// pressionado se a tecla **ou** o botão do pad correspondente estiver. (Os
    /// eventos do gilrs já são bombeados em `update`; aqui só consultamos o estado.)
    fn poll_input(&mut self, ctx: &egui::Context) {
        let mut pressed = [false; 10];
        ctx.input(|i| {
            for (idx, key) in self.input.keys.iter().enumerate() {
                if i.key_down(*key) {
                    pressed[idx] = true;
                }
            }
        });
        if let Some(gilrs) = &self.gilrs {
            for (_id, gamepad) in gilrs.gamepads() {
                for (idx, pad) in self.input.pads.iter().enumerate() {
                    if let Some(b) = pad {
                        if gamepad.is_pressed(*b) {
                            pressed[idx] = true;
                        }
                    }
                }
            }
        }
        for (idx, &p) in pressed.iter().enumerate() {
            self.gba.bus.io.joypad.set_button(GBA_BUTTONS[idx], p);
        }
    }

    /// Bombeia os eventos pendentes do gilrs (necessário pra `is_pressed` refletir
    /// o estado atual) e devolve o último botão **pressionado** neste tick, se
    /// houver — usado pra capturar o remapeamento de gamepad.
    fn pump_gamepad(&mut self) -> Option<gilrs::Button> {
        let mut last = None;
        if let Some(gilrs) = &mut self.gilrs {
            while let Some(ev) = gilrs.next_event() {
                if let gilrs::EventType::ButtonPressed(b, _) = ev.event {
                    if b != gilrs::Button::Unknown {
                        last = Some(b);
                    }
                }
            }
        }
        last
    }

    /// Janela de configuração de controles: uma grade Botão × (Teclado, Gamepad)
    /// com remapeamento ao clicar (aguarda a próxima tecla/botão).
    fn input_config_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_input_config;
        egui::Window::new("🎮 Controles")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Clique num campo e pressione a tecla/botão. Esc cancela.")
                        .small()
                        .weak(),
                );
                egui::Grid::new("input_grid")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Botão").strong());
                        ui.label(egui::RichText::new("Teclado").strong());
                        ui.label(egui::RichText::new("Gamepad").strong());
                        ui.end_row();
                        for (i, name) in GBA_NAMES.iter().enumerate() {
                            ui.label(*name);
                            let key_label = if self.rebinding == Some(Rebind::Key(i)) {
                                "[pressione…]".to_string()
                            } else {
                                self.input.keys[i].name().to_string()
                            };
                            if ui.button(key_label).clicked() {
                                self.rebinding = Some(Rebind::Key(i));
                            }
                            let pad_label = if self.rebinding == Some(Rebind::Pad(i)) {
                                "[pressione…]".to_string()
                            } else {
                                self.input.pads[i].map(pad_name).unwrap_or("—").to_string()
                            };
                            if ui.button(pad_label).clicked() {
                                self.rebinding = Some(Rebind::Pad(i));
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Restaurar padrão").clicked() {
                        self.input = InputConfig::default();
                        self.rebinding = None;
                    }
                    if self.rebinding.is_some() && ui.button("Cancelar (Esc)").clicked() {
                        self.rebinding = None;
                    }
                });
                let pads = self.gilrs.as_ref().map_or(0, |g| g.gamepads().count());
                ui.label(
                    egui::RichText::new(format!("Gamepads conectados: {pads}"))
                        .small()
                        .weak(),
                );
            });
        self.show_input_config = open;
    }

    /// Janela da biblioteca: escolher pasta + grid de capas clicáveis.
    fn library_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_library;
        let mut pick = false;
        let mut launch: Option<PathBuf> = None;
        egui::Window::new("📚 Biblioteca")
            .open(&mut open)
            .default_size([700.0, 480.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("📁 Escolher pasta…").clicked() {
                        pick = true;
                    }
                    match &self.library.dir {
                        Some(d) => ui.label(d.display().to_string()),
                        None => ui.label("Nenhuma pasta selecionada."),
                    };
                });
                ui.separator();
                if self.library.entries.is_empty() {
                    ui.label("Nenhuma ROM .gba aqui. Escolha uma pasta com seus jogos.");
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for entry in &self.library.entries {
                            if cover_cell(ui, entry) {
                                launch = Some(entry.path.clone());
                            }
                        }
                    });
                });
            });
        self.show_library = open;
        if pick {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                self.library.scan(dir);
            }
        }
        if let Some(path) = launch {
            self.open_rom(path);
            self.show_library = false;
        }
    }

    /// Verifica se a thread de conexão de link terminou. Em sucesso, instala a
    /// sessão e configura o `Gba` com o nosso papel na mesa; em falha, volta ao
    /// modo solo com um aviso (cancelamento é silencioso). Enquanto pendente,
    /// pede repintura pra UI animar o spinner.
    fn poll_link(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.link_pending else {
            return;
        };
        match pending.poll() {
            None => ctx.request_repaint(),
            Some(Ok(session)) => {
                let papel = if session.id == 0 { "host" } else { "convidado" };
                self.gba.link_configure(true, session.id);
                self.set_status(format!("🔗 link conectado — somos o {papel}"));
                self.link = Some(session);
                self.link_pending = None;
                self.link_advertiser = None; // conectou: para de se anunciar
            }
            Some(Err(e)) => {
                // `Interrupted` = cancelamento pedido pelo usuário, sem alarde.
                if e.kind() != std::io::ErrorKind::Interrupted {
                    self.set_status(format!("link falhou ({e})"));
                }
                self.link_pending = None;
                self.link_advertiser = None;
            }
        }
    }

    /// Painel de Link: hospedar/conectar (ocioso), spinner (conectando) ou
    /// status + desconectar (conectado). A conexão roda em thread de fundo
    /// ([`link::PendingLink`]), então a UI nunca trava esperando o parceiro.
    fn link_window(&mut self, ctx: &egui::Context) {
        // Espelha o estado em valores `Copy` pra não segurar `&self` enquanto a
        // UI roda — as mutações de fato acontecem depois, via `action`.
        let connected_id = self.link.as_ref().map(|s| s.id);
        let pending_hosting = self.link_pending.as_ref().map(|p| p.hosting);
        // Snapshot dos hosts descobertos na LAN (vazio se a descoberta está off).
        let peers = self
            .discovery
            .as_ref()
            .map(|d| d.peers())
            .unwrap_or_default();
        let mut open = self.show_link;
        let mut action: Option<LinkAction> = None;

        egui::Window::new("🔗 Link")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if let Some(id) = connected_id {
                    let papel = if id == 0 {
                        "host (parent)"
                    } else {
                        "convidado (child)"
                    };
                    ui.label(format!("✅ Conectado — somos o {papel}."));
                    ui.label(
                        egui::RichText::new(
                            "A troca acontece dentro do jogo (balcão da Pokémon Center).",
                        )
                        .small()
                        .weak(),
                    );
                    ui.separator();
                    if ui.button("Desconectar").clicked() {
                        action = Some(LinkAction::Disconnect);
                    }
                } else if let Some(hosting) = pending_hosting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if hosting {
                            "Aguardando o parceiro conectar…"
                        } else {
                            "Conectando…"
                        });
                    });
                    if hosting {
                        ui.label(
                            egui::RichText::new(format!(
                                "Ouvindo na porta {}. Passe o seu IP da rede pro parceiro.",
                                self.link_port
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    ui.separator();
                    if ui.button("Cancelar").clicked() {
                        action = Some(LinkAction::Cancel);
                    }
                } else {
                    ui.label("Hospedar (você gera o clock — é o master):");
                    ui.horizontal(|ui| {
                        ui.label("Porta:");
                        ui.add(egui::TextEdit::singleline(&mut self.link_port).desired_width(70.0));
                        if ui.button("Hospedar").clicked() {
                            action = Some(LinkAction::Host);
                        }
                    });
                    ui.separator();
                    ui.label("Conectar a um host:");
                    ui.horizontal(|ui| {
                        ui.label("Endereço:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.link_addr).desired_width(160.0),
                        );
                        if ui.button("Conectar").clicked() {
                            action = Some(LinkAction::Join);
                        }
                    });
                    ui.separator();
                    ui.label("Parceiros na rede:");
                    if self.discovery.is_none() {
                        ui.label(
                            egui::RichText::new("Descoberta indisponível — use o endereço manual.")
                                .small()
                                .weak(),
                        );
                    } else if peers.is_empty() {
                        ui.label(
                            egui::RichText::new("Procurando hosts na LAN…")
                                .small()
                                .weak(),
                        );
                    } else {
                        for peer in &peers {
                            if ui
                                .button(format!("🔗 {} ({})", peer.name, peer.addr))
                                .clicked()
                            {
                                action = Some(LinkAction::JoinAddr(peer.addr.to_string()));
                            }
                        }
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Os dois lados precisam estar na mesma rede (LAN).")
                            .small()
                            .weak(),
                    );
                }
            });
        self.show_link = open;

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
                Err(_) => self.set_status("porta inválida"),
            },
            Some(LinkAction::Join) => {
                let addr = self.link_addr.trim().to_string();
                if addr.is_empty() {
                    self.set_status("endereço vazio");
                } else {
                    self.link_pending = Some(link::PendingLink::join(addr));
                }
            }
            Some(LinkAction::JoinAddr(addr)) => {
                self.link_pending = Some(link::PendingLink::join(addr));
            }
            Some(LinkAction::Cancel) => {
                if let Some(p) = &self.link_pending {
                    p.cancel();
                }
                self.link_pending = None;
                self.link_advertiser = None; // para de anunciar
                self.set_status("conexão de link cancelada");
            }
            Some(LinkAction::Disconnect) => {
                self.link = None;
                self.gba.link_configure(false, 0);
                self.set_status("link encerrado");
            }
            None => {}
        }
    }

    /// Roda exatamente 1 frame, lidando com o áudio: com dispositivo, drena as
    /// amostras pro host (ou descarta se `mute`, no fast-forward, pra não estourar
    /// o buffer nem sair em pitch errado); sem dispositivo, só esvazia o APU.
    fn step_frame(&mut self, mute: bool) {
        if let Some(session) = &mut self.link {
            // Link event-driven: o master roda até o jogo armar cada
            // transferência e troca pela rede ali; o child espelha. Um frame de
            // cada lado por chamada. Se o parceiro sumir, degrada pra solo
            // (cabo "puxado").
            if let Err(e) = session.run_frame(&mut self.gba) {
                self.link = None;
                self.gba.link_configure(false, 0);
                self.set_status(format!("link caiu ({e}) — seguindo sem parceiro"));
            }
        } else {
            self.gba.run_frame();
        }
        if let Some(audio) = &mut self.audio {
            if mute {
                self.gba.bus.apu.buffer.clear();
            } else {
                let samples = self.gba.bus.apu.drain();
                audio.push(&samples, auroragba_core::apu::OUTPUT_RATE);
            }
        } else {
            self.gba.bus.apu.buffer.clear();
        }
        self.frame_count += 1;
        if self.frame_count.is_multiple_of(REWIND_INTERVAL_FRAMES) {
            self.push_rewind_snapshot();
        }
    }

    /// Ritmo normal pelo consumo de áudio: roda frames só até repor o buffer-alvo
    /// (no máx. 4 por update, pra não travar se a UI ficar lenta). Como o áudio é
    /// consumido em tempo real, isso ancora a emulação ao tempo real. Sem áudio,
    /// roda 1 frame por update (sincroniza pelo vsync).
    fn run_paced(&mut self) {
        let target = self.audio.as_ref().map(|a| a.target());
        let mut ran = 0;
        loop {
            let go = match (target, self.audio.as_ref()) {
                (Some(t), Some(audio)) => audio.queued() < t && ran < 4,
                _ => ran < 1,
            };
            if !go {
                break;
            }
            self.step_frame(false);
            ran += 1;
        }
    }

    /// Aplica o remapeamento em andamento: Esc cancela; senão grava a primeira
    /// tecla (rebind de teclado) ou o `pad_event` capturado (rebind de gamepad).
    fn apply_rebind(
        &mut self,
        ctx: &egui::Context,
        rebind: Rebind,
        pad_event: Option<gilrs::Button>,
    ) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.rebinding = None;
            return;
        }
        match rebind {
            Rebind::Key(idx) => {
                let key = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => Some(*key),
                        _ => None,
                    })
                });
                if let Some(key) = key {
                    self.input.keys[idx] = key;
                    self.rebinding = None;
                    self.set_status(format!("{} → tecla {}", GBA_NAMES[idx], key.name()));
                }
            }
            Rebind::Pad(idx) => {
                if let Some(b) = pad_event {
                    self.input.pads[idx] = Some(b);
                    self.rebinding = None;
                    self.set_status(format!("{} → pad {}", GBA_NAMES[idx], pad_name(b)));
                }
            }
        }
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
        // Bombeia o gamepad (mantém `is_pressed` atualizado) e, se estamos
        // remapeando um botão, captura a próxima tecla/botão pressionado. Enquanto
        // `configuring`, não repassamos input ao jogo nem disparamos hotkeys (pra
        // a tecla capturada não "vazar" pro emulador).
        let pad_event = self.pump_gamepad();
        let configuring = self.rebinding.is_some();

        // Recebe as capas que o worker da biblioteca terminou; se ainda há
        // pendentes e a janela está aberta, segue repintando pra elas aparecerem.
        let covers_pending = self.library.poll(ctx);
        if covers_pending && self.show_library {
            ctx.request_repaint();
        }
        if let Some(rebind) = self.rebinding {
            self.apply_rebind(ctx, rebind, pad_event);
        }

        // Recebe a sessão de link quando a thread de conexão termina (e segue
        // repintando enquanto ela está pendente, pra a UI animar o spinner).
        self.poll_link(ctx);

        // Hotkeys globais. F5/F9/F12 disparam na borda (key_pressed); rewind e
        // fast-forward agem enquanto a tecla está segurada (key_down). Nenhuma é
        // um binding do GBA por padrão, então não conflitam com os botões.
        let (f5, f9, f12, rewinding, fast_forward) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::F9),
                i.key_pressed(egui::Key::F12),
                i.key_down(egui::Key::R),
                i.key_down(egui::Key::Space),
            )
        });
        if !configuring {
            if f5 {
                self.save_state_slot(self.current_slot);
            }
            if f9 {
                self.load_state_slot(self.current_slot);
            }
            if f12 {
                self.screenshot();
            }
        }

        if self.hunting {
            // Modo caça: o Hunter dirige a emulação (amassa A/Start, reseta entre
            // tentativas). Roda um lote de frames por update pra não travar a UI.
            self.hunt_step();
            self.refresh_texture();
            ctx.request_repaint();
        } else if self.running {
            if configuring {
                // Remapeando controles: roda no ritmo normal, sem repassar input.
                self.run_paced();
            } else {
                self.poll_input(ctx);
                if rewinding {
                    // Volta no tempo consumindo o anel de snapshots.
                    self.rewind_step();
                    self.gba.bus.apu.buffer.clear();
                } else if fast_forward {
                    // Acelera: roda frames (sem o gate de áudio) até gastar o
                    // orçamento de tempo desta janela de update. Assim o ganho não
                    // fica preso em "N frames fixos × refresh" — usa toda a CPU.
                    let start = Instant::now();
                    let mut n = 0;
                    loop {
                        self.step_frame(true);
                        n += 1;
                        if n >= FAST_FORWARD_MAX_FRAMES || start.elapsed() >= FAST_FORWARD_BUDGET {
                            break;
                        }
                    }
                } else {
                    self.run_paced();
                }
            }
            // Alimenta o detector de cursor (debug) com o estado da RAM deste
            // frame; no-op se não estiver rastreando. (O localizador de batalha usa
            // snapshots por botão, não acumulação por frame.)
            if SHOW_RAM_FINDER {
                self.cursor_finder.observe(&self.gba.bus.iwram[..]);
            }

            self.refresh_texture();
            ctx.request_repaint();

            // Persiste o save no máximo ~1×/s (um save no jogo gera milhares de
            // escritas byte-a-byte no Flash; não faz sentido tocar o disco a cada).
            if self.frame_count.is_multiple_of(60) {
                self.flush_save();
            }

            // Mede o fps emulado (frames avançados por segundo de tempo real),
            // reamostrando ~1×/s. Útil pra ver o efeito do fast-forward.
            let elapsed = self.fps_sample.0.elapsed().as_secs_f64();
            if elapsed >= 0.5 {
                self.fps = (self.frame_count - self.fps_sample.1) as f64 / elapsed;
                self.fps_sample = (Instant::now(), self.frame_count);
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
                    if ui.button("📚 Biblioteca de ROMs").clicked() {
                        self.show_library = !self.show_library;
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
                ui.menu_button("Estado", |ui| {
                    ui.label("Slot (● = ocupado):");
                    ui.horizontal(|ui| {
                        for slot in 0..SAVE_SLOTS {
                            let label = if self.slot_exists(slot) {
                                format!("●{}", slot + 1)
                            } else {
                                format!("{}", slot + 1)
                            };
                            ui.selectable_value(&mut self.current_slot, slot, label);
                        }
                    });
                    ui.separator();
                    if ui
                        .button(format!("💾 Salvar no slot {} (F5)", self.current_slot + 1))
                        .clicked()
                    {
                        self.save_state_slot(self.current_slot);
                        ui.close_menu();
                    }
                    let can_load = self.slot_exists(self.current_slot);
                    if ui
                        .add_enabled(
                            can_load,
                            egui::Button::new(format!(
                                "📂 Carregar slot {} (F9)",
                                self.current_slot + 1
                            )),
                        )
                        .clicked()
                    {
                        self.load_state_slot(self.current_slot);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("📷 Captura de tela (F12)").clicked() {
                        self.screenshot();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Configurações", |ui| {
                    if ui.button("🎮 Controles…").clicked() {
                        self.show_input_config = true;
                        ui.close_menu();
                    }
                    if ui.button("🔗 Link…").clicked() {
                        self.show_link = true;
                        ui.close_menu();
                    }
                });
                ui.separator();
                ui.label(format!("Scale: {:.0}x", self.scale));
                ui.add(egui::Slider::new(&mut self.scale, 1.0..=6.0).show_value(false));
            });
        });

        // Rodapé: mensagem efêmera de status (esq.) + lembrete dos atalhos (dir.).
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Velocidade emulada: fps e múltiplo do tempo real (GBA ≈ 59,73 fps).
                if self.running {
                    let speed = self.fps / 59.7275;
                    ui.label(
                        egui::RichText::new(format!("{:.0} fps · {:.1}×", self.fps, speed))
                            .monospace()
                            .small(),
                    );
                    ui.separator();
                }
                if let Some((msg, t)) = &self.status {
                    if t.elapsed() < STATUS_DURATION {
                        ui.label(msg);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(
                            "F5 salvar · F9 carregar · F12 screenshot · Espaço FF · R rewind",
                        )
                        .weak()
                        .small(),
                    );
                });
            });
        });

        // Janela de configuração de controles (quando aberta).
        if self.show_input_config {
            self.input_config_window(ctx);
        }
        // Janela da biblioteca de ROMs (quando aberta).
        if self.show_library {
            self.library_window(ctx);
        }
        // Janela do Link (quando aberta). Repinta de tempos em tempos pra a
        // lista de parceiros na LAN refletir os anúncios que vão chegando.
        if self.show_link {
            self.link_window(ctx);
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // Mantém repintando enquanto remapeia (pra capturar a tecla/botão sem
        // depender de outro evento) ou se a janela está aberta.
        if self.show_input_config {
            ctx.request_repaint();
        }

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

    /// Persiste a configuração de controles no storage do eframe (chamado
    /// periodicamente e ao fechar).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("input_config", self.input.serialize());
        if let Some(dir) = &self.library.dir {
            storage.set_string("library_dir", dir.display().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_config_roundtrips() {
        let cfg = InputConfig::default();
        let parsed = InputConfig::parse(&cfg.serialize());
        assert_eq!(cfg.keys, parsed.keys);
        assert_eq!(cfg.pads, parsed.pads);
    }

    #[test]
    fn input_config_parse_overrides_known_and_ignores_garbage() {
        let mut text = InputConfig::default().serialize();
        text.push_str("key.A=W\nlinha invalida\npad.B=North\nkey.Inexistente=Q\n");
        let cfg = InputConfig::parse(&text);
        assert_eq!(cfg.keys[0], egui::Key::W); // A → W
        assert_eq!(cfg.pads[1], Some(gilrs::Button::North)); // B → North
                                                             // O resto permanece no padrão.
        assert_eq!(cfg.keys[1], InputConfig::default().keys[1]);
    }

    #[test]
    fn pad_name_roundtrips() {
        for b in InputConfig::default().pads.into_iter().flatten() {
            assert_eq!(pad_from_name(pad_name(b)), Some(b));
        }
    }
}
