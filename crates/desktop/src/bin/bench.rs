//! Régua de performance do core, headless e sem I/O por frame — o alvo limpo do
//! flamegraph da Fase 9 e a medida repetível do ganho de cada otimização.
//!
//! Carrega a ROM, faz direct boot e roda N frames amassando A/START (mesmo
//! roteiro do `rng_scan`, pra exercitar a lógica real do jogo: intro, RNG,
//! menus). Não lê nem imprime nada por frame, então o tempo medido é quase só
//! o `Gba::step()` em loop.
//!
//! Uso:
//!   cargo run --release -p auroragba-desktop --bin bench -- <rom.gba> [frames]
//!
//! Imprime frames, tempo de parede, fps do core e o múltiplo do tempo real
//! (GBA = 59,7275 fps). A baseline da Fase 9 é ~108 fps (×1,81) no Emerald.

use std::time::Instant;

use auroragba_core::joypad::Button;
use auroragba_core::Gba;

/// Taxa de quadros real do GBA (≈ 16,78 MHz / 280896 ciclos por frame).
const GBA_FPS: f64 = 59.7275;

/// Mesma cadência de input do `rng_scan`: tapa A (8 on / 8 off) e dá START de
/// vez em quando pra destravar telas de título/nome.
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
    let path = args.next().expect("uso: bench <rom.gba> [frames]");
    let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(600);

    let rom = std::fs::read(&path)?;
    let code = String::from_utf8_lossy(&rom[0xAC..0xB0]).into_owned();
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset(); // direct boot

    // Aquece alguns frames fora da medição (a intro tem picos atípicos de carga).
    for frame in 0..60 {
        mash(&mut gba, frame);
        gba.run_frame();
    }

    let start = Instant::now();
    for frame in 60..(60 + frames) {
        mash(&mut gba, frame);
        gba.run_frame();
    }
    let elapsed = start.elapsed();

    let secs = elapsed.as_secs_f64();
    let fps = frames as f64 / secs;
    println!("ROM {code}: {frames} frames em {secs:.3}s");
    println!("core: {fps:.1} fps  (×{:.2} do tempo real)", fps / GBA_FPS);
    Ok(())
}
