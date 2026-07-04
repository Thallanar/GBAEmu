//! Filtros de upscale de pixel-art (família **HQx**), como alternativa aos
//! shaders de pós-processo.
//!
//! Recebe o framebuffer do GBA (RGBA8, bytes `[R, G, B, A]`) e devolve uma
//! imagem ampliada por um fator inteiro (2×/3×/4×), pronta pra subir numa
//! textura. O algoritmo vem da crate [`hqx`] (LGPL-2.1-or-later); aqui só
//! cuidamos do empacotamento de pixel: a `hqx` opera em `u32 = 0xAARRGGBB`,
//! enquanto o framebuffer é `[R, G, B, A]` em memória (que em little-endian
//! seria `0xAABBGGRR` — daí a necessidade de reempacotar, sem `transmute`).
//!
//! O [`Scaler`] guarda buffers reutilizáveis pra não realocar a cada frame.

#![forbid(unsafe_code)]

/// Filtro de upscale selecionado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Upscale {
    #[default]
    Off,
    Hq2x,
    Hq3x,
    Hq4x,
}

impl Upscale {
    /// Todos os filtros, na ordem de exibição (o primeiro é o "sem filtro").
    pub const ALL: [Upscale; 4] = [Upscale::Off, Upscale::Hq2x, Upscale::Hq3x, Upscale::Hq4x];

    /// Fator de ampliação inteiro (1 = sem filtro).
    pub fn factor(self) -> usize {
        match self {
            Upscale::Off => 1,
            Upscale::Hq2x => 2,
            Upscale::Hq3x => 3,
            Upscale::Hq4x => 4,
        }
    }

    /// Rótulo amigável (UI).
    pub fn label(self) -> &'static str {
        match self {
            Upscale::Off => "Nenhum",
            Upscale::Hq2x => "HQ2x",
            Upscale::Hq3x => "HQ3x",
            Upscale::Hq4x => "HQ4x",
        }
    }

    /// Chave estável para persistência.
    pub fn key(self) -> &'static str {
        match self {
            Upscale::Off => "off",
            Upscale::Hq2x => "hq2x",
            Upscale::Hq3x => "hq3x",
            Upscale::Hq4x => "hq4x",
        }
    }

    pub fn from_key(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|u| u.key() == s)
            .unwrap_or(Upscale::Off)
    }

    /// Deriva do fator inteiro (2/3/4); qualquer outro valor cai em `Off`. Útil
    /// para o contrato JNI, que passa o fator como `int`.
    pub fn from_factor(f: i32) -> Self {
        match f {
            2 => Upscale::Hq2x,
            3 => Upscale::Hq3x,
            4 => Upscale::Hq4x,
            _ => Upscale::Off,
        }
    }
}

/// Amplia framebuffers reaproveitando os buffers intermediários entre frames.
#[derive(Default)]
pub struct Scaler {
    /// Fonte empacotada em `0xAARRGGBB`.
    src_u32: Vec<u32>,
    /// Destino da `hqx` em `0xAARRGGBB`.
    dst_u32: Vec<u32>,
}

impl Scaler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Amplia `src` (RGBA8 `w`×`h`, bytes `[R, G, B, A]`) por `filter`, escrevendo
    /// o resultado RGBA8 em `out` e devolvendo as dimensões de saída `(ow, oh)`.
    /// Com [`Upscale::Off`] copia `src` sem alterar.
    pub fn scale(
        &mut self,
        filter: Upscale,
        src: &[u8],
        w: usize,
        h: usize,
        out: &mut Vec<u8>,
    ) -> (usize, usize) {
        if filter == Upscale::Off {
            out.clear();
            out.extend_from_slice(src);
            return (w, h);
        }

        let f = filter.factor();
        let (ow, oh) = (w * f, h * f);
        let n = w * h;

        // [R,G,B,A] em memória → u32 0xAARRGGBB que a hqx espera.
        self.src_u32.clear();
        self.src_u32.reserve(n);
        for px in src.chunks_exact(4).take(n) {
            self.src_u32.push(
                (px[3] as u32) << 24 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | (px[2] as u32),
            );
        }

        self.dst_u32.clear();
        self.dst_u32.resize(ow * oh, 0);
        match filter {
            Upscale::Hq2x => hqx::hq2x(&self.src_u32, &mut self.dst_u32, w as u32, h as u32),
            Upscale::Hq3x => hqx::hq3x(&self.src_u32, &mut self.dst_u32, w as u32, h as u32),
            Upscale::Hq4x => hqx::hq4x(&self.src_u32, &mut self.dst_u32, w as u32, h as u32),
            Upscale::Off => unreachable!(),
        }

        // 0xAARRGGBB → [R,G,B,A] de volta.
        out.clear();
        out.reserve(ow * oh * 4);
        for &px in &self.dst_u32 {
            out.extend_from_slice(&[
                (px >> 16) as u8,
                (px >> 8) as u8,
                px as u8,
                (px >> 24) as u8,
            ]);
        }
        (ow, oh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_copia_intacto() {
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut out = Vec::new();
        let (w, h) = Scaler::new().scale(Upscale::Off, &src, 2, 1, &mut out);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, src);
    }

    #[test]
    fn hq2x_dobra_dimensoes_e_preserva_cor_solida() {
        // Imagem 2×2 toda vermelha (R,G,B,A) → 4×4 toda vermelha (HQx num campo
        // uniforme não inventa cor: valida o empacotamento de canais).
        let red = [0xFF, 0x00, 0x00, 0xFF];
        let src: Vec<u8> = red.iter().copied().cycle().take(2 * 2 * 4).collect();
        let mut out = Vec::new();
        let (w, h) = Scaler::new().scale(Upscale::Hq2x, &src, 2, 2, &mut out);
        assert_eq!((w, h), (4, 4));
        assert_eq!(out.len(), 4 * 4 * 4);
        for px in out.chunks_exact(4) {
            assert_eq!(
                px, red,
                "cor deve sobreviver ao round-trip de empacotamento"
            );
        }
    }

    #[test]
    fn fatores_e_chaves() {
        assert_eq!(Upscale::Hq4x.factor(), 4);
        assert_eq!(Upscale::from_key("hq3x"), Upscale::Hq3x);
        assert_eq!(Upscale::from_key("xxx"), Upscale::Off);
        assert_eq!(Upscale::from_factor(2), Upscale::Hq2x);
        assert_eq!(Upscale::from_factor(9), Upscale::Off);
    }
}
