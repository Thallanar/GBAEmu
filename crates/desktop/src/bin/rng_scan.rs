//! Acha o `gRngValue` de um jogo Gen 3 **empiricamente**, rodando a ROM real:
//! escaneia a IWRAM procurando a palavra que evolui pela assinatura do LCG da
//! Gen 3 (`seed = seed * 0x41C64E6D + 0x6073`, k passos por frame).
//!
//! É a ferramenta pra mapear o `rng_addr` de um [`GameProfile`] novo (Ruby/
//! Sapphire/FireRed/...) sem depender só dos símbolos do decomp — e pra
//! confirmar os símbolos contra a revisão da ROM que o usuário tem.
//!
//! Uso:
//!   cargo run --release -p auroragba-desktop --bin rng_scan -- <rom.gba> [frames]
//!   cargo run --release -p auroragba-desktop --bin rng_scan -- <rom.gba> --sprites <dir>
//!   cargo run --release -p auroragba-desktop --bin rng_scan -- <rom.gba> --watch <addr-hex> [frames]
//!
//! `--sprites` decodifica o sprite (normal+shiny) de cada alvo do perfil
//! detectado pelo game code e grava PPMs — oráculo visual dos índices internos
//! de espécie (sprite errado = índice errado).
//!
//! `--watch` imprime o u32 no endereço a cada 30 frames (com A-mash) — ex.:
//! observar o `gMain.callback2` do jogo e mapear, pelos símbolos do decomp, em
//! que tela a lógica está presa.

use std::path::PathBuf;

use auroragba_core::joypad::Button;
use auroragba_core::Gba;
use auroragba_shiny::games;
use auroragba_shiny::gfx::RomGfx;

const IWRAM_BASE: u32 = 0x0300_0000;
const IWRAM_WORDS: usize = 32 * 1024 / 4;
/// Máximo de chamadas a `Random()` num frame pra ainda casar a assinatura.
const MAX_LCG_STEPS: u32 = 128;
/// Um candidato precisa ter avançado pelo LCG em pelo menos N frames distintos
/// (mata palavras paradas, que casam trivialmente com k=0). A chance de um
/// avanço espúrio casar o LCG é ~128/2³² por mudança, então N baixo já zera
/// falso-positivo junto com a regra acertos > desvios.
const MIN_CHANGES: u32 = 8;
/// Depois de tantos desvios sem acerto nenhum, desiste da palavra (poda).
const PRUNE_MISSES: u32 = 32;

/// Um passo do LCG da Gen 3 (o `Random()` de R/S/E/FRLG).
fn lcg(seed: u32) -> u32 {
    seed.wrapping_mul(0x41C6_4E6D).wrapping_add(0x6073)
}

fn snapshot(gba: &mut Gba, buf: &mut [u32; IWRAM_WORDS]) {
    for (i, w) in buf.iter_mut().enumerate() {
        *w = gba.bus.read_u32(IWRAM_BASE + (i as u32) * 4);
    }
}

/// Roteiro de input do scan: tapa A (8 on / 8 off) e, de vez em quando, START.
/// O START destrava telas que A sozinho não passa (título de R/S, e a tela de
/// nome do new game, onde START pula pro "OK") — sem ele o jogo nunca chega
/// onde `Random()` é chamado.
fn mash(gba: &mut Gba, frame: u32) {
    gba.bus
        .io
        .joypad
        .set_button(Button::A, (frame / 8).is_multiple_of(2));
    gba.bus
        .io
        .joypad
        .set_button(Button::Start, (frame / 8) % 12 == 11);
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path: PathBuf = args
        .next()
        .expect("uso: rng_scan <rom.gba> [frames | --sprites <dir>]")
        .into();
    let rom = std::fs::read(&path)?;
    let code = String::from_utf8_lossy(&rom[0xAC..0xB0]).into_owned();
    println!("ROM: {} ({} bytes, code {code})", path.display(), rom.len());

    match args.next().as_deref() {
        Some("--sprites") => {
            let dir = args.next().expect("--sprites precisa do diretório");
            return dump_sprites(&rom, &code, &dir);
        }
        Some("--watch") => {
            let addr = u32::from_str_radix(
                args.next().expect("--watch precisa do endereço").trim_start_matches("0x"),
                16,
            )
            .expect("endereço hex inválido");
            let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1200);
            watch(rom, addr, frames);
        }
        rest => {
            let frames: u32 = rest.and_then(|s| s.parse().ok()).unwrap_or(1200);
            scan_rng(rom, frames);
        }
    }
    Ok(())
}

/// Roda a ROM amassando A e imprime o u32 em `addr` a cada 30 frames (e sempre
/// que mudar). Pra observar ponteiros de estado do jogo (ex.: gMain.callback2).
fn watch(rom: Vec<u8>, addr: u32, frames: u32) {
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset();
    let mut last = 0u32;
    for frame in 0..frames {
        mash(&mut gba, frame);
        gba.run_frame();
        let v = gba.bus.read_u32(addr);
        if v != last || frame % 30 == 0 {
            println!("frame {frame:5}: [{addr:08X}] = {v:08X}");
            last = v;
        }
    }
}

/// Roda a ROM amassando A e, frame a frame, conta pra cada palavra da IWRAM
/// quantas mudanças casam a assinatura `lcg^k` (1 ≤ k ≤ MAX_LCG_STEPS) e
/// quantas desviam. O gRngValue acumula acertos de sobra; desvios ocasionais
/// são tolerados porque os jogos **re-semeiam** o RNG de vez em quando (R/S
/// fazem isso na intro — eliminar no primeiro desvio perdia o endereço certo).
fn scan_rng(rom: Vec<u8>, frames: u32) {
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset(); // direct boot

    let mut prev = Box::new([0u32; IWRAM_WORDS]);
    let mut cur = Box::new([0u32; IWRAM_WORDS]);
    let mut matches = vec![0u32; IWRAM_WORDS];
    let mut misses = vec![0u32; IWRAM_WORDS];

    snapshot(&mut gba, &mut prev);
    println!("Rodando {frames} frames (A-mash) e filtrando pela assinatura do LCG...");
    for frame in 0..frames {
        mash(&mut gba, frame);
        gba.run_frame();
        snapshot(&mut gba, &mut cur);

        for i in 0..IWRAM_WORDS {
            if cur[i] == prev[i] || (misses[i] >= PRUNE_MISSES && matches[i] == 0) {
                continue;
            }
            let mut x = prev[i];
            let mut ok = false;
            for _ in 0..MAX_LCG_STEPS {
                x = lcg(x);
                if x == cur[i] {
                    ok = true;
                    break;
                }
            }
            if ok {
                matches[i] += 1;
            } else {
                misses[i] += 1;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }

    let hits: Vec<(u32, u32, u32)> = (0..IWRAM_WORDS)
        .filter(|&i| matches[i] >= MIN_CHANGES && matches[i] > misses[i])
        .map(|i| (IWRAM_BASE + (i as u32) * 4, matches[i], misses[i]))
        .collect();
    if hits.is_empty() {
        println!(
            "Nenhum candidato com ≥{MIN_CHANGES} avanços de LCG — rode mais frames \
             (o jogo pode não chamar Random() nesta tela)."
        );
    } else {
        println!("Candidatos a gRngValue (endereço, avanços de LCG, re-seeds):");
        for (addr, m, ms) in hits {
            println!("  {addr:08X}  ({m} avanços, {ms} desvios)");
        }
    }
}

/// Decodifica o sprite normal+shiny de cada alvo do perfil do jogo e grava
/// `<dir>/<code>_<species>_<nome>[_shiny].ppm`.
fn dump_sprites(rom: &[u8], code: &str, dir: &str) -> std::io::Result<()> {
    use std::io::Write;
    let profile = games::detect(code).expect("game code sem perfil em games.rs");
    let gfx = RomGfx::locate(rom).expect("tabelas de sprite não encontradas na ROM");
    std::fs::create_dir_all(dir)?;
    for t in profile.targets {
        for shiny in [false, true] {
            let sp = gfx
                .decode_front(rom, t.species, shiny)
                .expect("sprite não decodificou");
            let slug: String = t
                .name
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let suffix = if shiny { "_shiny" } else { "" };
            let path = format!("{dir}/{code}_{}_{slug}{suffix}.ppm", t.species);
            let mut f = std::fs::File::create(&path)?;
            write!(f, "P6\n{} {}\n255\n", sp.width, sp.height)?;
            for px in sp.rgba.chunks_exact(4) {
                // PPM não tem alpha: pixel transparente vira magenta (destaca).
                if px[3] == 0 {
                    f.write_all(&[0xFF, 0x00, 0xFF])?;
                } else {
                    f.write_all(&px[0..3])?;
                }
            }
            println!("{path}");
        }
    }
    Ok(())
}
