//! Memory bus do GBA.
//!
//! Roteia reads/writes para a região correta usando os 4 bits superiores
//! do endereço (nibble de região), conforme GBATEK:
//!
//! | Nibble | Região              | Tamanho |
//! |--------|---------------------|---------|
//! | 0x0    | BIOS                | 16 KB   |
//! | 0x2    | EWRAM (on-board)    | 256 KB  |
//! | 0x3    | IWRAM (on-chip)     | 32 KB   |
//! | 0x4    | I/O Registers       | 1 KB    |
//! | 0x5    | Palette RAM         | 1 KB    |
//! | 0x6    | VRAM                | 96 KB   |
//! | 0x7    | OAM                 | 1 KB    |
//! | 0x8..D | Game Pak ROM (mirror) | até 32 MB |
//! | 0xE    | Game Pak SRAM       | 64 KB   |

use crate::cartridge::Cartridge;
use crate::dma::{self, Dma, Timing, DMA_BASE, DMA_END};
use crate::io::Io;
use crate::ppu::Ppu;

pub struct Bus {
    pub bios: Vec<u8>,
    /// Quando true, chamadas SWI são tratadas por HLE (BIOS embutida).
    /// Vira false se uma BIOS oficial for carregada no futuro.
    pub hle_bios: bool,
    pub ewram: Box<[u8; 0x40000]>, // 256 KB
    pub iwram: Box<[u8; 0x8000]>,  // 32 KB
    pub io: Io,
    pub dma: Dma,
    pub ppu: Ppu,
    pub palette: Box<[u8; 0x400]>, // 1 KB
    pub vram: Box<[u8; 0x18000]>,  // 96 KB
    pub oam: Box<[u8; 0x400]>,     // 1 KB
    pub cartridge: Cartridge,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            bios: crate::cpu::bios::builtin_bios(),
            hle_bios: true,
            ewram: Box::new([0; 0x40000]),
            iwram: Box::new([0; 0x8000]),
            io: Io::new(),
            dma: Dma::new(),
            ppu: Ppu::new(),
            palette: Box::new([0; 0x400]),
            vram: Box::new([0; 0x18000]),
            oam: Box::new([0; 0x400]),
            cartridge: Cartridge::default(),
        }
    }

    // ───────────────────── reads ─────────────────────

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        let region = (addr >> 24) & 0xF;
        match region {
            0x0 => self.bios.get(addr as usize).copied().unwrap_or(0),
            0x2 => self.ewram[(addr as usize) & 0x3FFFF],
            0x3 => self.iwram[(addr as usize) & 0x7FFF],
            0x4 => {
                // PPU regs: 0x04000000..0x04000056; DMA: 0xB0..0xDF; resto via Io.
                if (DMA_BASE..DMA_END).contains(&addr) {
                    self.dma.read_u8(addr)
                } else if addr < 0x0400_0060 {
                    self.ppu.read_u8(addr)
                } else {
                    self.io.read_u8(addr)
                }
            }
            0x5 => self.palette[(addr as usize) & 0x3FF],
            0x6 => self.vram[vram_offset(addr)],
            0x7 => self.oam[(addr as usize) & 0x3FF],
            0x8..=0xD => {
                let off = (addr as usize) & 0x01FF_FFFF;
                self.cartridge.rom.get(off).copied().unwrap_or(0)
            }
            0xE | 0xF => self
                .cartridge
                .save_data
                .get((addr as usize) & 0xFFFF)
                .copied()
                .unwrap_or(0),
            _ => 0, // open bus (placeholder)
        }
    }

    pub fn read_u16(&mut self, addr: u32) -> u16 {
        let a = addr & !1; // alinhamento half-word
        let lo = self.read_u8(a) as u16;
        let hi = self.read_u8(a + 1) as u16;
        lo | (hi << 8)
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        let a = addr & !3; // alinhamento word
        let b0 = self.read_u8(a) as u32;
        let b1 = self.read_u8(a + 1) as u32;
        let b2 = self.read_u8(a + 2) as u32;
        let b3 = self.read_u8(a + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    // ───────────────────── writes ─────────────────────

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let region = (addr >> 24) & 0xF;
        match region {
            0x0 => { /* BIOS é read-only */ }
            0x2 => self.ewram[(addr as usize) & 0x3FFFF] = val,
            0x3 => self.iwram[(addr as usize) & 0x7FFF] = val,
            0x4 => {
                if (DMA_BASE..DMA_END).contains(&addr) {
                    // Habilitar um canal com timing imediato dispara a cópia já.
                    if let Some(n) = self.dma.write_u8(addr, val) {
                        if self.dma.channels[n].timing() == Timing::Immediate {
                            self.run_dma_channel(n);
                        }
                    }
                } else if addr < 0x0400_0060 {
                    self.ppu.write_u8(addr, val);
                } else {
                    self.io.write_u8(addr, val);
                }
            }
            0x5 => self.palette[(addr as usize) & 0x3FF] = val,
            0x6 => self.vram[vram_offset(addr)] = val,
            0x7 => self.oam[(addr as usize) & 0x3FF] = val,
            0x8..=0xD => { /* ROM read-only (writes podem disparar EEPROM, ver Fase 4) */ }
            0xE | 0xF => {
                let idx = (addr as usize) & 0xFFFF;
                if idx < self.cartridge.save_data.len() {
                    self.cartridge.save_data[idx] = val;
                }
            }
            _ => {}
        }
    }

    pub fn write_u16(&mut self, addr: u32, val: u16) {
        let a = addr & !1;
        self.write_u8(a, val as u8);
        self.write_u8(a + 1, (val >> 8) as u8);
    }

    pub fn write_u32(&mut self, addr: u32, val: u32) {
        let a = addr & !3;
        self.write_u8(a, val as u8);
        self.write_u8(a + 1, (val >> 8) as u8);
        self.write_u8(a + 2, (val >> 16) as u8);
        self.write_u8(a + 3, (val >> 24) as u8);
    }

    // ───────────────────────── DMA ─────────────────────────

    /// Roda todos os canais habilitados cujo timing casa com o evento dado
    /// (chamado pelo `Gba::step` no início do VBlank/HBlank).
    pub fn run_dma_timing(&mut self, timing: Timing) {
        for n in 0..4 {
            if self.dma.channels[n].enabled() && self.dma.channels[n].timing() == timing {
                self.run_dma_channel(n);
            }
        }
    }

    /// Executa uma transferência completa do canal `n`. Atualiza os ponteiros
    /// internos, trata repeat/reload, limpa o enable quando termina e levanta
    /// a IRQ de fim de DMA se habilitada.
    fn run_dma_channel(&mut self, n: usize) {
        // Cópia local (DmaChannel é Copy): evita aliasing com self.read/write.
        let mut ch = self.dma.channels[n];
        if !ch.enabled() {
            return;
        }

        let unit = ch.unit_bytes();
        let src_step: i64 = match ch.src_control() {
            1 => -(unit as i64),
            2 => 0,
            _ => unit as i64, // 0=inc, 3=proibido (tratado como inc)
        };
        let dst_step: i64 = match ch.dst_control() {
            1 => -(unit as i64),
            2 => 0,
            _ => unit as i64, // 0=inc, 3=inc+reload
        };

        let mut src = ch.int_src;
        let mut dst = ch.int_dst;
        for _ in 0..ch.int_count {
            if unit == 4 {
                let v = self.read_u32(src);
                self.write_u32(dst, v);
            } else {
                let v = self.read_u16(src);
                self.write_u16(dst, v);
            }
            src = (src as i64 + src_step) as u32;
            dst = (dst as i64 + dst_step) as u32;
        }
        ch.int_src = src;
        ch.int_dst = dst;

        // Repeat (exceto imediato) recarrega; senão desabilita o canal.
        if ch.repeat() && ch.timing() != Timing::Immediate {
            dma::reload(&mut ch, n);
        } else {
            ch.control &= !(1 << 15);
        }

        let raise_irq = ch.irq_on_end();
        self.dma.channels[n] = ch;
        if raise_irq {
            self.io.raise(1 << (8 + n)); // DMA0..3 = bits 8..11
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

/// VRAM tem 96 KB mas é espelhada em janelas de 128 KB com folding 64K+32K+32K.
fn vram_offset(addr: u32) -> usize {
    let a = (addr as usize) & 0x1FFFF;
    if a < 0x10000 { a } else { 0x10000 + (a & 0x7FFF) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Endereços dos registradores do canal 0 de DMA.
    const DMA0_SAD: u32 = 0x0400_00B0;
    const DMA0_DAD: u32 = 0x0400_00B4;
    const DMA0_CNT_L: u32 = 0x0400_00B8;
    const DMA0_CNT_H: u32 = 0x0400_00BA;

    const SRC: u32 = 0x0200_0000;
    const DST: u32 = 0x0200_1000;

    /// Configura e dispara um DMA imediato do canal 0 com o controle dado.
    fn setup_immediate_dma(bus: &mut Bus, count: u16, control: u16) {
        bus.write_u32(DMA0_SAD, SRC);
        bus.write_u32(DMA0_DAD, DST);
        bus.write_u16(DMA0_CNT_L, count);
        // Escrever o controle (com enable) dispara a transferência imediata.
        bus.write_u16(DMA0_CNT_H, control);
    }

    #[test]
    fn dma_immediate_word_copy() {
        let mut bus = Bus::new();
        for i in 0..4u32 {
            bus.write_u32(SRC + i * 4, 0x1100_0000 + i);
        }
        // enable | 32-bit | timing imediato | src inc | dst inc.
        setup_immediate_dma(&mut bus, 4, (1 << 15) | (1 << 10));
        for i in 0..4u32 {
            assert_eq!(bus.read_u32(DST + i * 4), 0x1100_0000 + i);
        }
        // Sem repeat: o enable deve estar limpo após terminar.
        assert!(!bus.dma.channels[0].enabled());
    }

    #[test]
    fn dma_immediate_halfword_copy() {
        let mut bus = Bus::new();
        for i in 0..6u32 {
            bus.write_u16(SRC + i * 2, 0xA000 + i as u16);
        }
        // enable | 16-bit (bit10=0).
        setup_immediate_dma(&mut bus, 6, 1 << 15);
        for i in 0..6u32 {
            assert_eq!(bus.read_u16(DST + i * 2), 0xA000 + i as u16);
        }
    }

    #[test]
    fn dma_fixed_source_fill() {
        let mut bus = Bus::new();
        bus.write_u32(SRC, 0xCAFE_F00D);
        // enable | 32-bit | src fixo (bits 7-8 = 2 → 0b10 << 7 = 0x100).
        setup_immediate_dma(&mut bus, 3, (1 << 15) | (1 << 10) | (2 << 7));
        for i in 0..3u32 {
            assert_eq!(bus.read_u32(DST + i * 4), 0xCAFE_F00D);
        }
    }

    #[test]
    fn dma_irq_on_end_raises_flag() {
        let mut bus = Bus::new();
        bus.write_u32(SRC, 0);
        // enable | 32-bit | IRQ on end (bit14).
        setup_immediate_dma(&mut bus, 1, (1 << 15) | (1 << 10) | (1 << 14));
        // DMA0 → bit 8 do IF.
        assert!(bus.io.iflag & (1 << 8) != 0);
    }
}
