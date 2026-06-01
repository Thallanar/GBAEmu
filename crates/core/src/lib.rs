//! AuroraGBA — core de emulação do Game Boy Advance.
//!
//! Esta crate contém toda a lógica de emulação (CPU, PPU, APU, memória)
//! e é puramente computacional — não faz I/O nem desenha na tela.
//! Os frontends (desktop, android) consomem esta crate.

pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod gba;
pub mod ppu;
pub mod apu;

pub use gba::Gba;

/// Resolução nativa do GBA.
pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;

/// Frequência do clock principal do GBA em Hz (~16.78 MHz).
pub const CLOCK_HZ: u32 = 16_777_216;
