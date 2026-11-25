//! AuroraGBA — frontend desktop (Linux/Windows).
//!
//! Roda 1 frame por update da UI e exibe o framebuffer da PPU numa textura
//! 240×160 escalada.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use auroragba_core::joypad::Button as GbaButton;
use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};
use auroragba_shiny::games::GameProfile;
use auroragba_shiny::gfx::RomGfx;
use auroragba_shiny::{CheckResult, Hunter};
use eframe::egui;

mod audio;
mod discovery;
mod help_video;
mod i18n;
mod icons;
mod library;
mod link;
mod link_ui;
mod pacing;
mod png;
mod settings;
mod shader;
mod states;
mod theme;
mod widgets;

use auroragba_scale::{Scaler, Upscale};
use i18n::{t, LangPref};
use icons::Icon;
use shader::{Active, MultipassKind, ShaderKind, ShaderRenderer};

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
    // ROM opcional na linha de comando (`auroragba jogo.gba`): abre direto.
    let rom_arg = std::env::args()
        .skip(1)
        .find(|a| a.to_ascii_lowercase().ends_with(".gba"))
        .map(PathBuf::from);

    // Mesmo ícone do launcher Android (gerado por scripts/gen_desktop_icon.py).
    // Vale pra barra de título/tarefas no Windows e no X11. No Wayland o
    // compositor ignora o ícone da janela e usa o do `.desktop` cujo nome bate
    // com o `app_id` — ver assets/linux/auroragba.desktop.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/auroragba.png"))
        .expect("ícone embutido deve ser um PNG válido");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([720.0, 520.0])
            .with_title("AuroraGBA")
            .with_app_id("auroragba")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "AuroraGBA",
        options,
        Box::new(|cc| Box::new(AuroraApp::new(cc, link_session, rom_arg))),
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
    /// Renderizador de shader (glow). `None` se o backend GL não estiver
    /// disponível — nesse caso caímos no desenho via textura egui.
    shader: Option<Arc<Mutex<ShaderRenderer>>>,
    /// Efeito embutido selecionado (persistido). Ignorado enquanto `use_custom`
    /// ou `shader_multipass` estiver ativo.
    shader_kind: ShaderKind,
    /// Efeito multipass selecionado (tem prioridade sobre `shader_kind`/custom).
    shader_multipass: Option<MultipassKind>,
    /// Usar o shader importado de arquivo em vez do embutido `shader_kind`.
    use_custom: bool,
    /// Caminho do `.frag` importado (persistido pra recompilar no próximo boot).
    custom_path: Option<PathBuf>,
    /// Nome do arquivo importado, pra rótulo na UI (vazio = nenhum carregado).
    custom_label: String,
    /// Último erro de compilação do shader importado, exibido na janela de vídeo.
    shader_error: Option<String>,
    /// Filtro de upscale (HQx) aplicado ao framebuffer antes do shader (persistido).
    upscale: Upscale,
    /// Scaler com buffers reutilizáveis + o buffer de saída (RGBA8 ampliado).
    scaler: Scaler,
    scaled: Vec<u8>,

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
    /// Aviso que parou a caça (ex.: time cheio num presente). Fica no painel até
    /// a próxima caça começar.
    hunt_warning: Option<String>,
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
    /// Configurações abertas, e em qual seção.
    settings_open: bool,
    settings_page: settings::Page,
    /// Som ligado e volume (0–100).
    sound_on: bool,
    volume: u32,
    /// Toca o som acelerado no fast-forward (senão fica mudo).
    ff_sound: bool,
    /// Multiplicador do fast-forward (0 = o máximo que a máquina aguentar).
    ff_mult: u32,
    /// Espaço liga/desliga o acelerado (senão acelera só enquanto segurado).
    space_toggle: bool,
    /// Ícone do app pra tela Sobre (carregado na primeira vez).
    about_icon: Option<egui::TextureHandle>,
    /// Janela de Estados aberta, no modo carregar (senão salvar), e o slot
    /// ocupado esperando confirmação pra ser sobrescrito.
    states_open: bool,
    states_load_mode: bool,
    states_confirm: Option<usize>,
    /// Miniaturas dos slots (pela data do arquivo do estado).
    thumb_cache: HashMap<usize, (std::time::SystemTime, Option<egui::TextureHandle>)>,
    /// Janela "❓ Como usar" do Shiny Hunter aberta?
    show_hunt_help: bool,
    /// Player do vídeo de exemplo da ajuda (criado ao abrir a janela; trocado
    /// quando o alvo muda pra um modo com outro vídeo).
    help_player: Option<help_video::VideoPlayer>,
    /// Binding em remapeamento (aguardando a próxima tecla/botão). None = nenhum.
    rebinding: Option<Rebind>,
    /// Biblioteca de ROMs (varredura de pasta + capas).
    library: library::Library,
    /// Tela no centro da janela: o jogo ou a biblioteca.
    view: View,
    /// ROM selecionada na biblioteca (índice em `library.entries`).
    lib_selected: Option<usize>,
    /// Quando cada ROM foi aberta pela última vez (segundos Unix), por caminho.
    last_played: HashMap<PathBuf, u64>,
    /// Acelerar ligado pelo botão da barra (o Espaço acelera enquanto segurado).
    ff_toggle: bool,
    /// Painel do Shiny Hunter aberto (pelo botão da barra).
    show_hunter: bool,
    /// Idioma escolhido (automático = o do sistema).
    lang_pref: LangPref,
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
    /// Ritmo do jogo normal: quantos frames rodar em cada atualização de tela.
    pacer: pacing::Pacer,
    /// Trace de pacing (`AURORAGBA_PACING_TRACE`). `None` = desligado.
    pacing_trace: Option<pacing::PacingTrace>,
}

/// O que ocupa o centro da janela.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Game,
    Library,
}

/// Botão clicado na barra de ferramentas (aplicado depois de desenhar).
enum ToolAction {
    Library,
    BackToGame,
    OpenFile,
    TogglePause,
    Reset,
    ToggleFast,
    Screenshot,
    Link,
    Hunter,
    Settings,
    States,
}

/// Agora em segundos Unix.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "Jogado há 3 h", "Jogado ontem"… a partir do instante em que foi aberto.
fn played_label(when: Option<u64>) -> String {
    let Some(when) = when else {
        return t("Nunca jogado", "Never played", "Nunca jugado").to_string();
    };
    let secs = unix_now().saturating_sub(when);
    let (h, d) = (secs / 3600, secs / 86_400);
    match (h, d) {
        (0, _) => t(
            "Jogado agora há pouco",
            "Played just now",
            "Jugado hace un momento",
        )
        .to_string(),
        (h, 0) => format!("{} {h} h", t("Jogado há", "Played", "Jugado hace")) + t("", " ago", ""),
        (_, 1) => t("Jogado ontem", "Played yesterday", "Jugado ayer").to_string(),
        (_, d) if d < 14 => format!(
            "{} {d} {}",
            t("Há", "", "Hace"),
            t("dias", "days ago", "días")
        )
        .trim()
        .to_string(),
        (_, d) => format!(
            "{} {} {}",
            t("Há", "", "Hace"),
            d / 7,
            t("semanas", "weeks ago", "semanas")
        )
        .trim()
        .to_string(),
    }
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

/// Nome do arquivo (sem diretório) para rótulo na UI; cai no caminho completo.
fn file_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl AuroraApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        link_session: Option<link::LinkSession>,
        rom_arg: Option<PathBuf>,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let lang_pref = cc
            .storage
            .and_then(|s| s.get_string("language"))
            .map(|k| LangPref::from_key(&k))
            .unwrap_or_default();
        lang_pref.apply();
        let last_played: HashMap<PathBuf, u64> = cc
            .storage
            .and_then(|s| s.get_string("last_played"))
            .map(|text| {
                text.lines()
                    .filter_map(|l| {
                        let (secs, path) = l.split_once('\t')?;
                        Some((PathBuf::from(path), secs.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();

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

        // Renderizador de shader (glow). Só existe se o backend GL estiver
        // disponível; senão o desenho cai na textura egui.
        let shader = cc
            .gl
            .as_ref()
            .map(|gl| Arc::new(Mutex::new(ShaderRenderer::new(gl))));
        let stored_shader = cc
            .storage
            .and_then(|s| s.get_string("video_shader"))
            .unwrap_or_default();
        let shader_kind = ShaderKind::from_key(&stored_shader);
        // Um efeito multipass persistido (ex.: "blur") tem prioridade na seleção.
        let shader_multipass = MultipassKind::from_key(&stored_shader);

        // Se o último shader era um importado, tenta recompilá-lo do arquivo
        // persistido. Se o arquivo sumiu ou não compila, cai no embutido.
        let mut use_custom = false;
        let mut custom_path: Option<PathBuf> = None;
        let mut custom_label = String::new();
        if stored_shader == "custom" {
            let path = cc
                .storage
                .and_then(|s| s.get_string("video_shader_path"))
                .map(PathBuf::from);
            if let (Some(renderer), Some(gl), Some(path)) = (shader.as_ref(), cc.gl.as_ref(), path)
            {
                match std::fs::read_to_string(&path) {
                    Ok(src) => match renderer.lock().unwrap().load_custom(gl, &src) {
                        Ok(()) => {
                            use_custom = true;
                            custom_label = file_label(&path);
                            custom_path = Some(path);
                        }
                        Err(e) => log::warn!("shader importado não compilou: {e}"),
                    },
                    Err(e) => log::warn!("shader importado não pôde ser lido: {e}"),
                }
            }
        }
        // Escala inteira 1–6, ou 0 = ajustar à janela.
        let scale = cc
            .storage
            .and_then(|s| s.get_string("video_scale"))
            .and_then(|s| {
                if s == "fit" {
                    Some(0.0)
                } else {
                    s.parse::<f32>().ok()
                }
            })
            .filter(|s| *s == 0.0 || (1.0..=6.0).contains(s))
            .unwrap_or(3.0);
        let get_bool = |key: &str, default: bool| {
            cc.storage
                .and_then(|s| s.get_string(key))
                .map(|v| v == "1")
                .unwrap_or(default)
        };
        let upscale = cc
            .storage
            .and_then(|s| s.get_string("video_upscale"))
            .map(|k| Upscale::from_key(&k))
            .unwrap_or_default();

        let mut app = Self {
            gba: Gba::new(),
            rom_path: None,
            texture,
            running: false,
            scale,
            shader,
            shader_kind,
            shader_multipass,
            use_custom,
            custom_path,
            custom_label,
            shader_error: None,
            upscale,
            scaler: Scaler::new(),
            scaled: Vec::new(),

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
            hunt_warning: None,
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
            settings_open: false,
            settings_page: settings::Page::default(),
            sound_on: get_bool("sound_on", true),
            volume: cc
                .storage
                .and_then(|s| s.get_string("volume"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(100u32)
                .min(100),
            ff_sound: get_bool("ff_sound", false),
            ff_mult: cc
                .storage
                .and_then(|s| s.get_string("ff_mult"))
                .and_then(|s| s.parse().ok())
                .filter(|m| [0u32, 2, 4, 8].contains(m))
                .unwrap_or(0),
            space_toggle: get_bool("space_toggle", false),
            about_icon: None,
            states_open: false,
            states_load_mode: false,
            states_confirm: None,
            thumb_cache: HashMap::new(),
            show_hunt_help: false,
            help_player: None,
            rebinding: None,
            library: library::Library::new(cache_dir),
            view: View::Library,
            lib_selected: None,
            last_played,
            ff_toggle: false,
            show_hunter: false,
            lang_pref,
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
            pacer: pacing::Pacer::new(),
            pacing_trace: pacing::PacingTrace::from_env(),
        };

        app.apply_audio_gain();

        if let Some(s) = &app.link {
            app.set_status(if s.id == 0 {
                t(
                    "Link ativo: você hospeda",
                    "Link active: you are the host",
                    "Link activo: eres el anfitrión",
                )
            } else {
                t(
                    "Link ativo: você é o convidado",
                    "Link active: you are the guest",
                    "Link activo: eres el invitado",
                )
            });
        }

        // Restaura a última pasta de ROMs: sem ROM na linha de comando, a janela
        // abre na biblioteca.
        if let Some(dir) = saved_lib_dir {
            if dir.is_dir() {
                app.library.scan(dir);
                app.select_last_played();
            }
        }
        if let Some(path) = rom_arg {
            app.open_rom(path);
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
                self.view = View::Game;
                self.ff_toggle = false;
                self.show_hunter = false;
                self.last_played.insert(path.clone(), unix_now());
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
                self.thumb_cache.clear();
                self.rewind.clear();
                match self.profile {
                    Some(p) => log::info!("Jogo reconhecido: {} ({code})", p.name),
                    None => log::info!("Jogo não reconhecido pelo Shiny Hunter (code={code})"),
                }
            }
            Err(e) => {
                log::error!("Falha ao abrir ROM: {e}");
                self.set_status(format!(
                    "{} {}: {e}",
                    t("Não deu pra abrir", "Couldn't open", "No se pudo abrir"),
                    file_label(&path)
                ));
            }
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
            self.set_status(t(
                "Carregue uma ROM antes de salvar estado",
                "Load a ROM before saving a state",
                "Carga una ROM antes de guardar un estado",
            ));
            return;
        }
        let blob = self.gba.save_state();
        match self.slot_path(slot) {
            Some(path) => match std::fs::write(&path, &blob) {
                Ok(()) => {
                    self.save_slot_thumb(slot);
                    self.set_status(format!(
                        "{} {}",
                        t(
                            "Estado salvo no slot",
                            "State saved to slot",
                            "Estado guardado en la ranura"
                        ),
                        slot + 1
                    ));
                }
                Err(e) => self.set_status(format!(
                    "{}: {e}",
                    t(
                        "Falha ao salvar estado",
                        "Failed to save state",
                        "Error al guardar el estado"
                    )
                )),
            },
            None => self.set_status(t(
                "Carregue uma ROM antes de salvar estado",
                "Load a ROM before saving a state",
                "Carga una ROM antes de guardar un estado",
            )),
        }
    }

    /// Carrega o estado do slot informado por cima do jogo atual. Recusa estados
    /// de outro jogo (o cabeçalho do save state guarda o game code).
    fn load_state_slot(&mut self, slot: usize) {
        let Some(path) = self.slot_path(slot) else {
            self.set_status(t(
                "Carregue uma ROM antes de carregar estado",
                "Load a ROM before loading a state",
                "Carga una ROM antes de cargar un estado",
            ));
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => match self.gba.load_state(&bytes) {
                Ok(()) => {
                    // O estado restaurado é um novo "agora": o anel de rewind
                    // antigo não bate mais com esta linha do tempo.
                    self.rewind.clear();
                    self.running = true;
                    self.set_status(format!(
                        "{} {}",
                        t(
                            "Estado carregado do slot",
                            "State loaded from slot",
                            "Estado cargado de la ranura"
                        ),
                        slot + 1
                    ));
                }
                Err(e) => self.set_status(format!(
                    "{}: {e}",
                    t(
                        "Falha ao carregar estado",
                        "Failed to load state",
                        "Error al cargar el estado"
                    )
                )),
            },
            Err(_) => self.set_status(format!(
                "{} {} {}",
                t("Slot", "Slot", "Ranura"),
                slot + 1,
                t("vazio", "is empty", "vacía")
            )),
        }
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
            self.set_status(t(
                "Carregue uma ROM antes de capturar a tela",
                "Load a ROM before taking a screenshot",
                "Carga una ROM antes de capturar la pantalla",
            ));
            return;
        };
        let dir = rom
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("screenshots");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.set_status(format!(
                "{}: {e}",
                t(
                    "Falha ao criar a pasta de capturas",
                    "Failed to create the screenshots folder",
                    "Error al crear la carpeta de capturas"
                )
            ));
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
            Ok(()) => self.set_status(format!(
                "{} {}",
                t(
                    "Captura salva em",
                    "Screenshot saved to",
                    "Captura guardada en"
                ),
                path.display()
            )),
            Err(e) => self.set_status(format!(
                "{}: {e}",
                t(
                    "Falha ao salvar a captura",
                    "Failed to save the screenshot",
                    "Error al guardar la captura"
                )
            )),
        }
    }

    /// Inicia a caça com o alvo selecionado. O jogador deve estar **parado na
    /// frente do alvo** com o save carregado; a primeira tentativa amassa A até
    /// a batalha, e as seguintes resetam sozinhas.
    fn start_hunt(&mut self) {
        if self.profile.is_some() {
            self.hunter = Hunter::new();
            self.hunt_warning = None;
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
        // Pra alvos de espécie fixa (lendário/inicial) é a própria espécie; pra
        // selvagem "qualquer" (species 0) mostramos a espécie do ÚLTIMO encontro —
        // assim o painel exibe quem apareceu (e, ao parar, o shiny achado).
        ui.separator();
        let ctx = ui.ctx().clone();
        let display_species = if target.species != 0 {
            target.species
        } else {
            self.hunter.last_species
        };
        let normal_tex = self.target_sprite(&ctx, display_species, false);
        let shiny_tex = self.target_sprite(&ctx, display_species, true);
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

        if let Some(w) = &self.hunt_warning {
            ui.label(egui::RichText::new(w).color(egui::Color32::from_rgb(0xE0, 0x80, 0x40)));
        }

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
        // Iniciar/parar à esquerda; "Como usar" no lado oposto.
        ui.horizontal(|ui| {
            if self.hunting {
                if ui.button("⏹ Parar caça").clicked() {
                    self.hunting = false;
                }
            } else if ui.button("▶ Iniciar caça").clicked() {
                self.start_hunt();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("❓ Como usar")
                    .on_hover_text("Passo a passo e vídeo de exemplo deste modo de caça")
                    .clicked()
                {
                    self.show_hunt_help = true;
                }
            });
        });

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
        if result == CheckResult::PartyFull {
            // Presente com o time cheio: ele iria pro PC (invisível pra caça) e o
            // A-mash seguiria até a tela de apelido. Para e explica.
            self.hunting = false;
            self.running = true;
            let msg = "⚠ Time cheio: o presente iria para o PC, onde a caça não o vê. \
                       Libere um espaço no time, salve em frente ao presente e comece \
                       a caça de novo.";
            self.set_status(msg);
            self.hunt_warning = Some(msg.into());
        }
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

    /// Janela "❓ Como usar" do Shiny Hunter: passo a passo do modo do alvo
    /// selecionado e, logo abaixo, o vídeo de exemplo (quando existe). O conteúdo
    /// vem de `auroragba_shiny::help` — o mesmo texto que o Android mostra.
    fn hunt_help_window(&mut self, ctx: &egui::Context) {
        let Some(profile) = self.profile else {
            self.show_hunt_help = false;
            return;
        };
        let target = profile.targets[self.selected_target];
        // Desktop ainda só em português (o seletor de idioma dele vem depois).
        let help =
            auroragba_shiny::help::help_for(profile, &target, auroragba_shiny::lang::Lang::Pt);

        // Player do vídeo deste modo: cria ao abrir; troca se o alvo mudou de modo.
        match (help.video, &self.help_player) {
            (Some(v), Some(p)) if p.video() == v => {}
            (Some(v), _) => self.help_player = help_video::VideoPlayer::new(v),
            (None, _) => self.help_player = None,
        }

        let mut open = self.show_hunt_help;
        egui::Window::new(format!("❓ Como usar — {}", help.title))
            .id(egui::Id::new("hunt_help"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(egui::RichText::new(target.name).strong());
                ui.add_space(4.0);
                for para in help.body.split("\n\n") {
                    ui.label(para);
                    ui.add_space(6.0);
                }
                ui.separator();
                match &mut self.help_player {
                    Some(player) => player.show(ui, 560.0),
                    None if help.video.is_some() => {
                        ui.label("Não foi possível iniciar o vídeo de exemplo.");
                    }
                    None => {
                        ui.label(
                            egui::RichText::new(auroragba_shiny::help::NO_VIDEO_NOTE[0]).weak(),
                        );
                    }
                }
            });
        self.show_hunt_help = open;
        if !open {
            // Libera o decoder e a textura ao fechar.
            self.help_player = None;
        }
    }

    /// Nome do jogo pra mostrar: o do perfil do Shiny Hunter (ex.: "Pokémon
    /// Emerald") ou o nome do arquivo, sem `_`.
    fn display_name(path: &std::path::Path, code: &str) -> String {
        if let Some(p) = auroragba_shiny::games::detect(code) {
            return p.name.to_string();
        }
        path.file_stem()
            .map(|s| s.to_string_lossy().replace('_', " ").trim().to_string())
            .unwrap_or_default()
    }

    /// Nome do jogo carregado (barra de ferramentas).
    fn game_title(&self) -> String {
        match &self.rom_path {
            Some(p) => Self::display_name(p, &self.gba.bus.cartridge.game_code()),
            None => String::new(),
        }
    }

    /// Pré-seleciona na biblioteca o último jogo aberto (ou o primeiro).
    fn select_last_played(&mut self) {
        let last = self
            .library
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| self.last_played.get(&e.path).map(|&t| (i, t)))
            .max_by_key(|&(_, t)| t)
            .map(|(i, _)| i);
        self.lib_selected = last.or(if self.library.entries.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    /// Abre o seletor de arquivo e carrega a ROM escolhida.
    fn pick_rom_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("GBA ROM", &["gba"])
            .pick_file()
        {
            self.open_rom(path);
        }
    }

    /// Escolhe a pasta da biblioteca e a varre.
    fn pick_library_dir(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.library.scan(dir);
            self.select_last_played();
        }
    }

    /// Barra de ferramentas do topo: nome do jogo à esquerda, ações à direita
    /// (só os ícones quando a janela é estreita demais pros nomes).
    fn toolbar(&mut self, ctx: &egui::Context) {
        let mut action: Option<ToolAction> = None;
        let has_rom = self.rom_path.is_some();
        let hunting = self.hunting;
        let frame = egui::Frame::none()
            .fill(theme::SURFACE)
            .inner_margin(egui::Margin {
                left: 16.0,
                right: 12.0,
                top: 0.0,
                bottom: 0.0,
            });
        egui::TopBottomPanel::top("toolbar")
            .exact_height(56.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let lib_label = t("Biblioteca", "Library", "Biblioteca");
                    let pause_label = if self.running {
                        t("Pausar", "Pause", "Pausar")
                    } else {
                        t("Continuar", "Resume", "Continuar")
                    };
                    let reset_label = t("Reiniciar", "Restart", "Reiniciar");
                    let ff_on_label = if self.ff_mult == 0 {
                        t("Acelerado", "Fast", "Acelerado").to_string()
                    } else {
                        format!("{} {}×", t("Acelerado", "Fast", "Acelerado"), self.ff_mult)
                    };
                    let ff_label: &str = if self.ff_toggle {
                        &ff_on_label
                    } else {
                        t("Acelerar", "Fast forward", "Acelerar")
                    };
                    let states_label = t("Estados", "States", "Estados");
                    let shot_label = t("Captura", "Screenshot", "Captura");
                    let link_label = t("Link", "Link", "Link");
                    let hunter_label = "Shiny Hunter";
                    let cfg_label = t("Configurações", "Settings", "Ajustes");
                    let open_label = t("Abrir arquivo…", "Open file…", "Abrir archivo…");
                    let back_label = t("Voltar ao jogo", "Back to game", "Volver al juego");

                    let labels: Vec<&str> = match self.view {
                        View::Game => {
                            let mut v = vec![
                                lib_label,
                                pause_label,
                                reset_label,
                                ff_label,
                                states_label,
                                shot_label,
                                link_label,
                                cfg_label,
                            ];
                            if self.profile.is_some() {
                                v.push(hunter_label);
                            }
                            v
                        }
                        View::Library => vec![cfg_label, open_label, back_label],
                    };
                    let needed: f32 = labels
                        .iter()
                        .map(|l| widgets::tool_button_width(ui, l) + 4.0)
                        .sum::<f32>()
                        + 3.0 * 13.0;
                    // O título precisa de espaço pro nome do jogo (até ~220 px).
                    let title_w = ui
                        .painter()
                        .layout_no_wrap(self.game_title(), theme::bold(15.0), theme::TEXT)
                        .size()
                        .x
                        .clamp(100.0, 220.0);
                    let compact = needed > ui.available_width() - title_w - 16.0;

                    match self.view {
                        View::Game => {
                            if widgets::tool_button(
                                ui,
                                Icon::Gear,
                                cfg_label,
                                self.settings_open,
                                true,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Settings);
                            }
                            if self.profile.is_some()
                                && widgets::tool_button(
                                    ui,
                                    Icon::Sparkle,
                                    hunter_label,
                                    self.show_hunter || hunting,
                                    !hunting,
                                    compact,
                                )
                                .clicked()
                            {
                                action = Some(ToolAction::Hunter);
                            }
                            if widgets::tool_button(
                                ui,
                                Icon::Link,
                                link_label,
                                self.show_link,
                                !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Link);
                            }
                            widgets::tool_separator(ui);
                            if widgets::tool_button(
                                ui,
                                Icon::Camera,
                                shot_label,
                                false,
                                has_rom,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Screenshot);
                            }
                            if widgets::tool_button(
                                ui,
                                Icon::States,
                                states_label,
                                self.states_open,
                                has_rom && !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::States);
                            }
                            widgets::tool_separator(ui);
                            if widgets::tool_button(
                                ui,
                                Icon::FastForward,
                                ff_label,
                                self.ff_toggle,
                                has_rom && !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::ToggleFast);
                            }
                            if widgets::tool_button(
                                ui,
                                Icon::Reset,
                                reset_label,
                                false,
                                has_rom && !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Reset);
                            }
                            let pause_icon = if self.running {
                                Icon::Pause
                            } else {
                                Icon::Play
                            };
                            if widgets::tool_button(
                                ui,
                                pause_icon,
                                pause_label,
                                false,
                                has_rom && !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::TogglePause);
                            }
                            widgets::tool_separator(ui);
                            if widgets::tool_button(
                                ui,
                                Icon::Library,
                                lib_label,
                                false,
                                !hunting,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Library);
                            }
                        }
                        View::Library => {
                            if widgets::tool_button(
                                ui,
                                Icon::Gear,
                                cfg_label,
                                self.settings_open,
                                true,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::Settings);
                            }
                            if widgets::tool_button(
                                ui,
                                Icon::File,
                                open_label,
                                false,
                                true,
                                compact,
                            )
                            .clicked()
                            {
                                action = Some(ToolAction::OpenFile);
                            }
                            if has_rom
                                && widgets::tool_button(
                                    ui,
                                    Icon::Back,
                                    back_label,
                                    false,
                                    true,
                                    compact,
                                )
                                .clicked()
                            {
                                action = Some(ToolAction::BackToGame);
                            }
                        }
                    }

                    // O que sobrou à esquerda: "AURORAGBA" + nome do jogo/tela.
                    let title = match self.view {
                        View::Library => lib_label.to_string(),
                        View::Game if has_rom => self.game_title(),
                        View::Game => String::new(),
                    };
                    let w = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::vec2(w, 40.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add_space(3.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(egui::Label::new(
                                egui::RichText::new("AURORAGBA")
                                    .font(theme::semibold(11.0))
                                    .color(theme::MUTED),
                            ));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(title)
                                        .font(theme::bold(15.0))
                                        .color(theme::TEXT),
                                )
                                .truncate(true),
                            );
                        },
                    );
                });
            });

        match action {
            Some(ToolAction::Library) => {
                self.view = View::Library;
                self.select_last_played();
            }
            Some(ToolAction::BackToGame) => self.view = View::Game,
            Some(ToolAction::OpenFile) => self.pick_rom_file(),
            Some(ToolAction::TogglePause) => {
                self.running = !self.running;
                self.pacer.reset();
            }
            Some(ToolAction::Reset) => {
                if let Some(p) = self.rom_path.clone() {
                    self.open_rom(p);
                }
            }
            Some(ToolAction::ToggleFast) => {
                self.ff_toggle = !self.ff_toggle;
                self.pacer.reset();
            }
            Some(ToolAction::Screenshot) => self.screenshot(),
            Some(ToolAction::Link) => self.show_link = !self.show_link,
            Some(ToolAction::Hunter) => self.show_hunter = !self.show_hunter,
            Some(ToolAction::Settings) => self.settings_open = true,
            Some(ToolAction::States) => {
                self.states_open = true;
                self.states_load_mode = false;
                self.states_confirm = None;
            }
            None => {}
        }
    }

    /// Rodapé: velocidade, mensagem efêmera e lembrete dos atalhos.
    fn status_bar(&mut self, ctx: &egui::Context) {
        let frame = egui::Frame::none()
            .fill(theme::STATUS_BG)
            .inner_margin(egui::Margin::symmetric(16.0, 0.0));
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let small = |s: String, c: egui::Color32| {
                        egui::Label::new(egui::RichText::new(s).font(theme::regular(12.0)).color(c))
                    };
                    let in_game = self.rom_path.is_some() && self.view == View::Game;
                    if in_game {
                        let speed = if (self.running && !self.settings_open && !self.states_open) || self.hunting {
                            let txt = format!("{:.0} fps · {:.1}×", self.fps, self.fps / 59.7275);
                            if i18n::get() == i18n::Lang::En {
                                txt
                            } else {
                                txt.replace('.', ",")
                            }
                        } else {
                            t("Pausado", "Paused", "En pausa").to_string()
                        };
                        ui.add(small(speed, theme::TEXT_SOFT));
                        let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 0.0, egui::Color32::from_rgb(0x33, 0x33, 0x3D));
                    }
                    let msg = match &self.status {
                        Some((msg, at)) if at.elapsed() < STATUS_DURATION => Some(msg.clone()),
                        _ if self.hunting => Some(
                            t(
                                "Caça em andamento: pare pelo painel para voltar a jogar",
                                "Hunt in progress: stop it from the panel to play again",
                                "Caza en curso: detenla desde el panel para volver a jugar",
                            )
                            .to_string(),
                        ),
                        _ => None,
                    };
                    if let Some(msg) = msg {
                        ui.add(small(msg, theme::MUTED).truncate(true));
                    }
                    if in_game && !self.hunting {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(small(
                                t(
                                    "F5 salvar · F9 carregar · F12 captura · Espaço acelerar · R voltar no tempo",
                                    "F5 save · F9 load · F12 screenshot · Space fast forward · R rewind",
                                    "F5 guardar · F9 cargar · F12 captura · Espacio acelerar · R retroceder",
                                )
                                .to_string(),
                                theme::FAINT,
                            ));
                        });
                    }
                });
            });
    }

    /// A tela do jogo, em escala inteira, centralizada na área livre.
    fn game_view(&mut self, ui: &mut egui::Ui, gl: Option<&Arc<eframe::glow::Context>>) {
        let area = ui.max_rect();
        // "Ajustar" (escala 0): a maior escala inteira que cabe na área livre.
        let k = if self.scale < 1.0 {
            (area.width() / SCREEN_WIDTH as f32)
                .min(area.height() / SCREEN_HEIGHT as f32)
                .floor()
                .max(1.0)
        } else {
            self.scale.round()
        };
        let size = egui::vec2(SCREEN_WIDTH as f32 * k, SCREEN_HEIGHT as f32 * k);
        // Alinha ao pixel físico pra a escala inteira não borrar.
        let ppp = ui.ctx().pixels_per_point();
        let min = ((area.center() - size / 2.0) * ppp).round() / ppp;
        let rect = egui::Rect::from_min_size(min, size);
        if self.rom_path.is_none() {
            return;
        }
        match (&self.shader, gl) {
            (Some(renderer), Some(gl)) => {
                // Filtro de upscale (HQx) antes do shader: amplia o framebuffer
                // na CPU; sem filtro, `scale` só copia.
                let (iw, ih) = self.scaler.scale(
                    self.upscale,
                    &self.gba.bus.ppu.framebuffer[..],
                    SCREEN_WIDTH,
                    SCREEN_HEIGHT,
                    &mut self.scaled,
                );
                renderer
                    .lock()
                    .unwrap()
                    .upload(gl, iw as i32, ih as i32, &self.scaled);
                let cb = shader::callback(
                    rect,
                    renderer.clone(),
                    self.shader_active(),
                    [iw as f32, ih as f32],
                    self.frame_count as i32,
                );
                ui.painter().add(cb);
            }
            _ => {
                // Fallback: textura egui (backend sem glow).
                egui::Image::new(&self.texture).paint_at(ui, rect);
            }
        }

        if !self.running && !self.hunting {
            let msg = t(
                "Pausado · clique em Continuar para voltar",
                "Paused · click Resume to continue",
                "En pausa · haz clic en Continuar para seguir",
            );
            let galley =
                ui.painter()
                    .layout_no_wrap(msg.to_owned(), theme::semibold(13.0), theme::TEXT);
            let pill = egui::Rect::from_center_size(
                egui::pos2(area.center().x, area.top() + 20.0 + 17.0),
                galley.size() + egui::vec2(32.0, 16.0),
            );
            let p = ui.painter();
            p.rect_filled(pill, 999.0, theme::SURFACE2);
            p.rect_stroke(pill, 999.0, egui::Stroke::new(1.0_f32, theme::BORDER));
            p.galley(pill.center() - galley.size() / 2.0, galley, theme::TEXT);
        }
    }

    /// Biblioteca em tela cheia: a tela inicial sem ROM (e o botão Biblioteca).
    fn library_page(&mut self, ui: &mut egui::Ui) {
        let mut launch: Option<PathBuf> = None;
        let mut pick_dir = false;
        let mut open_file = false;

        let Some(dir) = self.library.dir.clone() else {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.28).max(24.0));
                ui.label(
                    egui::RichText::new(t(
                        "Escolha a pasta dos seus jogos",
                        "Choose your games folder",
                        "Elige la carpeta de tus juegos",
                    ))
                    .font(theme::bold(26.0))
                    .color(theme::TEXT),
                );
                ui.label(
                    egui::RichText::new(t(
                        "O AuroraGBA mostra as ROMs .gba da pasta com capa. Os saves ficam ao lado de cada jogo.",
                        "AuroraGBA lists the folder's .gba ROMs with covers. Saves stay next to each game.",
                        "AuroraGBA muestra las ROM .gba de la carpeta con portada. Las partidas quedan junto a cada juego.",
                    ))
                    .color(theme::MUTED),
                );
                ui.add_space(16.0);
                if widgets::accent_button(ui, t("Escolher pasta…", "Choose folder…", "Elegir carpeta…"), 44.0)
                    .clicked()
                {
                    pick_dir = true;
                }
                ui.add_space(4.0);
                if widgets::outline_button(ui, t("Abrir um arquivo…", "Open a single file…", "Abrir un archivo…"), 40.0)
                    .clicked()
                {
                    open_file = true;
                }
            });
            if pick_dir {
                self.pick_library_dir();
            }
            if open_file {
                self.pick_rom_file();
            }
            return;
        };

        // Cabeçalho: título, pasta e "Trocar pasta…".
        let count = self.library.entries.len();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(
                    egui::RichText::new(t("Seus jogos", "Your games", "Tus juegos"))
                        .font(theme::bold(26.0))
                        .color(theme::TEXT),
                );
                let mut shown = dir.display().to_string();
                if let Some(home) =
                    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
                {
                    let home = PathBuf::from(home).display().to_string();
                    if !home.is_empty() && shown.starts_with(&home) {
                        shown = format!("~{}", &shown[home.len()..]);
                    }
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{shown} · {count} ROMs · {}",
                        t(
                            "clique duas vezes para jogar",
                            "double-click to play",
                            "doble clic para jugar"
                        )
                    ))
                    .font(theme::regular(13.0))
                    .color(theme::MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                let link = ui.add(
                    egui::Label::new(
                        egui::RichText::new(t(
                            "Trocar pasta…",
                            "Change folder…",
                            "Cambiar carpeta…",
                        ))
                        .font(theme::semibold(13.0))
                        .color(theme::ACCENT),
                    )
                    .sense(egui::Sense::click()),
                );
                if link
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    pick_dir = true;
                }
            });
        });
        ui.add_space(12.0);

        if count == 0 {
            ui.label(
                egui::RichText::new(t(
                    "Nenhuma ROM .gba nesta pasta.",
                    "No .gba ROMs in this folder.",
                    "No hay ROM .gba en esta carpeta.",
                ))
                .color(theme::MUTED),
            );
        } else {
            let gap = 20.0;
            let avail = ui.available_width();
            let cols = (((avail + gap) / (170.0 + gap)).floor() as usize).clamp(2, 8);
            let card_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
            let cover = card_w - 20.0;
            let card_h = 10.0 + cover + 10.0 + 38.0 + 18.0 + 30.0;
            let scroll_h = (ui.available_height() - 64.0).max(120.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    for row in 0..count.div_ceil(cols) {
                        ui.horizontal(|ui| {
                            for i in row * cols..((row + 1) * cols).min(count) {
                                let (clicked, double) = self.library_card(ui, i, card_w, card_h);
                                if clicked {
                                    self.lib_selected = Some(i);
                                }
                                if double {
                                    launch = Some(self.library.entries[i].path.clone());
                                }
                            }
                        });
                    }
                });

            // "Jogar <jogo>" embaixo, centralizado.
            if let Some(i) = self.lib_selected.filter(|&i| i < count) {
                let e = &self.library.entries[i];
                let name = Self::display_name(&e.path, &e.code);
                let path = e.path.clone();
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    let label = format!("{} {name}", t("Jogar", "Play", "Jugar"));
                    if widgets::accent_button(ui, &label, 44.0).clicked() {
                        launch = Some(path);
                    }
                });
            }
        }

        if pick_dir {
            self.pick_library_dir();
        }
        if let Some(path) = launch {
            self.open_rom(path);
        }
    }

    /// Um cartão da biblioteca: capa, nome, quando jogou e o selo do Shiny
    /// Hunter (jogo suportado). Devolve (clicado, clique duplo).
    fn library_card(&self, ui: &mut egui::Ui, i: usize, w: f32, h: f32) -> (bool, bool) {
        let entry = &self.library.entries[i];
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
        let resp = resp.on_hover_text(entry.path.display().to_string());
        if ui.is_rect_visible(rect) {
            let selected = self.lib_selected == Some(i);
            let p = ui.painter();
            p.rect_filled(rect, 14.0, theme::SURFACE);
            if selected {
                p.rect_stroke(
                    rect.shrink(1.0),
                    14.0,
                    egui::Stroke::new(2.0_f32, theme::ACCENT),
                );
            } else if resp.hovered() {
                p.rect_stroke(
                    rect.shrink(0.5),
                    14.0,
                    egui::Stroke::new(1.0_f32, theme::BORDER),
                );
            }
            let cover_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(10.0, 10.0),
                egui::vec2(w - 20.0, w - 20.0),
            );
            p.rect_filled(cover_rect, 10.0, egui::Color32::from_rgb(0x1E, 0x2A, 0x36));
            match &entry.cover {
                Some(tex) => {
                    // Preserva o aspecto (box art é retrato, screenshot é paisagem).
                    let ts = tex.size_vec2();
                    let s = (cover_rect.width() / ts.x).min(cover_rect.height() / ts.y);
                    let r = egui::Rect::from_center_size(cover_rect.center(), ts * s);
                    egui::Image::new(tex).rounding(8.0).paint_at(ui, r);
                }
                None => {
                    p.text(
                        cover_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "…",
                        theme::regular(22.0),
                        theme::FAINT,
                    );
                }
            }
            let x = rect.left() + 12.0;
            let mut y = cover_rect.bottom() + 10.0;
            let name = Self::display_name(&entry.path, &entry.code);
            let mut job =
                egui::text::LayoutJob::simple(name, theme::semibold(14.0), theme::TEXT, w - 24.0);
            job.wrap.max_rows = 2;
            let galley = ui.fonts(|f| f.layout_job(job));
            let name_h = galley.size().y;
            ui.painter().galley(egui::pos2(x, y), galley, theme::TEXT);
            y += name_h.max(19.0) + 4.0;
            let p = ui.painter();
            p.text(
                egui::pos2(x, y),
                egui::Align2::LEFT_TOP,
                played_label(self.last_played.get(&entry.path).copied()),
                theme::regular(12.0),
                theme::MUTED,
            );
            if auroragba_shiny::games::detect(&entry.code).is_some() {
                let label = "Shiny Hunter";
                let galley =
                    p.layout_no_wrap(label.to_owned(), theme::semibold(11.0), theme::ACCENT);
                let badge = egui::Rect::from_min_size(
                    egui::pos2(x - 2.0, rect.bottom() - 32.0),
                    egui::vec2(galley.size().x + 32.0, 22.0),
                );
                p.rect_filled(badge, 999.0, theme::ACCENT_SOFT);
                icons::paint(
                    p,
                    egui::Rect::from_center_size(
                        egui::pos2(badge.left() + 15.0, badge.center().y),
                        egui::vec2(11.0, 11.0),
                    ),
                    Icon::Sparkle,
                    theme::ACCENT,
                );
                p.galley(
                    egui::pos2(
                        badge.left() + 24.0,
                        badge.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    theme::ACCENT,
                );
            }
        }
        (resp.clicked(), resp.double_clicked())
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
                self.gba.link_configure(true, session.id);
                self.set_status(if session.id == 0 {
                    t(
                        "Link conectado: você hospeda",
                        "Link connected: you are the host",
                        "Link conectado: eres el anfitrión",
                    )
                } else {
                    t(
                        "Link conectado: você é o convidado",
                        "Link connected: you are the guest",
                        "Link conectado: eres el invitado",
                    )
                });
                self.link = Some(session);
                self.link_pending = None;
                self.link_advertiser = None; // conectou: para de se anunciar
            }
            Some(Err(e)) => {
                // `Interrupted` = cancelamento pedido pelo usuário, sem alarde.
                if e.kind() != std::io::ErrorKind::Interrupted {
                    self.set_status(format!(
                        "{} ({e})",
                        t("O link falhou", "The link failed", "El link falló")
                    ));
                }
                self.link_pending = None;
                self.link_advertiser = None;
            }
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
                self.set_status(format!(
                    "{} ({e})",
                    t(
                        "O link caiu: seguindo sem parceiro",
                        "The link dropped: playing solo",
                        "El link se cayó: sigues sin compañero"
                    )
                ));
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

    /// Ritmo normal, guiado pelo monitor: o [`pacing::Pacer`] diz quantos frames
    /// rodar nesta atualização de tela (normalmente 0 ou 1, em cadência
    /// uniforme) e o áudio reamostra na velocidade resultante. Devolve quantos
    /// frames rodou.
    fn run_paced(&mut self) -> u32 {
        let frames = self.pacer.tick(Instant::now());
        if let Some(audio) = &mut self.audio {
            audio.set_speed(self.pacer.speed());
        }
        for _ in 0..frames {
            self.step_frame(false);
        }
        frames
    }

    /// Acelerado. Com multiplicador, roda N× os frames que o ritmo normal
    /// rodaria (e o som, se ligado, toca acelerado); no "Máximo", roda frames
    /// sem som até gastar o orçamento de tempo desta atualização de tela, sem
    /// ficar preso em "N frames × refresh".
    fn run_fast(&mut self) {
        if self.ff_mult == 0 {
            let start = Instant::now();
            let mut n = 0;
            loop {
                self.step_frame(true);
                n += 1;
                if n >= FAST_FORWARD_MAX_FRAMES || start.elapsed() >= FAST_FORWARD_BUDGET {
                    break;
                }
            }
            self.pacer.reset();
            return;
        }
        let frames = self.pacer.tick(Instant::now()) * self.ff_mult;
        if let Some(audio) = &mut self.audio {
            audio.set_speed(self.pacer.speed() * self.ff_mult as f64);
        }
        for _ in 0..frames {
            self.step_frame(!self.ff_sound);
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
                    self.set_status(format!(
                        "{} → {} {}",
                        settings::button_label(idx),
                        t("tecla", "key", "tecla"),
                        key.name()
                    ));
                }
            }
            Rebind::Pad(idx) => {
                if let Some(b) = pad_event {
                    self.input.pads[idx] = Some(b);
                    self.rebinding = None;
                    self.set_status(format!(
                        "{} → {} {}",
                        settings::button_label(idx),
                        t("controle", "controller", "mando"),
                        pad_name(b)
                    ));
                }
            }
        }
    }

    /// Abre um seletor de arquivo, lê o `.frag` e o compila como shader custom.
    /// Sucesso passa a seleção pra custom; falha guarda o log em `shader_error`.
    fn import_shader(&mut self, gl: Option<&Arc<eframe::glow::Context>>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Fragment shader", &["frag", "fsh", "glsl"])
            .pick_file()
        else {
            return;
        };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.shader_error = Some(format!("não consegui ler o arquivo: {e}"));
                return;
            }
        };
        let Some((renderer, gl)) = self.shader.as_ref().zip(gl) else {
            return;
        };
        match renderer.lock().unwrap().load_custom(gl, &src) {
            Ok(()) => {
                self.custom_label = file_label(&path);
                self.custom_path = Some(path);
                self.use_custom = true;
                self.shader_error = None;
            }
            Err(e) => self.shader_error = Some(e),
        }
    }

    /// Seleção de shader ativa (embutido ou importado) para pintura/persistência.
    fn shader_active(&self) -> Active {
        if let Some(mp) = self.shader_multipass {
            Active::Multipass(mp)
        } else if self.use_custom {
            Active::Custom
        } else {
            Active::Builtin(self.shader_kind)
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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Contexto GL do backend glow (pra subir o framebuffer e desenhar com
        // shader). `None` em backends sem glow — aí caímos na textura egui.
        let gl = frame.gl().cloned();
        // Bombeia o gamepad (mantém `is_pressed` atualizado) e, se estamos
        // remapeando um botão, captura a próxima tecla/botão pressionado. Enquanto
        // `configuring`, não repassamos input ao jogo nem disparamos hotkeys (pra
        // a tecla capturada não "vazar" pro emulador).
        let pad_event = self.pump_gamepad();
        let configuring = self.rebinding.is_some();

        // Recebe as capas que o worker da biblioteca terminou; se ainda há
        // pendentes e a janela está aberta, segue repintando pra elas aparecerem.
        let covers_pending = self.library.poll(ctx);
        if covers_pending && self.view == View::Library {
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
            if self.space_toggle
                && self.rom_path.is_some()
                && !self.hunting
                && ctx.input(|i| i.key_pressed(egui::Key::Space))
            {
                self.ff_toggle = !self.ff_toggle;
                self.pacer.reset();
            }
        }

        if self.hunting {
            // Modo caça: o Hunter dirige a emulação (amassa A/Start, reseta entre
            // tentativas). Roda um lote de frames por update pra não travar a UI.
            self.hunt_step();
            self.pacer.reset();
            if self.shader.is_none() {
                self.refresh_texture();
            }
            ctx.request_repaint();
        } else if self.running
            && self.view == View::Game
            && !self.settings_open
            && !self.states_open
        {
            if configuring {
                // Remapeando controles: roda no ritmo normal, sem repassar input.
                self.run_paced();
            } else {
                self.poll_input(ctx);
                if rewinding {
                    // Volta no tempo consumindo o anel de snapshots.
                    self.rewind_step();
                    self.gba.bus.apu.buffer.clear();
                    self.pacer.reset();
                } else if (fast_forward && !self.space_toggle) || self.ff_toggle {
                    self.run_fast();
                } else {
                    let ran = self.run_paced();
                    if let Some(trace) = &mut self.pacing_trace {
                        let audio_ms = self.audio.as_ref().map(|a| a.queued_ms());
                        trace.record(Instant::now(), ran, &self.pacer, audio_ms);
                    }
                }
            }
            // Alimenta o detector de cursor (debug) com o estado da RAM deste
            // frame; no-op se não estiver rastreando. (O localizador de batalha usa
            // snapshots por botão, não acumulação por frame.)
            if SHOW_RAM_FINDER {
                self.cursor_finder.observe(&self.gba.bus.iwram[..]);
            }

            if self.shader.is_none() {
                self.refresh_texture();
            }
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

        self.toolbar(ctx);
        self.status_bar(ctx);

        // Janelas (quando abertas).
        self.settings_window(ctx, gl.as_ref());
        self.states_window(ctx);
        if self.rebinding.is_some() {
            // Repinta sem parar pra capturar a tecla/botão do remapeamento.
            ctx.request_repaint();
        }
        if self.show_hunt_help {
            self.hunt_help_window(ctx);
        }

        // Repinta de tempos em tempos pra lista de parceiros na LAN refletir os
        // anúncios que vão chegando.
        if self.show_link {
            self.link_window(ctx);
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        // Painel do Shiny Hunter: abre pelo botão da barra e fica visível durante
        // a caça (é por ele que ela para).
        if self.profile.is_some() && self.view == View::Game && (self.show_hunter || self.hunting) {
            egui::SidePanel::right("shiny_panel")
                .min_width(230.0)
                .show(ctx, |ui| self.shiny_panel(ui));
        }

        match self.view {
            View::Library => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::none()
                            .fill(theme::BG)
                            .inner_margin(egui::Margin::symmetric(40.0, 28.0)),
                    )
                    .show(ctx, |ui| self.library_page(ui));
            }
            View::Game => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(theme::GAME_BG))
                    .show(ctx, |ui| self.game_view(ui, gl.as_ref()));
            }
        }
    }

    /// Persiste a configuração de controles no storage do eframe (chamado
    /// periodicamente e ao fechar).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("input_config", self.input.serialize());
        let scale = if self.scale < 1.0 {
            "fit".to_string()
        } else {
            format!("{:.0}", self.scale)
        };
        storage.set_string("video_scale", scale);
        let flag = |b: bool| if b { "1" } else { "0" }.to_string();
        storage.set_string("sound_on", flag(self.sound_on));
        storage.set_string("volume", self.volume.to_string());
        storage.set_string("ff_sound", flag(self.ff_sound));
        storage.set_string("ff_mult", self.ff_mult.to_string());
        storage.set_string("space_toggle", flag(self.space_toggle));
        if let Some(mp) = self.shader_multipass {
            storage.set_string("video_shader", mp.key().to_string());
        } else if self.use_custom {
            storage.set_string("video_shader", "custom".to_string());
            if let Some(p) = &self.custom_path {
                storage.set_string("video_shader_path", p.display().to_string());
            }
        } else {
            storage.set_string("video_shader", self.shader_kind.key().to_string());
        }
        storage.set_string("video_upscale", self.upscale.key().to_string());
        if let Some(dir) = &self.library.dir {
            storage.set_string("library_dir", dir.display().to_string());
        }
        storage.set_string("language", self.lang_pref.key().to_string());
        let played: String = self
            .last_played
            .iter()
            .map(|(path, secs)| format!("{secs}\t{}\n", path.display()))
            .collect();
        storage.set_string("last_played", played);
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
