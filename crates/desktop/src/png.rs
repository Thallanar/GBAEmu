//! Codificador PNG mínimo (RGBA 8-bit, **sem compressão**) para salvar
//! screenshots sem puxar uma dependência só pra isso.
//!
//! O formato PNG embrulha os pixels num fluxo zlib. Em vez de implementar o
//! algoritmo DEFLATE de verdade, usamos blocos "stored" (tipo 00) — DEFLATE
//! permite blocos literais não comprimidos, então o "zlib" aqui é só os bytes
//! crus com um cabeçalho. O arquivo fica grande (tela do GBA = 240×160, ~150 KB),
//! mas é trivialmente correto e dependência-zero. Implementamos CRC-32 (PNG) e
//! Adler-32 (zlib) à mão.

/// CRC-32 (polinômio 0xEDB88320), usado no fim de cada chunk PNG.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            // máscara = 0xFFFFFFFF se o bit baixo é 1, senão 0.
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Adler-32, a checksum do fluxo zlib (vai depois dos blocos DEFLATE).
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

/// Escreve um chunk PNG: tamanho (big-endian), tipo, dados e o CRC do
/// (tipo + dados).
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Codifica `width*height` pixels RGBA8 (`rgba.len() == width*height*4`) num PNG.
pub fn encode_rgba(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    debug_assert_eq!(rgba.len(), width * height * 4);

    // Dados crus do PNG: cada linha começa com um byte de filtro (0 = None).
    let mut raw = Vec::with_capacity(height * (1 + width * 4));
    for y in 0..height {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * width * 4..(y + 1) * width * 4]);
    }

    // Fluxo zlib: cabeçalho (0x78 0x01) + blocos DEFLATE "stored" + Adler-32.
    let mut zlib = vec![0x78, 0x01];
    let mut i = 0;
    loop {
        let chunk = (raw.len() - i).min(0xFFFF);
        let is_final = i + chunk >= raw.len();
        zlib.push(if is_final { 1 } else { 0 }); // BFINAL + BTYPE=00 (stored)
        zlib.extend_from_slice(&(chunk as u16).to_le_bytes()); // LEN
        zlib.extend_from_slice(&(!(chunk as u16)).to_le_bytes()); // ~LEN
        zlib.extend_from_slice(&raw[i..i + chunk]);
        i += chunk;
        if is_final {
            break;
        }
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    // bit depth 8, color type 6 (RGBA), compressão 0, filtro 0, sem entrelace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        // CRC-32 de "123456789" = 0xCBF43926 (vetor de teste padrão).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler32_known_vector() {
        // Adler-32 de "Wikipedia" = 0x11E60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn encodes_valid_png_header_and_size() {
        let px = vec![0u8; 2 * 2 * 4];
        let png = encode_rgba(2, 2, &px);
        // Assinatura PNG.
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // Primeiro chunk é IHDR com 13 bytes de dados.
        assert_eq!(&png[8..12], &13u32.to_be_bytes());
        assert_eq!(&png[12..16], b"IHDR");
        // Largura/altura no IHDR.
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[20..24], &2u32.to_be_bytes());
        // Termina com um chunk IEND.
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }
}
