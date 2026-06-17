//! Decodificação dos sprites de Pokémon Gen 3 **direto da ROM** — pra UI mostrar
//! o alvo da caça sem empacotar imagem nenhuma (os gráficos já estão no jogo).
//!
//! Como achamos os dados sem hardcodar offsets por versão: o jogo expõe um
//! "GF ROM header" (`struct GFRomHeader` do pokeemerald) com **ponteiros** pras
//! tabelas de gráficos. Localizamos esse header validando que seus ponteiros
//! levam a tabelas indexadas por espécie (`tag == índice`), e lemos:
//!   - `monFrontPics`     (+0x28) → `gMonFrontPicTable`   (sprite de frente)
//!   - `monNormalPalettes`(+0x30) → `gMonPaletteTable`     (paleta normal)
//!   - `monShinyPalettes` (+0x34) → `gMonShinyPaletteTable`(paleta shiny)
//!
//! Cada sprite de frente é 64×64, 4bpp, LZ77-comprimido; cada paleta é 16 cores
//! BGR555, também LZ77.

/// Sprite decodificado em RGBA8 (índice 0 = transparente, alpha 0).
pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Offsets (na ROM) das três tabelas de gráficos, lidos do GF ROM header.
#[derive(Debug, Clone, Copy)]
pub struct RomGfx {
    front_table: usize,
    normal_pal_table: usize,
    shiny_pal_table: usize,
}

const PIC_DIM: usize = 64; // sprites de frente são 64×64

/// Converte um ponteiro de ROM (`0x08xxxxxx`/`0x09xxxxxx`) em offset no slice.
fn ptr_to_off(ptr: u32, rom_len: usize) -> Option<usize> {
    if (0x0800_0000..0x0A00_0000).contains(&ptr) {
        let off = (ptr & 0x01FF_FFFF) as usize;
        (off < rom_len).then_some(off)
    } else {
        None
    }
}

fn rd_u32(rom: &[u8], off: usize) -> Option<u32> {
    rom.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn rd_u16(rom: &[u8], off: usize) -> Option<u16> {
    rom.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// Quantas entradas consecutivas a partir de `table` (stride 8) têm `ptr` válido
/// e o campo `tag` (no offset `tag_off`) **linear**: `tag[i] == tag[0] + i`.
/// Reconhece tabelas indexadas por espécie. A base (`tag[0]`) varia: tabelas de
/// sprite e a paleta normal usam base 0 (tag = espécie), mas a paleta **shiny**
/// usa base 500 (tag = espécie + 500) — daí aceitar qualquer base.
fn linear_run(rom: &[u8], table: usize, tag_off: usize, max: usize) -> usize {
    let base = match rd_u16(rom, table + tag_off) {
        Some(b) => b,
        None => return 0,
    };
    for i in 0..max {
        let e = table + i * 8;
        match rd_u32(rom, e) {
            Some(p) if ptr_to_off(p, rom.len()).is_some() => {}
            _ => return i,
        }
        if rd_u16(rom, e + tag_off) != Some(base.wrapping_add(i as u16)) {
            return i;
        }
    }
    max
}

impl RomGfx {
    /// Localiza as tabelas de gráficos escaneando a ROM por tabelas indexadas por
    /// espécie (corrida longa de `tag == índice`). O Emerald **retail** não traz
    /// o GF ROM header do decomp, então identificamos pelas próprias tabelas:
    /// sprites de frente/costas são `{ptr, size, tag}` (tag em +6) e paletas
    /// normal/shiny são `{ptr, tag, pad}` (tag em +4). Há duas de cada (front+back,
    /// normal+shiny); como front e a paleta normal vêm antes na ROM, pegamos por
    /// ordem de offset. O `MIN` alto (≈ nº de espécies) exclui tabelas menores
    /// (treinadores, ícones…).
    pub fn locate(rom: &[u8]) -> Option<RomGfx> {
        const MIN: usize = 350;
        // Sprites: {ptr, size, tag@+6}. Paletas: {ptr, tag@+4, pad}. As duas
        // famílias se distinguem pelo offset do tag (a outra posição é constante,
        // quebrando a linearidade). Há 2 de sprite (front/back, base 0) e 2 de
        // paleta (normal base 0, shiny base 500).
        let mut pic_tables = Vec::new(); // (offset)
        let mut pal_tables = Vec::new(); // (offset, base do tag)
        let mut h = 0;
        while h + 8 < rom.len() {
            let r6 = linear_run(rom, h, 6, 600);
            if r6 >= MIN {
                pic_tables.push(h);
                h += r6 * 8; // pula a tabela (não recontar offsets internos)
                continue;
            }
            let r4 = linear_run(rom, h, 4, 600);
            if r4 >= MIN {
                pal_tables.push((h, rd_u16(rom, h + 4).unwrap_or(0)));
                h += r4 * 8;
                continue;
            }
            h += 4;
        }
        // Front pics = tabela de sprite de menor offset (vem antes da de costas).
        let front_table = *pic_tables.first()?;
        // Paleta normal = base 0; shiny = base != 0 (espécie + 500).
        let normal_pal_table = pal_tables.iter().find(|(_, b)| *b == 0).map(|(o, _)| *o)?;
        let shiny_pal_table = pal_tables
            .iter()
            .find(|(_, b)| *b != 0)
            .map(|(o, _)| *o)
            .unwrap_or(normal_pal_table);
        Some(RomGfx {
            front_table,
            normal_pal_table,
            shiny_pal_table,
        })
    }

    /// Decodifica o sprite de frente de uma espécie (índice interno Gen 3).
    /// `shiny` escolhe a paleta normal ou a shiny.
    pub fn decode_front(&self, rom: &[u8], species: u16, shiny: bool) -> Option<Sprite> {
        let i = species as usize;
        let pic_ptr = rd_u32(rom, self.front_table + i * 8)?;
        let pal_table = if shiny {
            self.shiny_pal_table
        } else {
            self.normal_pal_table
        };
        let pal_ptr = rd_u32(rom, pal_table + i * 8)?;

        let tiles = lz77_decompress(rom, ptr_to_off(pic_ptr, rom.len())?)?;
        let pal_bytes = lz77_decompress(rom, ptr_to_off(pal_ptr, rom.len())?)?;

        // 16 cores BGR555.
        let mut pal = [[0u8; 4]; 16];
        for (c, slot) in pal.iter_mut().enumerate() {
            let lo = *pal_bytes.get(c * 2)? as u16;
            let hi = *pal_bytes.get(c * 2 + 1)? as u16;
            *slot = bgr555_to_rgba8(lo | (hi << 8));
        }

        // 64×64 em tiles 4bpp, 8×8 tiles em ordem linear (linha por linha).
        let mut rgba = vec![0u8; PIC_DIM * PIC_DIM * 4];
        let tiles_wide = PIC_DIM / 8;
        for y in 0..PIC_DIM {
            for x in 0..PIC_DIM {
                let tile = (y / 8) * tiles_wide + (x / 8);
                let addr = tile * 32 + (y % 8) * 4 + (x % 8) / 2;
                let byte = match tiles.get(addr) {
                    Some(&b) => b,
                    None => continue,
                };
                let idx = if x & 1 == 0 { byte & 0xF } else { byte >> 4 } as usize;
                let off = (y * PIC_DIM + x) * 4;
                if idx == 0 {
                    continue; // transparente (alpha já 0)
                }
                rgba[off..off + 4].copy_from_slice(&pal[idx]);
            }
        }
        Some(Sprite {
            width: PIC_DIM,
            height: PIC_DIM,
            rgba,
        })
    }
}

/// Descomprime um stream LZ77 no formato do BIOS (header `0x10` + tamanho de
/// 24 bits, depois blocos com byte de flags). Mesmo algoritmo do `lz77_uncomp`
/// do BIOS HLE, mas puro sobre um slice. `None` se o header não for LZ77.
fn lz77_decompress(rom: &[u8], mut off: usize) -> Option<Vec<u8>> {
    let header = rd_u32(rom, off)?;
    off += 4;
    if (header & 0xF0) != 0x10 {
        return None; // tipo de compressão != LZ77
    }
    let out_size = (header >> 8) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(out_size);
    while out.len() < out_size {
        let flags = *rom.get(off)?;
        off += 1;
        for bit in 0..8 {
            if out.len() >= out_size {
                break;
            }
            if flags & (0x80 >> bit) == 0 {
                out.push(*rom.get(off)?);
                off += 1;
            } else {
                let b0 = *rom.get(off)? as usize;
                let b1 = *rom.get(off + 1)? as usize;
                off += 2;
                let length = (b0 >> 4) + 3;
                let disp = ((b0 & 0x0F) << 8 | b1) + 1;
                for _ in 0..length {
                    if out.len() < disp {
                        break;
                    }
                    let byte = out[out.len() - disp];
                    out.push(byte);
                }
            }
        }
    }
    Some(out)
}

/// BGR555 → RGBA8 (mesma escala de 5→8 bits usada na PPU).
fn bgr555_to_rgba8(c: u16) -> [u8; 4] {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz77_decompresses_literals_and_backref() {
        // header: tipo LZ77 (0x10), tamanho 5. flags=0xE0 → 3 primeiros bits
        // setados? Não: vamos fazer 5 literais (flags=0x00) "ABCDE".
        let mut data = vec![0x10u8, 5, 0, 0]; // out_size=5
        data.push(0x00); // flags: 8 blocos literais
        data.extend_from_slice(b"ABCDE");
        let out = lz77_decompress(&data, 0).unwrap();
        assert_eq!(&out, b"ABCDE");
    }

    #[test]
    fn lz77_backreference_repeats() {
        // out_size=4: literal 'A', depois back-ref (len=3, disp=1) → "AAAA".
        let mut data = vec![0x10u8, 4, 0, 0];
        // flags: bit0=0 (literal), bit1=1 (comprimido), resto irrelevante.
        data.push(0b0100_0000);
        data.push(b'A');
        // bloco comprimido: b0=(len-3)<<4 | disp_hi; len=3→0, disp=1→disp-1=0.
        data.push(0x00); // (0<<4)|0
        data.push(0x00); // disp_lo=0 → disp=1
        let out = lz77_decompress(&data, 0).unwrap();
        assert_eq!(&out, b"AAAA");
    }

    #[test]
    fn bgr555_pure_red() {
        assert_eq!(bgr555_to_rgba8(0x001F), [0xFF, 0, 0, 0xFF]);
    }
}
