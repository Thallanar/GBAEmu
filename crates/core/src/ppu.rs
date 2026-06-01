//! PPU — Picture Processing Unit.
//!
//! Por ora, apenas a máquina de estados de scanlines: gera HBlank/VBlank/VCount
//! IRQs no tempo certo e mantém DISPCNT/DISPSTAT/VCOUNT acessíveis via I/O.
//! O desenho dos pixels virá numa próxima iteração.
//!
//! Timing (1 dot = 4 ciclos da CPU):
//!   - 240 dots de HDraw + 68 dots de HBlank = 308 dots = 1232 ciclos/scanline
//!   - 160 scanlines visíveis + 68 de VBlank  = 228 scanlines/frame
//!   - Total: 228 × 1232 = 280 896 ciclos/frame (~59.7 Hz)

use crate::io::irq_bits;
use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

pub const CYCLES_PER_SCANLINE: u32 = 1232;
pub const HDRAW_CYCLES: u32 = 960;
pub const TOTAL_SCANLINES: u16 = 228;
pub const VISIBLE_SCANLINES: u16 = 160;

// DISPSTAT bits
const DISPSTAT_VBLANK_FLAG: u16 = 1 << 0;
const DISPSTAT_HBLANK_FLAG: u16 = 1 << 1;
const DISPSTAT_VCOUNT_FLAG: u16 = 1 << 2;
const DISPSTAT_VBLANK_IRQ:  u16 = 1 << 3;
const DISPSTAT_HBLANK_IRQ:  u16 = 1 << 4;
const DISPSTAT_VCOUNT_IRQ:  u16 = 1 << 5;
// bits 8..15: VCount setting (alvo da VCount match interrupt)

pub struct Ppu {
    /// Framebuffer RGBA8, 240×160.
    pub framebuffer: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4]>,

    pub dispcnt: u16,
    pub dispstat: u16,
    pub vcount: u16,

    /// Ciclos acumulados no scanline atual.
    cycles: u32,
    /// `true` quando passamos do HDraw para HBlank no scanline atual.
    in_hblank: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            framebuffer: Box::new([0; SCREEN_WIDTH * SCREEN_HEIGHT * 4]),
            dispcnt: 0,
            dispstat: 0,
            vcount: 0,
            cycles: 0,
            in_hblank: false,
        }
    }

    /// Avança `cycles` ciclos. Retorna bitmap de IRQs a sinalizar.
    /// Recebe slices da VRAM e palette para poder renderizar scanlines.
    pub fn tick(&mut self, cycles: u32, vram: &[u8], palette: &[u8]) -> u16 {
        let mut irqs: u16 = 0;
        self.cycles += cycles;

        // Pode haver mais de uma transição de fase num único tick.
        loop {
            if !self.in_hblank && self.cycles >= HDRAW_CYCLES {
                // Entra em HBlank. Render do scanline atual (se visível).
                if self.vcount < VISIBLE_SCANLINES {
                    self.render_scanline(self.vcount, vram, palette);
                }
                self.in_hblank = true;
                self.dispstat |= DISPSTAT_HBLANK_FLAG;
                if self.dispstat & DISPSTAT_HBLANK_IRQ != 0 {
                    irqs |= irq_bits::HBLANK;
                }
                continue;
            }
            if self.cycles >= CYCLES_PER_SCANLINE {
                self.cycles -= CYCLES_PER_SCANLINE;
                self.in_hblank = false;
                self.dispstat &= !DISPSTAT_HBLANK_FLAG;
                self.vcount = (self.vcount + 1) % TOTAL_SCANLINES;

                // VBlank começa exatamente no scanline 160.
                if self.vcount == VISIBLE_SCANLINES {
                    self.dispstat |= DISPSTAT_VBLANK_FLAG;
                    if self.dispstat & DISPSTAT_VBLANK_IRQ != 0 {
                        irqs |= irq_bits::VBLANK;
                    }
                } else if self.vcount == 0 {
                    // Novo frame.
                    self.dispstat &= !DISPSTAT_VBLANK_FLAG;
                }

                // VCount match.
                let vcount_target = (self.dispstat >> 8) & 0xFF;
                if self.vcount == vcount_target {
                    self.dispstat |= DISPSTAT_VCOUNT_FLAG;
                    if self.dispstat & DISPSTAT_VCOUNT_IRQ != 0 {
                        irqs |= irq_bits::VCOUNT;
                    }
                } else {
                    self.dispstat &= !DISPSTAT_VCOUNT_FLAG;
                }
                continue;
            }
            break;
        }
        irqs
    }

    // ───────────────── Render ─────────────────

    fn render_scanline(&mut self, y: u16, vram: &[u8], palette: &[u8]) {
        let mode = self.dispcnt & 0b111;
        let force_blank = self.dispcnt & 0x80 != 0;

        if force_blank {
            self.fill_scanline_white(y);
            return;
        }

        match mode {
            3 => self.render_mode3(y, vram),
            4 => self.render_mode4(y, vram, palette),
            5 => self.render_mode5(y, vram, palette),
            _ => self.render_backdrop(y, palette),
        }
    }

    /// Modo 3: 240×160, BGR555 direto na VRAM (sem double-buffer).
    fn render_mode3(&mut self, y: u16, vram: &[u8]) {
        let yu = y as usize;
        for x in 0..SCREEN_WIDTH {
            let off = (yu * SCREEN_WIDTH + x) * 2;
            if off + 1 >= vram.len() { break; }
            let color = u16::from_le_bytes([vram[off], vram[off + 1]]);
            self.put_pixel(x, yu, bgr555_to_rgba8(color));
        }
    }

    /// Modo 4: 240×160, paletizado 1 byte/pixel, dois frames (selecionado pelo
    /// bit 4 do DISPCNT).
    fn render_mode4(&mut self, y: u16, vram: &[u8], palette: &[u8]) {
        let frame_base = if self.dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
        let yu = y as usize;
        for x in 0..SCREEN_WIDTH {
            let off = frame_base + yu * SCREEN_WIDTH + x;
            if off >= vram.len() { break; }
            let idx = vram[off] as usize;
            let color = palette_color(palette, idx);
            self.put_pixel(x, yu, color);
        }
    }

    /// Modo 5: 160×128 BGR555 com double-buffer; resto da tela é backdrop.
    fn render_mode5(&mut self, y: u16, vram: &[u8], palette: &[u8]) {
        let frame_base = if self.dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
        let yu = y as usize;
        let backdrop = palette_color(palette, 0);

        for x in 0..SCREEN_WIDTH {
            let pixel = if x < 160 && yu < 128 {
                let off = frame_base + (yu * 160 + x) * 2;
                if off + 1 < vram.len() {
                    bgr555_to_rgba8(u16::from_le_bytes([vram[off], vram[off + 1]]))
                } else {
                    backdrop
                }
            } else {
                backdrop
            };
            self.put_pixel(x, yu, pixel);
        }
    }

    fn render_backdrop(&mut self, y: u16, palette: &[u8]) {
        let color = palette_color(palette, 0);
        let yu = y as usize;
        for x in 0..SCREEN_WIDTH {
            self.put_pixel(x, yu, color);
        }
    }

    fn fill_scanline_white(&mut self, y: u16) {
        let yu = y as usize;
        for x in 0..SCREEN_WIDTH {
            self.put_pixel(x, yu, [0xFF, 0xFF, 0xFF, 0xFF]);
        }
    }

    #[inline]
    fn put_pixel(&mut self, x: usize, y: usize, rgba: [u8; 4]) {
        let off = (y * SCREEN_WIDTH + x) * 4;
        self.framebuffer[off..off + 4].copy_from_slice(&rgba);
    }

    /// Faixa de I/O servida pela PPU: 0x04000000..0x04000056.
    pub fn read_u8(&self, addr: u32) -> u8 {
        match addr {
            0x0400_0000 => self.dispcnt as u8,
            0x0400_0001 => (self.dispcnt >> 8) as u8,
            0x0400_0004 => self.dispstat as u8,
            0x0400_0005 => (self.dispstat >> 8) as u8,
            0x0400_0006 => self.vcount as u8,
            0x0400_0007 => (self.vcount >> 8) as u8,
            _ => 0,
        }
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        match addr {
            0x0400_0000 => self.dispcnt = (self.dispcnt & 0xFF00) | val as u16,
            0x0400_0001 => self.dispcnt = (self.dispcnt & 0x00FF) | ((val as u16) << 8),
            // DISPSTAT: bits 0..2 são read-only (flags).
            0x0400_0004 => {
                let preserve = self.dispstat & 0x0007;
                self.dispstat = (self.dispstat & 0xFF00) | (val as u16 & 0xF8) | preserve;
            }
            0x0400_0005 => self.dispstat = (self.dispstat & 0x00FF) | ((val as u16) << 8),
            // VCOUNT é read-only.
            _ => {}
        }
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

/// Converte BGR555 (16 bits) para RGBA8 (4 bytes).
/// Escala cada componente de 5 bits para 8 bits via (v << 3) | (v >> 2).
#[inline]
pub fn bgr555_to_rgba8(c: u16) -> [u8; 4] {
    let r = (c & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = ((c >> 10) & 0x1F) as u8;
    [(r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2), 0xFF]
}

/// Lê a cor `idx` da palette em RGBA8.
fn palette_color(palette: &[u8], idx: usize) -> [u8; 4] {
    let off = idx * 2;
    if off + 1 >= palette.len() {
        return [0, 0, 0, 0xFF];
    }
    bgr555_to_rgba8(u16::from_le_bytes([palette[off], palette[off + 1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_mem() -> (Vec<u8>, Vec<u8>) {
        (vec![0u8; 0x18000], vec![0u8; 0x400])
    }

    #[test]
    fn vcount_increments_each_scanline() {
        let mut p = Ppu::new();
        let (v, pal) = dummy_mem();
        p.tick(CYCLES_PER_SCANLINE, &v, &pal);
        assert_eq!(p.vcount, 1);
        p.tick(CYCLES_PER_SCANLINE * 3, &v, &pal);
        assert_eq!(p.vcount, 4);
    }

    #[test]
    fn vblank_irq_fires_at_scanline_160_when_enabled() {
        let mut p = Ppu::new();
        let (v, pal) = dummy_mem();
        p.dispstat |= DISPSTAT_VBLANK_IRQ;
        let mut total_irqs = 0u16;
        for _ in 0..160 {
            total_irqs |= p.tick(CYCLES_PER_SCANLINE, &v, &pal);
        }
        assert_eq!(p.vcount, 160);
        assert!(total_irqs & irq_bits::VBLANK != 0);
        assert!(p.dispstat & DISPSTAT_VBLANK_FLAG != 0);
    }

    #[test]
    fn frame_completes_in_280896_cycles() {
        let mut p = Ppu::new();
        let (v, pal) = dummy_mem();
        p.tick(228 * CYCLES_PER_SCANLINE, &v, &pal);
        assert_eq!(p.vcount, 0);
    }

    #[test]
    fn hblank_flag_during_hblank_window() {
        let mut p = Ppu::new();
        let (v, pal) = dummy_mem();
        p.tick(HDRAW_CYCLES + 10, &v, &pal);
        assert!(p.dispstat & DISPSTAT_HBLANK_FLAG != 0);
        p.tick(CYCLES_PER_SCANLINE - HDRAW_CYCLES, &v, &pal);
        assert!(p.dispstat & DISPSTAT_HBLANK_FLAG == 0);
    }

    #[test]
    fn bgr555_red_is_pure_red() {
        let c = bgr555_to_rgba8(0x001F);
        assert_eq!(c, [0xFF, 0, 0, 0xFF]);
    }

    #[test]
    fn mode3_renders_red_pixel() {
        let mut p = Ppu::new();
        p.dispcnt = 3; // mode 3
        let mut v = vec![0u8; 0x18000];
        // Pixel (0, 0) = vermelho puro (BGR555 = 0x001F).
        v[0] = 0x1F;
        v[1] = 0x00;
        let pal = vec![0u8; 0x400];
        // Avança 1 scanline para forçar render.
        p.tick(CYCLES_PER_SCANLINE, &v, &pal);
        assert_eq!(p.framebuffer[0..4], [0xFF, 0, 0, 0xFF]);
    }
}
