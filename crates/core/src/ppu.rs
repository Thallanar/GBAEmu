//! PPU — Picture Processing Unit.
//!
//! Máquina de estados de scanlines (HBlank/VBlank/VCount IRQs) + renderização:
//!   - Modos 0/1/2: backgrounds em tiles (texto e afim), compostos por prioridade.
//!   - Modos 3/4/5: bitmap (BGR555 direto / paletizado / double-buffer).
//!
//! Timing (1 dot = 4 ciclos da CPU):
//!   - 240 dots de HDraw + 68 dots de HBlank = 308 dots = 1232 ciclos/scanline
//!   - 160 scanlines visíveis + 68 de VBlank  = 228 scanlines/frame
//!   - Total: 228 × 1232 = 280 896 ciclos/frame (~59.7 Hz)
//!
//! Sprites (OBJ), janelas, blending e mosaic virão numa próxima iteração.

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
const DISPSTAT_VBLANK_IRQ: u16 = 1 << 3;
const DISPSTAT_HBLANK_IRQ: u16 = 1 << 4;
const DISPSTAT_VCOUNT_IRQ: u16 = 1 << 5;
// bits 8..15: VCount setting (alvo da VCount match interrupt)

pub struct Ppu {
    /// Framebuffer RGBA8, 240×160.
    pub framebuffer: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4]>,

    pub dispcnt: u16,
    pub dispstat: u16,
    pub vcount: u16,

    /// Controle dos 4 backgrounds (BG0CNT..BG3CNT).
    pub bgcnt: [u16; 4],
    /// Scroll horizontal/vertical dos backgrounds de texto (9 bits cada).
    pub bg_hofs: [u16; 4],
    pub bg_vofs: [u16; 4],

    // Parâmetros afins de BG2 (índice 0) e BG3 (índice 1), em ponto fixo 1.7.8.
    bg_pa: [i16; 2],
    bg_pb: [i16; 2],
    bg_pc: [i16; 2],
    bg_pd: [i16; 2],
    /// Pontos de referência BGxX/BGxY (28-bit signed, ponto fixo 1.19.8), valor
    /// dos registradores e a cópia interna incrementada a cada scanline.
    bg_ref_x: [i32; 2],
    bg_ref_y: [i32; 2],
    bg_cur_x: [i32; 2],
    bg_cur_y: [i32; 2],

    /// Ciclos acumulados no scanline atual.
    cycles: u32,
    /// `true` quando passamos do HDraw para HBlank no scanline atual.
    in_hblank: bool,
}

/// Resultado de um `tick`: IRQs a sinalizar + eventos de fase (para o DMA).
#[derive(Default)]
pub struct TickResult {
    /// Bitmap de IRQs a levantar no IF.
    pub irqs: u16,
    /// Entramos no HBlank de uma scanline visível (dispara HBlank-DMA).
    pub entered_hblank: bool,
    /// Entramos no VBlank (scanline 160) — dispara VBlank-DMA.
    pub entered_vblank: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            framebuffer: Box::new([0; SCREEN_WIDTH * SCREEN_HEIGHT * 4]),
            dispcnt: 0,
            dispstat: 0,
            vcount: 0,
            bgcnt: [0; 4],
            bg_hofs: [0; 4],
            bg_vofs: [0; 4],
            bg_pa: [0x0100; 2], // identidade (1.0 em 1.7.8)
            bg_pb: [0; 2],
            bg_pc: [0; 2],
            bg_pd: [0x0100; 2],
            bg_ref_x: [0; 2],
            bg_ref_y: [0; 2],
            bg_cur_x: [0; 2],
            bg_cur_y: [0; 2],
            cycles: 0,
            in_hblank: false,
        }
    }

    /// Avança `cycles` ciclos. Retorna IRQs a sinalizar + eventos de fase.
    /// Recebe slices da VRAM e palette para poder renderizar scanlines.
    pub fn tick(&mut self, cycles: u32, vram: &[u8], palette: &[u8], oam: &[u8]) -> TickResult {
        let mut result = TickResult::default();
        self.cycles += cycles;

        // Pode haver mais de uma transição de fase num único tick.
        loop {
            if !self.in_hblank && self.cycles >= HDRAW_CYCLES {
                // Entra em HBlank. Render do scanline atual (se visível).
                if self.vcount < VISIBLE_SCANLINES {
                    self.render_scanline(self.vcount, vram, palette, oam);
                    // Avança os pontos de referência afins para a próxima linha.
                    self.increment_affine_refs();
                    // HBlank-DMA só dispara em scanlines visíveis.
                    result.entered_hblank = true;
                }
                self.in_hblank = true;
                self.dispstat |= DISPSTAT_HBLANK_FLAG;
                if self.dispstat & DISPSTAT_HBLANK_IRQ != 0 {
                    result.irqs |= irq_bits::HBLANK;
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
                    result.entered_vblank = true;
                    if self.dispstat & DISPSTAT_VBLANK_IRQ != 0 {
                        result.irqs |= irq_bits::VBLANK;
                    }
                } else if self.vcount == 0 {
                    // Novo frame: zera VBlank e recarrega refs afins dos registradores.
                    self.dispstat &= !DISPSTAT_VBLANK_FLAG;
                    self.reload_affine_refs();
                }

                // VCount match.
                let vcount_target = (self.dispstat >> 8) & 0xFF;
                if self.vcount == vcount_target {
                    self.dispstat |= DISPSTAT_VCOUNT_FLAG;
                    if self.dispstat & DISPSTAT_VCOUNT_IRQ != 0 {
                        result.irqs |= irq_bits::VCOUNT;
                    }
                } else {
                    self.dispstat &= !DISPSTAT_VCOUNT_FLAG;
                }
                continue;
            }
            break;
        }
        result
    }

    fn reload_affine_refs(&mut self) {
        for k in 0..2 {
            self.bg_cur_x[k] = self.bg_ref_x[k];
            self.bg_cur_y[k] = self.bg_ref_y[k];
        }
    }

    fn increment_affine_refs(&mut self) {
        for k in 0..2 {
            self.bg_cur_x[k] = self.bg_cur_x[k].wrapping_add(self.bg_pb[k] as i32);
            self.bg_cur_y[k] = self.bg_cur_y[k].wrapping_add(self.bg_pd[k] as i32);
        }
    }

    // ───────────────── Render ─────────────────

    fn render_scanline(&mut self, y: u16, vram: &[u8], palette: &[u8], oam: &[u8]) {
        let yu = y as usize;
        let mode = self.dispcnt & 0b111;

        if self.dispcnt & 0x80 != 0 {
            self.fill_scanline_white(y);
            return;
        }

        // Buffer de linha + prioridade por pixel (4 = backdrop, atrás de tudo).
        let backdrop = palette_color(palette, 0);
        let mut line = [backdrop; SCREEN_WIDTH];
        let mut prio = [4u8; SCREEN_WIDTH];

        match mode {
            0 => self.render_bgs(
                yu,
                vram,
                palette,
                &mut line,
                &mut prio,
                &[false, false, false, false],
            ),
            1 => self.render_bgs(
                yu,
                vram,
                palette,
                &mut line,
                &mut prio,
                &[false, false, true, false],
            ),
            2 => self.render_bgs(
                yu,
                vram,
                palette,
                &mut line,
                &mut prio,
                &[false, false, true, true],
            ),
            3 => self.render_mode3(yu, vram, &mut line, &mut prio),
            4 => self.render_mode4(yu, vram, palette, &mut line, &mut prio),
            5 => self.render_mode5(yu, vram, palette, &mut line, &mut prio),
            _ => {}
        }

        // Sprites (OBJ) — habilitados pelo bit 12 do DISPCNT.
        if self.dispcnt & (1 << 12) != 0 {
            self.render_sprites(yu, vram, palette, oam, &mut line, &mut prio);
        }

        for (x, px) in line.iter().enumerate() {
            self.put_pixel(x, yu, *px);
        }
    }

    /// Compõe os backgrounds habilitados de um modo em tiles, do mais ao fundo
    /// para o mais à frente. `affine[b]` indica se o BG `b` é afim.
    fn render_bgs(
        &self,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
        affine: &[bool; 4],
    ) {
        // Coleta os BGs habilitados com (prioridade, índice, afim).
        let mut order: [(u8, usize, bool); 4] = [(0, 0, false); 4];
        let mut count = 0;
        for (bg, &is_affine) in affine.iter().enumerate() {
            if self.dispcnt & (1 << (8 + bg)) != 0 {
                let p = (self.bgcnt[bg] & 0b11) as u8;
                order[count] = (p, bg, is_affine);
                count += 1;
            }
        }
        // Ordena de trás para frente: maior prioridade-número e maior índice
        // primeiro; assim o último pintado (prioridade 0, BG0) fica na frente.
        let active = &mut order[..count];
        active.sort_by_key(|&(p, bg, _)| std::cmp::Reverse((p, bg)));

        for &(p, bg, is_affine) in active.iter() {
            if is_affine {
                self.render_affine_bg(bg, p, vram, palette, line, prio);
            } else {
                self.render_text_bg(bg, y, p, vram, palette, line, prio);
            }
        }
    }

    /// Renderiza uma scanline de um background em modo texto, pintando os pixels
    /// opacos sobre `line` e registrando a prioridade `p` em `prio`.
    #[allow(clippy::too_many_arguments)]
    fn render_text_bg(
        &self,
        bg: usize,
        y: usize,
        p: u8,
        vram: &[u8],
        palette: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        let cnt = self.bgcnt[bg];
        let char_base = (((cnt >> 2) & 0b11) as usize) * 0x4000;
        let is_8bpp = cnt & (1 << 7) != 0;
        let screen_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let size = (cnt >> 14) & 0b11;
        let (width, height) = match size {
            0 => (256usize, 256usize),
            1 => (512, 256),
            2 => (256, 512),
            _ => (512, 512),
        };

        let hofs = (self.bg_hofs[bg] & 0x1FF) as usize;
        let vofs = (self.bg_vofs[bg] & 0x1FF) as usize;
        let by = (y + vofs) & (height - 1);
        let map_y = by / 8;
        let py = by % 8;

        for x in 0..SCREEN_WIDTH {
            let bx = (x + hofs) & (width - 1);
            let map_x = bx / 8;

            // Seleção do screenblock (32×32 tiles cada) dentro do BG, conforme
            // o tamanho. Só larguras/alturas > 256 usam blocos extras.
            let sb = match size {
                0 => 0,                               // 256×256
                1 => map_x / 32,                      // 512×256
                2 => map_y / 32,                      // 256×512
                _ => (map_y / 32) * 2 + (map_x / 32), // 512×512
            };
            let tx = map_x % 32;
            let ty = map_y % 32;
            let entry_addr = screen_base + sb * 0x800 + (ty * 32 + tx) * 2;
            if entry_addr + 1 >= vram.len() {
                continue;
            }
            let entry = u16::from_le_bytes([vram[entry_addr], vram[entry_addr + 1]]);
            let tile_num = (entry & 0x3FF) as usize;
            let hflip = entry & (1 << 10) != 0;
            let vflip = entry & (1 << 11) != 0;
            let pal_bank = ((entry >> 12) & 0xF) as usize;

            let fx = if hflip { 7 - (bx % 8) } else { bx % 8 };
            let fy = if vflip { 7 - py } else { py };

            let color_idx = if is_8bpp {
                let addr = char_base + tile_num * 64 + fy * 8 + fx;
                if addr >= vram.len() {
                    continue;
                }
                vram[addr] as usize
            } else {
                let addr = char_base + tile_num * 32 + fy * 4 + fx / 2;
                if addr >= vram.len() {
                    continue;
                }
                let byte = vram[addr];
                let nibble = if fx & 1 == 0 { byte & 0xF } else { byte >> 4 } as usize;
                if nibble == 0 {
                    continue; // transparente
                }
                pal_bank * 16 + nibble
            };

            if color_idx == 0 {
                continue; // transparente
            }
            line[x] = palette_color(palette, color_idx);
            prio[x] = p;
        }
    }

    /// Renderiza uma scanline de um background afim (sempre 8bpp, mapa quadrado).
    fn render_affine_bg(
        &self,
        bg: usize,
        p: u8,
        vram: &[u8],
        palette: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        let k = bg - 2; // BG2 → 0, BG3 → 1
        let cnt = self.bgcnt[bg];
        let char_base = (((cnt >> 2) & 0b11) as usize) * 0x4000;
        let screen_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let wrap = cnt & (1 << 13) != 0;
        let size = (cnt >> 14) & 0b11;
        let dim = (128usize) << size; // 128/256/512/1024 pixels
        let tiles_wide = dim / 8;

        let pa = self.bg_pa[k] as i32;
        let pc = self.bg_pc[k] as i32;
        let mut tex_x = self.bg_cur_x[k];
        let mut tex_y = self.bg_cur_y[k];

        for x in 0..SCREEN_WIDTH {
            let sx = tex_x >> 8;
            let sy = tex_y >> 8;
            tex_x = tex_x.wrapping_add(pa);
            tex_y = tex_y.wrapping_add(pc);

            let (mx, my) = if wrap {
                (
                    (sx.rem_euclid(dim as i32)) as usize,
                    (sy.rem_euclid(dim as i32)) as usize,
                )
            } else {
                if sx < 0 || sy < 0 || sx >= dim as i32 || sy >= dim as i32 {
                    continue; // fora do mapa → transparente
                }
                (sx as usize, sy as usize)
            };

            let map_x = mx / 8;
            let map_y = my / 8;
            let entry_addr = screen_base + map_y * tiles_wide + map_x;
            if entry_addr >= vram.len() {
                continue;
            }
            let tile_num = vram[entry_addr] as usize;
            let addr = char_base + tile_num * 64 + (my % 8) * 8 + (mx % 8);
            if addr >= vram.len() {
                continue;
            }
            let color_idx = vram[addr] as usize;
            if color_idx == 0 {
                continue; // transparente
            }
            line[x] = palette_color(palette, color_idx);
            prio[x] = p;
        }
    }

    /// Prioridade efetiva de BG2 (camada onde os modos bitmap desenham).
    fn bg2_priority(&self) -> u8 {
        (self.bgcnt[2] & 0b11) as u8
    }

    /// Modo 3: 240×160, BGR555 direto na VRAM (sem double-buffer).
    fn render_mode3(
        &self,
        y: usize,
        vram: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        if self.dispcnt & (1 << 10) == 0 {
            return; // BG2 desabilitado
        }
        let p = self.bg2_priority();
        for x in 0..SCREEN_WIDTH {
            let off = (y * SCREEN_WIDTH + x) * 2;
            if off + 1 >= vram.len() {
                break;
            }
            let color = u16::from_le_bytes([vram[off], vram[off + 1]]);
            line[x] = bgr555_to_rgba8(color);
            prio[x] = p;
        }
    }

    /// Modo 4: 240×160, paletizado 1 byte/pixel, dois frames (selecionado pelo
    /// bit 4 do DISPCNT).
    fn render_mode4(
        &self,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        if self.dispcnt & (1 << 10) == 0 {
            return;
        }
        let p = self.bg2_priority();
        let frame_base = if self.dispcnt & (1 << 4) != 0 {
            0xA000
        } else {
            0
        };
        for x in 0..SCREEN_WIDTH {
            let off = frame_base + y * SCREEN_WIDTH + x;
            if off >= vram.len() {
                break;
            }
            let idx = vram[off] as usize;
            if idx == 0 {
                continue; // índice 0 = transparente (mostra backdrop)
            }
            line[x] = palette_color(palette, idx);
            prio[x] = p;
        }
    }

    /// Modo 5: 160×128 BGR555 com double-buffer; resto da tela fica como backdrop.
    fn render_mode5(
        &self,
        y: usize,
        vram: &[u8],
        _palette: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        if self.dispcnt & (1 << 10) == 0 {
            return;
        }
        let p = self.bg2_priority();
        let frame_base = if self.dispcnt & (1 << 4) != 0 {
            0xA000
        } else {
            0
        };
        for x in 0..160.min(SCREEN_WIDTH) {
            if y >= 128 {
                break;
            }
            let off = frame_base + (y * 160 + x) * 2;
            if off + 1 >= vram.len() {
                break;
            }
            line[x] = bgr555_to_rgba8(u16::from_le_bytes([vram[off], vram[off + 1]]));
            prio[x] = p;
        }
    }

    fn fill_scanline_white(&mut self, y: u16) {
        let yu = y as usize;
        for x in 0..SCREEN_WIDTH {
            self.put_pixel(x, yu, [0xFF, 0xFF, 0xFF, 0xFF]);
        }
    }

    /// Renderiza os sprites (OBJ) que cobrem a scanline `y` e os compõe sobre
    /// `line` respeitando a prioridade frente-a-frente com os backgrounds.
    ///
    /// Ordem entre sprites: o de menor índice de OAM fica por cima (independe da
    /// prioridade). Já a prioridade do sprite decide quem vence o BG no pixel.
    fn render_sprites(
        &self,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        oam: &[u8],
        line: &mut [[u8; 4]; SCREEN_WIDTH],
        prio: &mut [u8; SCREEN_WIDTH],
    ) {
        // Buffer dos sprites: cor + prioridade por pixel (255 = vazio).
        let mut obj = [[0u8; 4]; SCREEN_WIDTH];
        let mut obj_prio = [255u8; SCREEN_WIDTH];

        let one_d = self.dispcnt & (1 << 6) != 0;
        let rd16 = |off: usize| u16::from_le_bytes([oam[off], oam[off + 1]]);

        for i in 0..128 {
            let base = i * 8;
            let attr0 = rd16(base);
            let attr1 = rd16(base + 2);
            let attr2 = rd16(base + 4);

            let obj_mode = (attr0 >> 8) & 0b11;
            if obj_mode == 2 {
                continue; // sprite desabilitado
            }
            let affine = obj_mode == 1 || obj_mode == 3;
            let double = obj_mode == 3;

            let shape = (attr0 >> 14) & 0b11;
            let size = (attr1 >> 14) & 0b11;
            let (w, h) = sprite_dims(shape, size);
            let (bw, bh) = if double { (w * 2, h * 2) } else { (w, h) };

            // Linha relativa ao topo do sprite, com wrap em 256.
            let y0 = (attr0 & 0xFF) as i32;
            let row = (y as i32 - y0) & 0xFF;
            if row >= bh as i32 {
                continue; // sprite não cobre esta scanline
            }

            let x0 = (attr1 & 0x1FF) as i32;
            let is_8bpp = attr0 & (1 << 13) != 0;
            let tile_base = (attr2 & 0x3FF) as usize;
            let priority = ((attr2 >> 10) & 0b11) as u8;
            let pal_bank = ((attr2 >> 12) & 0xF) as usize;

            // Parâmetros afins (interleaved em OAM): grupo = bits 9-13 de attr1.
            let (pa, pb, pc, pd) = if affine {
                let g = (((attr1 >> 9) & 0x1F) as usize) * 0x20;
                (
                    rd16(g + 0x6) as i16 as i32,
                    rd16(g + 0xE) as i16 as i32,
                    rd16(g + 0x16) as i16 as i32,
                    rd16(g + 0x1E) as i16 as i32,
                )
            } else {
                (0, 0, 0, 0)
            };
            let hflip = !affine && attr1 & (1 << 12) != 0;
            let vflip = !affine && attr1 & (1 << 13) != 0;

            for col in 0..bw as i32 {
                let screen_x = (x0 + col) & 0x1FF;
                if screen_x >= SCREEN_WIDTH as i32 {
                    continue;
                }
                let sx = screen_x as usize;
                if obj_prio[sx] != 255 {
                    continue; // já pintado por sprite de índice menor
                }

                // Coordenada de textura (tex_x, tex_y) dentro do sprite [0,w)×[0,h).
                let (tex_x, tex_y) = if affine {
                    let ix = col - bw as i32 / 2;
                    let iy = row - bh as i32 / 2;
                    let tx = ((pa * ix + pb * iy) >> 8) + w as i32 / 2;
                    let ty = ((pc * ix + pd * iy) >> 8) + h as i32 / 2;
                    (tx, ty)
                } else {
                    let tx = if hflip { w as i32 - 1 - col } else { col };
                    let ty = if vflip { h as i32 - 1 - row } else { row };
                    (tx, ty)
                };
                if tex_x < 0 || tex_y < 0 || tex_x >= w as i32 || tex_y >= h as i32 {
                    continue;
                }

                if let Some(color) = self.sample_sprite(
                    vram,
                    palette,
                    tile_base,
                    is_8bpp,
                    pal_bank,
                    one_d,
                    w,
                    tex_x as usize,
                    tex_y as usize,
                ) {
                    obj[sx] = color;
                    obj_prio[sx] = priority;
                }
            }
        }

        // Composição: o sprite vence o BG quando sua prioridade ≤ a do pixel.
        for x in 0..SCREEN_WIDTH {
            if obj_prio[x] != 255 && obj_prio[x] <= prio[x] {
                line[x] = obj[x];
                prio[x] = obj_prio[x];
            }
        }
    }

    /// Amostra um texel de sprite. Devolve `None` se transparente (índice 0).
    #[allow(clippy::too_many_arguments)]
    fn sample_sprite(
        &self,
        vram: &[u8],
        palette: &[u8],
        tile_base: usize,
        is_8bpp: bool,
        pal_bank: usize,
        one_d: bool,
        w: usize,
        tex_x: usize,
        tex_y: usize,
    ) -> Option<[u8; 4]> {
        const OBJ_TILE_BASE: usize = 0x10000; // área de tiles de OBJ na VRAM
        const OBJ_PAL_BASE: usize = 0x100; // paleta de OBJ começa na cor 256

        let tile_x = tex_x / 8;
        let tile_y = tex_y / 8;
        let px = tex_x % 8;
        let py = tex_y % 8;

        // Mapeamento de tiles: 1D = linear (largura do sprite); 2D = grade de 32.
        let row_tiles = if one_d { w / 8 } else { 32 };
        let step = if is_8bpp { 2 } else { 1 }; // tiles de 8bpp ocupam 2 slots de 32 bytes
        let tile_id = tile_base + (tile_y * row_tiles + tile_x) * step;

        let color_idx = if is_8bpp {
            let addr = OBJ_TILE_BASE + tile_id * 32 + py * 8 + px;
            if addr >= vram.len() {
                return None;
            }
            let idx = vram[addr] as usize;
            if idx == 0 {
                return None;
            }
            OBJ_PAL_BASE + idx
        } else {
            let addr = OBJ_TILE_BASE + tile_id * 32 + py * 4 + px / 2;
            if addr >= vram.len() {
                return None;
            }
            let byte = vram[addr];
            let nibble = if px & 1 == 0 { byte & 0xF } else { byte >> 4 } as usize;
            if nibble == 0 {
                return None;
            }
            OBJ_PAL_BASE + pal_bank * 16 + nibble
        };

        Some(palette_color(palette, color_idx))
    }

    #[inline]
    fn put_pixel(&mut self, x: usize, y: usize, rgba: [u8; 4]) {
        let off = (y * SCREEN_WIDTH + x) * 4;
        self.framebuffer[off..off + 4].copy_from_slice(&rgba);
    }

    /// Faixa de I/O servida pela PPU: 0x04000000..0x0400005F.
    pub fn read_u8(&self, addr: u32) -> u8 {
        let reg = addr & 0xFF;
        match reg {
            0x00 => self.dispcnt as u8,
            0x01 => (self.dispcnt >> 8) as u8,
            0x04 => self.dispstat as u8,
            0x05 => (self.dispstat >> 8) as u8,
            0x06 => self.vcount as u8,
            0x07 => (self.vcount >> 8) as u8,
            // BGxCNT (0x08..0x0F) — legíveis.
            0x08..=0x0F => {
                let bg = ((reg - 0x08) / 2) as usize;
                let v = self.bgcnt[bg];
                if reg & 1 == 0 {
                    v as u8
                } else {
                    (v >> 8) as u8
                }
            }
            _ => 0, // scroll/afins são write-only
        }
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let reg = addr & 0xFF;
        let v = val as u16;
        match reg {
            0x00 => self.dispcnt = (self.dispcnt & 0xFF00) | v,
            0x01 => self.dispcnt = (self.dispcnt & 0x00FF) | (v << 8),
            // DISPSTAT: bits 0..2 são read-only (flags).
            0x04 => {
                let preserve = self.dispstat & 0x0007;
                self.dispstat = (self.dispstat & 0xFF00) | (v & 0xF8) | preserve;
            }
            0x05 => self.dispstat = (self.dispstat & 0x00FF) | (v << 8),
            // VCOUNT (0x06/0x07) é read-only.
            0x08..=0x0F => {
                let bg = ((reg - 0x08) / 2) as usize;
                if reg & 1 == 0 {
                    self.bgcnt[bg] = (self.bgcnt[bg] & 0xFF00) | v;
                } else {
                    self.bgcnt[bg] = (self.bgcnt[bg] & 0x00FF) | (v << 8);
                }
            }
            // BGxHOFS/BGxVOFS (0x10..0x1F), write-only.
            0x10..=0x1F => {
                let bg = ((reg - 0x10) / 4) as usize;
                let is_v = (reg & 0b10) != 0;
                let target = if is_v {
                    &mut self.bg_vofs[bg]
                } else {
                    &mut self.bg_hofs[bg]
                };
                if reg & 1 == 0 {
                    *target = (*target & 0xFF00) | v;
                } else {
                    *target = (*target & 0x00FF) | (v << 8);
                }
            }
            // Parâmetros afins de BG2 (0x20..0x2F) e BG3 (0x30..0x3F).
            0x20..=0x2F => self.write_affine_reg(0, reg - 0x20, val),
            0x30..=0x3F => self.write_affine_reg(1, reg - 0x30, val),
            _ => {}
        }
    }

    /// Escreve um byte num registrador afim do BG `k` (0=BG2, 1=BG3).
    /// `off` é o deslocamento dentro do bloco de 0x10 bytes (PA/PB/PC/PD/X/Y).
    fn write_affine_reg(&mut self, k: usize, off: u32, val: u8) {
        let v = val as u16;
        let set16 = |dst: &mut i16, lo_byte: bool| {
            let cur = *dst as u16;
            *dst = if lo_byte {
                (cur & 0xFF00) | v
            } else {
                (cur & 0x00FF) | (v << 8)
            } as i16;
        };
        match off {
            0x0 => set16(&mut self.bg_pa[k], true),
            0x1 => set16(&mut self.bg_pa[k], false),
            0x2 => set16(&mut self.bg_pb[k], true),
            0x3 => set16(&mut self.bg_pb[k], false),
            0x4 => set16(&mut self.bg_pc[k], true),
            0x5 => set16(&mut self.bg_pc[k], false),
            0x6 => set16(&mut self.bg_pd[k], true),
            0x7 => set16(&mut self.bg_pd[k], false),
            // BGxX (0x8..0xB) e BGxY (0xC..0xF): 28-bit signed. Escrever também
            // atualiza a cópia interna usada na renderização.
            0x8..=0xB => {
                let sh = (off - 0x8) * 8;
                let raw = (self.bg_ref_x[k] as u32 & !(0xFF << sh)) | ((val as u32) << sh);
                self.bg_ref_x[k] = sign_extend_28(raw);
                self.bg_cur_x[k] = self.bg_ref_x[k];
            }
            0xC..=0xF => {
                let sh = (off - 0xC) * 8;
                let raw = (self.bg_ref_y[k] as u32 & !(0xFF << sh)) | ((val as u32) << sh);
                self.bg_ref_y[k] = sign_extend_28(raw);
                self.bg_cur_y[k] = self.bg_ref_y[k];
            }
            _ => {}
        }
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

/// Sign-extend de um valor de 28 bits (registradores BGxX/BGxY) para i32.
fn sign_extend_28(v: u32) -> i32 {
    ((v << 4) as i32) >> 4
}

/// Dimensões (largura, altura) de um sprite em pixels, conforme shape × size.
fn sprite_dims(shape: u16, size: u16) -> (usize, usize) {
    match (shape, size) {
        (0, 0) => (8, 8),
        (0, 1) => (16, 16),
        (0, 2) => (32, 32),
        (0, 3) => (64, 64),
        (1, 0) => (16, 8),
        (1, 1) => (32, 8),
        (1, 2) => (32, 16),
        (1, 3) => (64, 32),
        (2, 0) => (8, 16),
        (2, 1) => (8, 32),
        (2, 2) => (16, 32),
        (2, 3) => (32, 64),
        _ => (8, 8),
    }
}

/// Converte BGR555 (16 bits) para RGBA8 (4 bytes).
/// Escala cada componente de 5 bits para 8 bits via (v << 3) | (v >> 2).
#[inline]
pub fn bgr555_to_rgba8(c: u16) -> [u8; 4] {
    let r = (c & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = ((c >> 10) & 0x1F) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 3) | (g >> 2),
        (b << 3) | (b >> 2),
        0xFF,
    ]
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

    fn dummy_mem() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (vec![0u8; 0x18000], vec![0u8; 0x400], vec![0u8; 0x400])
    }

    #[test]
    fn vcount_increments_each_scanline() {
        let mut p = Ppu::new();
        let (v, pal, oam) = dummy_mem();
        p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam);
        assert_eq!(p.vcount, 1);
        p.tick(CYCLES_PER_SCANLINE * 3, &v, &pal, &oam);
        assert_eq!(p.vcount, 4);
    }

    #[test]
    fn vblank_irq_fires_at_scanline_160_when_enabled() {
        let mut p = Ppu::new();
        let (v, pal, oam) = dummy_mem();
        p.dispstat |= DISPSTAT_VBLANK_IRQ;
        let mut total_irqs = 0u16;
        for _ in 0..160 {
            total_irqs |= p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam).irqs;
        }
        assert_eq!(p.vcount, 160);
        assert!(total_irqs & irq_bits::VBLANK != 0);
        assert!(p.dispstat & DISPSTAT_VBLANK_FLAG != 0);
    }

    #[test]
    fn frame_completes_in_280896_cycles() {
        let mut p = Ppu::new();
        let (v, pal, oam) = dummy_mem();
        p.tick(228 * CYCLES_PER_SCANLINE, &v, &pal, &oam);
        assert_eq!(p.vcount, 0);
    }

    #[test]
    fn hblank_flag_during_hblank_window() {
        let mut p = Ppu::new();
        let (v, pal, oam) = dummy_mem();
        p.tick(HDRAW_CYCLES + 10, &v, &pal, &oam);
        assert!(p.dispstat & DISPSTAT_HBLANK_FLAG != 0);
        p.tick(CYCLES_PER_SCANLINE - HDRAW_CYCLES, &v, &pal, &oam);
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
        p.dispcnt = 3 | (1 << 10); // mode 3, BG2 habilitado
        let mut v = vec![0u8; 0x18000];
        // Pixel (0, 0) = vermelho puro (BGR555 = 0x001F).
        v[0] = 0x1F;
        v[1] = 0x00;
        let pal = vec![0u8; 0x400];
        let oam = vec![0u8; 0x400];
        // Avança 1 scanline para forçar render.
        p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam);
        assert_eq!(p.framebuffer[0..4], [0xFF, 0, 0, 0xFF]);
    }

    /// Modo 0, BG0 em texto 4bpp: monta 1 tile e verifica o pixel (0,0).
    #[test]
    fn mode0_text_bg_renders_tile_pixel() {
        let mut p = Ppu::new();
        let mut v = vec![0u8; 0x18000];
        let mut pal = vec![0u8; 0x400];

        // BG0 habilitado (DISPCNT bit 8), modo 0.
        p.dispcnt = 0x0100;
        // BG0CNT: char base 0 (block 0 = 0x0000), screen base block 1 (0x0800),
        // 4bpp, tamanho 0. screen_base = 1*0x800.
        p.bgcnt[0] = 1 << 8;

        // Map entry (tile 0, sem flip, palette bank 1) no screenblock 1, tile (0,0).
        let entry: u16 = 1 << 12; // pal_bank = 1
        v[0x800] = entry as u8;
        v[0x801] = (entry >> 8) as u8;

        // Tile 0 em char base 0: primeiro pixel (nibble baixo) = índice 1.
        v[0] = 0x01;

        // Palette: cor de (bank 1, índice 1) = palette[1*16 + 1] = índice 17.
        // BGR555 verde puro = 0x03E0 → (0,255,0).
        let color: u16 = 0x03E0;
        pal[17 * 2] = color as u8;
        pal[17 * 2 + 1] = (color >> 8) as u8;

        let oam = vec![0u8; 0x400];
        p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam);
        assert_eq!(p.framebuffer[0..4], [0, 0xFF, 0, 0xFF]);
    }

    /// Prioridade: BG1 (prioridade 0) deve cobrir BG0 (prioridade 1) no mesmo pixel.
    #[test]
    fn bg_priority_higher_covers_lower() {
        let mut p = Ppu::new();
        let mut v = vec![0u8; 0x18000];
        let mut pal = vec![0u8; 0x400];

        // BG0 e BG1 habilitados.
        p.dispcnt = 0x0300;
        // BG0: prioridade 1, screen base 1. BG1: prioridade 0, screen base 2.
        p.bgcnt[0] = 1 | (1 << 8);
        p.bgcnt[1] = 2 << 8; // prioridade 0, screen base 2

        // Tile 0, pixel 0 = índice 1 (4bpp), compartilhado pelos dois BGs.
        v[0] = 0x01;
        // BG0 entry (screen base 1 = 0x0800): tile 0, bank 0.
        v[0x0800] = 0x00;
        v[0x0801] = 0x00;
        // BG1 entry (screen base 2 = 0x1000): tile 0, bank 2 (para distinguir a cor).
        let entry_bg1: u16 = 2 << 12;
        v[0x1000] = entry_bg1 as u8;
        v[0x1001] = (entry_bg1 >> 8) as u8;

        // BG0 bank 0 índice 1 = palette[1] = vermelho 0x001F.
        pal[2] = 0x1F;
        // BG1 bank 2 índice 1 = palette[33] = azul 0x7C00.
        let blue: u16 = 0x7C00;
        pal[33 * 2] = blue as u8;
        pal[33 * 2 + 1] = (blue >> 8) as u8;

        let oam = vec![0u8; 0x400];
        p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam);
        // BG1 (prioridade 0) deve vencer → azul.
        assert_eq!(p.framebuffer[0..4], [0, 0, 0xFF, 0xFF]);
    }

    /// Sprite 4bpp em (0,0): monta 1 tile de OBJ e verifica que cobre o backdrop.
    #[test]
    fn sprite_renders_over_backdrop() {
        let mut p = Ppu::new();
        let mut v = vec![0u8; 0x18000];
        let mut pal = vec![0u8; 0x400];
        // OAM todo zerado já descreve sprite 0: y=0, x=0, 8×8 square, tile 0.
        let oam = vec![0u8; 0x400];

        // Modo 0, OBJ habilitado (bit 12), mapeamento 1D (bit 6).
        p.dispcnt = (1 << 12) | (1 << 6);

        // Tile 0 de OBJ (base 0x10000): pixel (0,0) nibble baixo = índice 1.
        v[0x10000] = 0x01;

        // Paleta de OBJ: cor 256 + (bank 0)*16 + 1 = índice 257 → amarelo.
        // BGR555 amarelo = R+G = 0x03FF.
        let yellow: u16 = 0x03FF;
        pal[257 * 2] = yellow as u8;
        pal[257 * 2 + 1] = (yellow >> 8) as u8;

        p.tick(CYCLES_PER_SCANLINE, &v, &pal, &oam);
        assert_eq!(p.framebuffer[0..4], bgr555_to_rgba8(yellow));
    }
}
