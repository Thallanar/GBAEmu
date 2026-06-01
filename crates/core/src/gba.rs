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

    /// Executa uma única instrução. Retorna ciclos consumidos.
    /// Após cada instrução, avança timers e propaga IRQs pendentes para o IF.
    pub fn step(&mut self) -> u32 {
        let cycles = self.cpu.step(&mut self.bus);

        // Avança timers e propaga IRQs solicitadas.
        let timer_irqs = self.bus.io.timers.tick(cycles);
        if timer_irqs != 0 {
            self.bus.io.raise(timer_irqs);
        }
        cycles
    }

    /// Executa um frame inteiro (~280896 ciclos). Placeholder.
    pub fn run_frame(&mut self) {
        let mut cycles = 0u32;
        while cycles < 280_896 {
            cycles += self.step();
        }
    }
}

impl Default for Gba {
    fn default() -> Self {
        Self::new()
    }
}
