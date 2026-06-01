//! PPU — Picture Processing Unit. Implementação na Fase 2.

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

pub struct Ppu {
    /// Framebuffer RGBA8, 240x160.
    pub framebuffer: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4]>,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            framebuffer: Box::new([0; SCREEN_WIDTH * SCREEN_HEIGHT * 4]),
        }
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}
