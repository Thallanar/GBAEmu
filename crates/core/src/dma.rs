//! DMA — Direct Memory Access (4 canais).
//!
//! Cada canal copia blocos de memória sem a CPU, em unidades de 16 ou 32 bits.
//! Registradores em 0x040000B0..0x040000DF (12 bytes por canal):
//!   SAD (origem), DAD (destino), CNT_L (contagem), CNT_H (controle).
//!
//! O disparo pode ser imediato, no VBlank, no HBlank ou "special" (FIFO de
//! som / video capture — ainda não implementados). A transferência em si vive
//! em [`crate::bus::Bus`] (precisa acessar a memória); aqui ficam só o estado
//! dos registradores e os helpers de decodificação do controle.
//!
//! Referência: GBATEK, "DMA Transfers".

/// Endereço-base do bloco de registradores de DMA.
pub const DMA_BASE: u32 = 0x0400_00B0;
/// Fim (exclusivo) do bloco de registradores de DMA.
pub const DMA_END: u32 = 0x0400_00E0;

/// Modo de disparo (bits 12-13 do controle).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Timing {
    Immediate,
    VBlank,
    HBlank,
    Special,
}

#[derive(Default, Clone, Copy)]
pub struct DmaChannel {
    pub sad: u32,     // registrador de origem
    pub dad: u32,     // registrador de destino
    pub count: u16,   // registrador de contagem (CNT_L)
    pub control: u16, // registrador de controle (CNT_H)

    // Valores internos "travados" no momento em que o canal é habilitado.
    pub int_src: u32,
    pub int_dst: u32,
    pub int_count: u32,
}

impl DmaChannel {
    pub fn enabled(&self) -> bool {
        self.control & (1 << 15) != 0
    }
    pub fn irq_on_end(&self) -> bool {
        self.control & (1 << 14) != 0
    }
    pub fn repeat(&self) -> bool {
        self.control & (1 << 9) != 0
    }
    /// 4 bytes (32 bits) ou 2 bytes (16 bits).
    pub fn unit_bytes(&self) -> u32 {
        if self.control & (1 << 10) != 0 {
            4
        } else {
            2
        }
    }
    pub fn timing(&self) -> Timing {
        match (self.control >> 12) & 0b11 {
            0 => Timing::Immediate,
            1 => Timing::VBlank,
            2 => Timing::HBlank,
            _ => Timing::Special,
        }
    }
    /// Controle de endereço de destino (bits 5-6): 0=inc, 1=dec, 2=fixo, 3=inc+reload.
    pub fn dst_control(&self) -> u8 {
        ((self.control >> 5) & 0b11) as u8
    }
    /// Controle de endereço de origem (bits 7-8): 0=inc, 1=dec, 2=fixo, 3=proibido.
    pub fn src_control(&self) -> u8 {
        ((self.control >> 7) & 0b11) as u8
    }
}

#[derive(Default)]
pub struct Dma {
    pub channels: [DmaChannel; 4],
}

impl Dma {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        let off = (addr - DMA_BASE) as usize;
        let ch = &self.channels[off / 12];
        match off % 12 {
            // SAD e DAD não são legíveis no hardware; devolvemos o registrador.
            0..=3 => (ch.sad >> ((off % 12) * 8)) as u8,
            4..=7 => (ch.dad >> (((off % 12) - 4) * 8)) as u8,
            8 => ch.count as u8,
            9 => (ch.count >> 8) as u8,
            10 => ch.control as u8,
            _ => (ch.control >> 8) as u8,
        }
    }

    /// Escreve um byte de registrador. Retorna `Some(canal)` quando o bit de
    /// enable transicionou 0→1 (momento de travar os valores internos).
    pub fn write_u8(&mut self, addr: u32, val: u8) -> Option<usize> {
        let off = (addr - DMA_BASE) as usize;
        let n = off / 12;
        let ch = &mut self.channels[n];
        let val = val as u32;
        match off % 12 {
            r @ 0..=3 => {
                let sh = r * 8;
                ch.sad = (ch.sad & !(0xFF << sh)) | (val << sh);
            }
            r @ 4..=7 => {
                let sh = (r - 4) * 8;
                ch.dad = (ch.dad & !(0xFF << sh)) | (val << sh);
            }
            8 => ch.count = (ch.count & 0xFF00) | val as u16,
            9 => ch.count = (ch.count & 0x00FF) | ((val as u16) << 8),
            10 => ch.control = (ch.control & 0xFF00) | val as u16,
            _ => {
                // Byte alto do controle: contém o bit de enable.
                let was_enabled = ch.enabled();
                ch.control = (ch.control & 0x00FF) | ((val as u16) << 8);
                if !was_enabled && ch.enabled() {
                    latch(ch, n);
                    return Some(n);
                }
            }
        }
        None
    }
}

/// Trava os valores internos (src/dst/count) a partir dos registradores,
/// aplicando as máscaras de endereço e contagem específicas do canal.
pub fn latch(ch: &mut DmaChannel, n: usize) {
    let (src_mask, dst_mask, count_mask) = channel_masks(n);
    ch.int_src = ch.sad & src_mask;
    ch.int_dst = ch.dad & dst_mask;
    let cnt = ch.count as u32 & count_mask;
    ch.int_count = if cnt == 0 { count_mask + 1 } else { cnt };
}

/// Recarrega só a contagem (e o destino, se controle de destino = inc+reload),
/// usado no modo repeat.
pub fn reload(ch: &mut DmaChannel, n: usize) {
    let (_, dst_mask, count_mask) = channel_masks(n);
    let cnt = ch.count as u32 & count_mask;
    ch.int_count = if cnt == 0 { count_mask + 1 } else { cnt };
    if ch.dst_control() == 3 {
        ch.int_dst = ch.dad & dst_mask;
    }
}

/// Máscaras (origem, destino, contagem) por canal.
/// DMA0 só acessa memória interna (origem 27-bit); DMA3 tem origem/destino de
/// 28 bits e contagem de 16 bits; os demais ficam no meio-termo.
fn channel_masks(n: usize) -> (u32, u32, u32) {
    match n {
        0 => (0x07FF_FFFF, 0x07FF_FFFF, 0x3FFF),
        1 | 2 => (0x0FFF_FFFF, 0x07FF_FFFF, 0x3FFF),
        _ => (0x0FFF_FFFF, 0x0FFF_FFFF, 0xFFFF),
    }
}
