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
                // PPU regs: 0x04000000..0x04000056. Demais via Io.
                if addr < 0x0400_0060 {
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
                if addr < 0x0400_0060 {
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
