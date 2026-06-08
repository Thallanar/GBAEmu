//! Biblioteca de ROMs: varre uma pasta, lê o header de cada `.gba` e gera uma
//! capa por jogo. As capas vêm da **box art do libretro** (pros jogos conhecidos,
//! casados pelo game code) com **fallback de screenshot** — bootando a ROM por
//! alguns frames e capturando o framebuffer. Tudo é gerado num worker em
//! background (rede + emulação não podem travar a UI) e cacheado em disco.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use auroragba_core::{Gba, SCREEN_HEIGHT, SCREEN_WIDTH};

/// Quantos frames bootar a ROM antes de capturar a capa por screenshot (~4 s a
/// 60 fps) — o suficiente pra passar de logos/BIOS e chegar a algo visual.
const COVER_FRAMES: u32 = 240;

/// Game code (header, 0xAC) → nome No-Intro da box art no libretro. Best-effort:
/// se o nome não casar (404), cai pro screenshot. Cada jogo conhecido é uma linha
/// (mesmo espírito data-driven do Shiny Hunter).
const BOXART: &[(&str, &str)] = &[
    ("BPEE", "Pokemon - Emerald Version (USA, Europe)"),
    ("AXVE", "Pokemon - Ruby Version (USA, Europe)"),
    ("AXPE", "Pokemon - Sapphire Version (USA, Europe)"),
    ("BPRE", "Pokemon - FireRed Version (USA, Europe)"),
    ("BPGE", "Pokemon - LeafGreen Version (USA, Europe)"),
];

/// Uma ROM na biblioteca.
pub struct RomEntry {
    pub path: PathBuf,
    pub title: String,
    pub code: String,
    /// Textura da capa (None enquanto o worker ainda não devolveu).
    pub cover: Option<egui::TextureHandle>,
}

/// Pedido de capa pro worker.
struct CoverJob {
    path: PathBuf,
    code: String,
}

/// Capa pronta (RGBA já decodificado — sem tipos do egui, pra ser `Send`).
struct CoverResult {
    path: PathBuf,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

pub struct Library {
    /// Pasta atual (persistida).
    pub dir: Option<PathBuf>,
    pub entries: Vec<RomEntry>,
    job_tx: Sender<CoverJob>,
    result_rx: Receiver<CoverResult>,
    _worker: JoinHandle<()>,
}

impl Library {
    /// Cria a biblioteca e sobe o worker de capas (cacheadas em `cache_dir`).
    pub fn new(cache_dir: PathBuf) -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<CoverJob>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<CoverResult>();
        let worker = std::thread::spawn(move || worker_loop(job_rx, result_tx, cache_dir));
        Self {
            dir: None,
            entries: Vec::new(),
            job_tx,
            result_rx,
            _worker: worker,
        }
    }

    /// Varre `dir` (recursivo, raso) por `.gba`, monta as entradas e enfileira a
    /// geração das capas.
    pub fn scan(&mut self, dir: PathBuf) {
        self.entries.clear();
        let mut roms = Vec::new();
        collect_gba(&dir, &mut roms, 0);
        roms.sort();
        for path in roms {
            let (title, code) = read_header(&path);
            let _ = self.job_tx.send(CoverJob {
                path: path.clone(),
                code: code.clone(),
            });
            self.entries.push(RomEntry {
                path,
                title,
                code,
                cover: None,
            });
        }
        self.dir = Some(dir);
    }

    /// Recebe as capas prontas do worker e cria as texturas (na thread da UI).
    /// Devolve `true` se ainda há capas pendentes (pra pedir repaint).
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        while let Ok(res) = self.result_rx.try_recv() {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.path == res.path) {
                let img =
                    egui::ColorImage::from_rgba_unmultiplied([res.width, res.height], &res.rgba);
                entry.cover = Some(ctx.load_texture(
                    format!("cover-{}", entry.path.display()),
                    img,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        self.entries.iter().any(|e| e.cover.is_none())
    }
}

/// Coleta `.gba` recursivamente (até 4 níveis, pra não explorar a árvore inteira).
fn collect_gba(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_gba(&path, out, depth + 1);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gba"))
        {
            out.push(path);
        }
    }
}

/// Lê título (0xA0, 12 bytes) e game code (0xAC, 4 bytes) do header, sem carregar
/// a ROM inteira. Se o título vier vazio, usa o nome do arquivo.
fn read_header(path: &Path) -> (String, String) {
    let mut buf = [0u8; 0xB0];
    if let Ok(mut f) = File::open(path) {
        if f.read_exact(&mut buf).is_ok() {
            let title = ascii_field(&buf[0xA0..0xAC]);
            let code = ascii_field(&buf[0xAC..0xB0]);
            let title = if title.is_empty() {
                file_stem(path)
            } else {
                title
            };
            return (title, code);
        }
    }
    (file_stem(path), String::new())
}

/// Texto ASCII imprimível de um campo do header (descarta zeros/lixo no fim).
fn ascii_field(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ROM")
        .to_string()
}

// ── Worker (background) ────────────────────────────────────────────────────

fn worker_loop(rx: Receiver<CoverJob>, tx: Sender<CoverResult>, cache_dir: PathBuf) {
    for job in rx {
        if let Some((width, height, rgba)) = produce_cover(&job, &cache_dir) {
            // Se o receptor sumiu (app fechou), encerra.
            if tx
                .send(CoverResult {
                    path: job.path,
                    width,
                    height,
                    rgba,
                })
                .is_err()
            {
                break;
            }
        }
    }
}

/// Produz a capa (RGBA): cache em disco → box art do libretro → screenshot.
fn produce_cover(job: &CoverJob, cache_dir: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let cache = cache_dir.join(format!("{}.png", cache_key(job)));

    if let Some(img) = decode_png_file(&cache) {
        return Some(img);
    }

    // Box art do libretro (só pros jogos conhecidos).
    if let Some(name) = boxart_name(&job.code) {
        if let Some(bytes) = fetch_boxart(name) {
            if let Some(img) = decode_png_bytes(&bytes) {
                let _ = fs::create_dir_all(cache_dir);
                let _ = fs::write(&cache, &bytes);
                return Some(img);
            }
        }
    }

    // Fallback: boota a ROM e captura o framebuffer.
    let img = screenshot_cover(&job.path)?;
    let _ = fs::create_dir_all(cache_dir);
    let _ = fs::write(&cache, crate::png::encode_rgba(img.0, img.1, &img.2));
    Some(img)
}

/// Chave de cache: game code quando houver (jogos iguais compartilham), senão o
/// nome do arquivo saneado.
fn cache_key(job: &CoverJob) -> String {
    let raw = if job.code.is_empty() {
        file_stem(&job.path)
    } else {
        job.code.clone()
    };
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn boxart_name(code: &str) -> Option<&'static str> {
    BOXART
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
}

/// Baixa a box art do libretro. None em qualquer falha (rede, 404, etc.).
fn fetch_boxart(name: &str) -> Option<Vec<u8>> {
    let url = format!(
        "https://thumbnails.libretro.com/Nintendo%20-%20Game%20Boy%20Advance/Named_Boxarts/{}.png",
        percent_encode(name)
    );
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .build();
    let resp = agent.get(&url).call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    let mut buf = Vec::new();
    resp.into_reader()
        .take(8 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// Percent-encoding pro path da URL (mantém só os caracteres não-reservados).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodifica PNG/JPG de bytes em RGBA8 (largura, altura, pixels).
fn decode_png_bytes(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w as usize, h as usize, img.into_raw()))
}

fn decode_png_file(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    decode_png_bytes(&fs::read(path).ok()?)
}

/// Capa por screenshot: boota a ROM por [`COVER_FRAMES`] e devolve o framebuffer.
fn screenshot_cover(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let rom = fs::read(path).ok()?;
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    for _ in 0..COVER_FRAMES {
        gba.run_frame();
        // Não acumula áudio (sem consumidor neste contexto).
        gba.bus.apu.buffer.clear();
    }
    Some((
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        gba.bus.ppu.framebuffer[..].to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_spaces_and_parens() {
        assert_eq!(
            percent_encode("Ruby (USA, Europe)"),
            "Ruby%20%28USA%2C%20Europe%29"
        );
    }

    #[test]
    fn boxart_lookup() {
        assert_eq!(
            boxart_name("BPEE"),
            Some("Pokemon - Emerald Version (USA, Europe)")
        );
        assert_eq!(boxart_name("ZZZZ"), None);
    }

    #[test]
    fn ascii_field_trims_garbage() {
        let mut f = *b"POKEMON EMER";
        assert_eq!(ascii_field(&f), "POKEMON EMER");
        f[5] = 0; // corta no zero
        assert_eq!(ascii_field(&f), "POKEM");
    }

    /// Teste de rede (ignorado por padrão): baixa e decodifica uma box art real
    /// do libretro. Rode com `cargo test -p auroragba-desktop -- --ignored`.
    #[test]
    #[ignore]
    fn fetch_and_decode_real_boxart() {
        let name = boxart_name("BPEE").unwrap();
        let bytes = fetch_boxart(name).expect("download da box art");
        let (w, h, rgba) = decode_png_bytes(&bytes).expect("decodificar a box art");
        assert!(w > 0 && h > 0 && rgba.len() == w * h * 4);
    }

    #[test]
    fn screenshot_cache_png_roundtrips_through_image() {
        // O cache de screenshot é gravado com o nosso encoder (DEFLATE "stored")
        // e relido pelo crate `image` — este teste garante que o `image` decodifica
        // o que produzimos, com as cores certas.
        let (w, h) = (4usize, 3usize);
        let mut rgba = vec![0u8; w * h * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i * 20) as u8, 255 - (i * 10) as u8, (i * 5) as u8, 255]);
        }
        let png = crate::png::encode_rgba(w, h, &rgba);
        let (dw, dh, decoded) = decode_png_bytes(&png).expect("image deve decodificar");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(decoded, rgba);
    }
}
