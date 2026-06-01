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
    /// Após cada instrução, avança timers + PPU e propaga IRQs.
    pub fn step(&mut self) -> u32 {
        let cycles = self.cpu.step(&mut self.bus);

        let timer_irqs = self.bus.io.timers.tick(cycles);
        // Borrows disjuntos: ppu, vram e palette são campos distintos do bus.
        let ppu_result = {
            let bus = &mut self.bus;
            bus.ppu.tick(cycles, &*bus.vram, &*bus.palette)
        };

        // DMA disparado por VBlank/HBlank (a transferência precisa do bus inteiro,
        // então roda fora do borrow da PPU acima).
        if ppu_result.entered_vblank {
            self.bus.run_dma_timing(crate::dma::Timing::VBlank);
        }
        if ppu_result.entered_hblank {
            self.bus.run_dma_timing(crate::dma::Timing::HBlank);
        }

        let all = timer_irqs | ppu_result.irqs;
        if all != 0 {
            self.bus.io.raise(all);
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
