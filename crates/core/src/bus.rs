//! Memory bus do GBA — roteia reads/writes para a região correta.
//!
//! Mapa de memória (GBATEK):
//!   0x00000000 - 0x00003FFF   BIOS         (16 KB)
//!   0x02000000 - 0x0203FFFF   EWRAM        (256 KB)
//!   0x03000000 - 0x03007FFF   IWRAM        (32 KB)
//!   0x04000000 - 0x040003FE   I/O Registers
//!   0x05000000 - 0x050003FF   Palette RAM  (1 KB)
//!   0x06000000 - 0x06017FFF   VRAM         (96 KB)
//!   0x07000000 - 0x070003FF   OAM          (1 KB)
//!   0x08000000 - 0x09FFFFFF   ROM cartridge

use crate::cartridge::Cartridge;

pub struct Bus {
    pub bios: Vec<u8>,
    pub ewram: Box<[u8; 0x40000]>,
    pub iwram: Box<[u8; 0x8000]>,
    pub cartridge: Cartridge,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            bios: Vec::new(),
            ewram: Box::new([0; 0x40000]),
            iwram: Box::new([0; 0x8000]),
            cartridge: Cartridge::default(),
        }
    }

    pub fn read_u8(&self, _addr: u32) -> u8 {
        // TODO
        0
    }

    pub fn write_u8(&mut self, _addr: u32, _val: u8) {
        // TODO
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
