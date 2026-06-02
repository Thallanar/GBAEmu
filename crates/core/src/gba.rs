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

    /// "Power-cycle": reinicia CPU, bus e periféricos do zero, mas **preserva o
    /// cartucho** (ROM + Flash). Equivale a desligar e ligar o console — o save
    /// na memória de backup sobrevive. É o primitivo usado pelo Shiny Hunter
    /// para o soft-reset entre tentativas.
    pub fn reset(&mut self) {
        // Tira o cartucho fora antes de recriar o bus, depois devolve.
        let cartridge = std::mem::take(&mut self.bus.cartridge);
        self.bus = Bus::new();
        self.bus.cartridge = cartridge;
        self.cpu = Cpu::new();
        self.cpu.setup_direct_boot();
        self.cpu.regs.set_pc(0x0800_0000);
    }

    /// Executa uma única instrução. Retorna ciclos consumidos.
    /// Após cada instrução, avança timers + PPU e propaga IRQs.
    pub fn step(&mut self) -> u32 {
        let cycles = self.cpu.step(&mut self.bus);

        let timer = self.bus.io.timers.tick(cycles);
        let timer_irqs = timer.irqs;
        self.bus.apu.tick(cycles);

        // Direct Sound: cada overflow dos timers 0/1 avança 1 amostra das FIFOs
        // que usam aquele timer. Depois, reabastece as FIFOs via DMA special.
        for (t, &count) in timer.snd_overflows.iter().enumerate() {
            for _ in 0..count {
                self.bus.apu.on_timer_overflow(t as u8);
            }
        }
        self.refill_sound_fifos();
        // Borrows disjuntos: ppu, vram e palette são campos distintos do bus.
        let ppu_result = {
            let bus = &mut self.bus;
            bus.ppu.tick(cycles, &*bus.vram, &*bus.palette, &*bus.oam)
        };

        // DMA disparado por VBlank/HBlank (a transferência precisa do bus inteiro,
        // então roda fora do borrow da PPU acima).
        if ppu_result.entered_vblank {
            self.bus.run_dma_timing(crate::dma::Timing::VBlank);
        }
        if ppu_result.entered_hblank {
            self.bus.run_dma_timing(crate::dma::Timing::HBlank);
        }

        let key_irq = if self.bus.io.joypad.irq_pending() {
            crate::io::irq_bits::KEYPAD
        } else {
            0
        };

        let all = timer_irqs | ppu_result.irqs | key_irq;
        if all != 0 {
            self.bus.io.raise(all);
        }
        cycles
    }

    /// Reabastece as FIFOs do Direct Sound via DMA "special". DMA1/DMA2 em modo
    /// special com destino numa FIFO transferem 4 words (16 amostras) sempre que
    /// a FIFO cai à metade. Origem incrementa, destino é fixo, e o canal repete
    /// (não desabilita).
    fn refill_sound_fifos(&mut self) {
        for ch in 1..=2usize {
            let c = self.bus.dma.channels[ch];
            if !c.enabled() || c.timing() != crate::dma::Timing::Special {
                continue;
            }
            let fifo = match c.int_dst {
                0x0400_00A0 => 0,
                0x0400_00A4 => 1,
                _ => continue,
            };
            if !self.bus.apu.fifo_needs_refill(fifo) {
                continue;
            }
            let mut src = c.int_src;
            let dst = c.int_dst;
            for _ in 0..4 {
                let w = self.bus.read_u32(src);
                self.bus.write_u32(dst, w); // roteado para a FIFO do APU
                src = src.wrapping_add(4);
            }
            self.bus.dma.channels[ch].int_src = src;
        }
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
