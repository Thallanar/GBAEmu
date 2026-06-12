//! Régua de EQUIVALÊNCIA do core: hash FNV-1a do stream de áudio drenado ao
//! longo de N frames. Como o áudio depende de toda a interação CPU/timer/DMA/
//! APU, hash idêntico entre duas versões ⇒ a emulação inteira andou em
//! lockstep — é a prova "bit a bit" usada na Fase 9 (batch de timers) e na
//! Fase 10 (cache de decode), agora como ferramenta permanente.
//!
//! Uso:
//!   cargo run --release -p auroragba-desktop --bin audiohash -- <rom.gba> [frames]
//!
//! Rode no baseline (main) e no branch da otimização: os dois devem imprimir
//! o MESMO hash e o MESMO total de amostras.

use auroragba_core::joypad::Button;
use auroragba_core::Gba;

/// Mesma cadência de input do `bench`/`rng_scan`, pra exercitar lógica real.
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
    let path = args.next().expect("uso: audiohash <rom.gba> [frames]");
    let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(900);

    let rom = std::fs::read(&path)?;
    let code = String::from_utf8_lossy(&rom[0xAC..0xB0]).into_owned();
    let mut gba = Gba::new();
    gba.load_rom(rom);
    gba.reset(); // direct boot

    // FNV-1a 64 bits sobre cada amostra i16 (little-endian), drenada por frame.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut samples: u64 = 0;
    for frame in 0..frames {
        mash(&mut gba, frame);
        gba.run_frame();
        for s in gba.bus.apu.buffer.drain(..) {
            for b in s.to_le_bytes() {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            samples += 1;
        }
    }

    println!("ROM {code}: {frames} frames, {samples} amostras");
    println!("hash fnv1a64: {hash:016x}");
    Ok(())
}
