//! Estrutura de topo que junta CPU, bus, PPU e APU.

use crate::bus::Bus;
use crate::cpu::Cpu;

/// Instância completa do emulador.
pub struct Gba {
    pub cpu: Cpu,
    pub bus: Bus,
}

impl Gba {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(),
        }
    }

    /// Carrega uma ROM na cartridge.
    pub fn load_rom(&mut self, rom: Vec<u8>) {
        self.bus.cartridge.load(rom);
    }

    /// Executa um frame inteiro (placeholder).
    pub fn run_frame(&mut self) {
        // TODO: loop até VBlank (~280896 ciclos por frame)
    }
}

impl Default for Gba {
    fn default() -> Self {
        Self::new()
    }
}
