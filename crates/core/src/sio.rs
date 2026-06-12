//! Serial I/O — o "link cable" (Fase Link, etapa a).
//!
//! Esta etapa implementa os registradores seriais com a semântica de **cabo
//! desconectado**: os jogos enxergam um console sem parceiro (em vez de lerem
//! zeros sem significado) e os fluxos de link — Cable Club do Pokémon, telas
//! de "aguardando parceiro" — se comportam como num GBA real sozinho. É a
//! fundação das próximas etapas (lockstep entre instâncias + transporte).
//!
//! Modelo (GBATEK):
//!   - RCNT (0x4000134): bit 15 = 0 → modo serial (SIOCNT decide); bit 15 = 1
//!     → general-purpose/JOY. Guardamos o que foi escrito; bits de entrada
//!     leem 0 (linhas soltas).
//!   - SIOCNT (0x4000128): bits 12-13 = modo (00/01 normal 8/32, 10 multi,
//!     11 UART), bit 7 = start/busy, bit 14 = IRQ enable.
//!   - SIOMULTI0-3 (0x4000120-126): dados recebidos por unidade (no modo
//!     normal de 32 bits, SIOMULTI0/1 = SIODATA32).
//!   - SIOMLT_SEND (0x400012A): o que enviamos (no modo normal de 8 bits, o
//!     mesmo endereço é SIODATA8, o shift register de envio/recepção).
//!
//! Comportamento sem parceiro:
//!   - **Multi-player**: SI lê 0 (achamos que somos o parent) e SD lê 0
//!     ("nem todos prontos") — é assim que os jogos detectam a falta de
//!     parceiros. Start com clock interno completa na hora: recebemos nosso
//!     próprio dado em SIOMULTI0 e 0xFFFF (linha alta) dos parceiros
//!     ausentes; busy limpa; IRQ serial se habilitada.
//!   - **Normal (8/32 bits) com clock interno**: completa na hora lendo a
//!     linha alta (0xFF/0xFFFFFFFF); busy limpa; IRQ se habilitada.
//!   - **Normal com clock externo e UART**: ninguém do outro lado gera clock
//!     — a transferência **nunca completa** (busy fica preso), como no
//!     hardware sem cabo.

pub const SIOMULTI0_ADDR: u32 = 0x0400_0120;
pub const SIOCNT_ADDR: u32 = 0x0400_0128;
pub const SIOMLT_SEND_ADDR: u32 = 0x0400_012A;
pub const RCNT_ADDR: u32 = 0x0400_0134;

/// Bit start/busy do SIOCNT.
const START: u16 = 1 << 7;
/// Bit de IRQ enable do SIOCNT.
const IRQ_ENABLE: u16 = 1 << 14;

/// Modo serial efetivo (RCNT + SIOCNT).
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum Mode {
    Normal8,
    Normal32,
    Multi,
    Uart,
    /// RCNT bit 15 = 1 (general-purpose ou JOY bus): SIOCNT inerte.
    GeneralPurpose,
}

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Sio {
    pub rcnt: u16,
    pub siocnt: u16,
    /// SIOMULTI0-3; no modo normal de 32 bits, [0]/[1] são o SIODATA32.
    pub siomulti: [u16; 4],
    /// SIOMLT_SEND; no modo normal de 8 bits, é o SIODATA8 (envio/recepção).
    pub siomlt_send: u16,
}

impl Sio {
    fn mode(&self) -> Mode {
        if self.rcnt & (1 << 15) != 0 {
            return Mode::GeneralPurpose;
        }
        match (self.siocnt >> 12) & 0b11 {
            0b00 => Mode::Normal8,
            0b01 => Mode::Normal32,
            0b10 => Mode::Multi,
            _ => Mode::Uart,
        }
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        let v = self.read_u16(addr & !1);
        if addr & 1 == 0 {
            v as u8
        } else {
            (v >> 8) as u8
        }
    }

    fn read_u16(&self, addr: u32) -> u16 {
        match addr {
            SIOMULTI0_ADDR => self.siomulti[0],
            a if a == SIOMULTI0_ADDR + 2 => self.siomulti[1],
            a if a == SIOMULTI0_ADDR + 4 => self.siomulti[2],
            a if a == SIOMULTI0_ADDR + 6 => self.siomulti[3],
            SIOCNT_ADDR => {
                if self.mode() == Mode::Multi {
                    // SI (bit 2) = 0: nos vemos como parent; SD (bit 3) = 0:
                    // "nem todos prontos" — a deixa de "sem parceiro" dos jogos.
                    self.siocnt & !0b1100
                } else {
                    self.siocnt
                }
            }
            SIOMLT_SEND_ADDR => self.siomlt_send,
            RCNT_ADDR => self.rcnt,
            _ => 0,
        }
    }

    /// Escrita de 1 byte (STRB ou decomposição do bus em região não-SIO).
    /// É read-modify-write do halfword guardado — a primitiva real é
    /// [`Self::write_u16`], porque os registradores seriais são de 16 bits e
    /// uma escrita parcial não pode disparar transferência com o outro byte
    /// "velho" (era o bug da decomposição low-primeiro do bus).
    pub fn write_u8(&mut self, addr: u32, val: u8) -> bool {
        let base = addr & !1;
        let cur = self.stored_u16(base);
        let new = if addr & 1 == 0 {
            (cur & 0xFF00) | val as u16
        } else {
            (cur & 0x00FF) | ((val as u16) << 8)
        };
        self.write_u16(base, new)
    }

    /// Escrita atômica de 16 bits (STRH — o jeito documentado de programar o
    /// SIO). Devolve `true` se uma transferência completou e a IRQ serial
    /// deve ser levantada (o chamador cuida do IF; este módulo não conhece o
    /// resto do chip).
    pub fn write_u16(&mut self, addr: u32, val: u16) -> bool {
        match addr {
            SIOMULTI0_ADDR => self.siomulti[0] = val,
            a if a == SIOMULTI0_ADDR + 2 => self.siomulti[1] = val,
            a if a == SIOMULTI0_ADDR + 4 => self.siomulti[2] = val,
            a if a == SIOMULTI0_ADDR + 6 => self.siomulti[3] = val,
            SIOCNT_ADDR => {
                self.siocnt = val;
                return self.maybe_complete_transfer();
            }
            SIOMLT_SEND_ADDR => self.siomlt_send = val,
            RCNT_ADDR => self.rcnt = val,
            _ => {}
        }
        false
    }

    /// Valor guardado (sem a máscara de estado-de-linha da leitura) — pro
    /// read-modify-write do [`Self::write_u8`].
    fn stored_u16(&self, addr: u32) -> u16 {
        match addr {
            SIOMULTI0_ADDR => self.siomulti[0],
            a if a == SIOMULTI0_ADDR + 2 => self.siomulti[1],
            a if a == SIOMULTI0_ADDR + 4 => self.siomulti[2],
            a if a == SIOMULTI0_ADDR + 6 => self.siomulti[3],
            SIOCNT_ADDR => self.siocnt,
            SIOMLT_SEND_ADDR => self.siomlt_send,
            RCNT_ADDR => self.rcnt,
            _ => 0,
        }
    }

    /// Sem parceiro: completa a transferência iniciada (se o modo permite) e
    /// diz se a IRQ serial deve subir.
    fn maybe_complete_transfer(&mut self) -> bool {
        if self.siocnt & START == 0 {
            return false;
        }
        match self.mode() {
            Mode::Multi => {
                // Clock vem sempre do parent (nós): completa na hora. Nosso
                // dado ecoa em SIOMULTI0; parceiros ausentes leem linha alta.
                self.siomulti = [self.siomlt_send, 0xFFFF, 0xFFFF, 0xFFFF];
            }
            // Normal: bit 0 = fonte do clock (1 = interno). Com clock externo
            // (0) e sem parceiro, nada gera clock — busy fica preso.
            Mode::Normal8 if self.siocnt & 1 != 0 => {
                self.siomlt_send = (self.siomlt_send & 0xFF00) | 0x00FF;
            }
            Mode::Normal32 if self.siocnt & 1 != 0 => {
                self.siomulti[0] = 0xFFFF;
                self.siomulti[1] = 0xFFFF;
            }
            _ => return false, // externo/UART/general-purpose: sem clock, sem fim
        }
        self.siocnt &= !START;
        self.siocnt & IRQ_ENABLE != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escrita de 16 bits (STRH), como o bus entrega pro SIO.
    fn w16(sio: &mut Sio, addr: u32, v: u16) -> bool {
        sio.write_u16(addr, v)
    }

    fn r16(sio: &Sio, addr: u32) -> u16 {
        sio.read_u8(addr) as u16 | ((sio.read_u8(addr + 1) as u16) << 8)
    }

    #[test]
    fn registradores_guardam_o_que_foi_escrito() {
        let mut sio = Sio::default();
        w16(&mut sio, RCNT_ADDR, 0x8000);
        w16(&mut sio, SIOMLT_SEND_ADDR, 0x1234);
        assert_eq!(r16(&sio, RCNT_ADDR), 0x8000);
        assert_eq!(r16(&sio, SIOMLT_SEND_ADDR), 0x1234);
    }

    #[test]
    fn multi_sem_parceiro_completa_com_linha_alta_e_irq() {
        let mut sio = Sio::default();
        w16(&mut sio, SIOMLT_SEND_ADDR, 0xBEEF);
        // Multi (bits 12-13 = 10), IRQ enable (14), start (7).
        let irq = w16(&mut sio, SIOCNT_ADDR, 0b10 << 12 | IRQ_ENABLE | START);
        assert!(irq, "transferência sem parceiro deve levantar a IRQ serial");
        // Busy limpou; SI/SD leem 0 (parent, "nem todos prontos").
        let cnt = r16(&sio, SIOCNT_ADDR);
        assert_eq!(cnt & START, 0, "busy deve limpar");
        assert_eq!(cnt & 0b1100, 0, "SI/SD devem ler 0 sem cabo");
        // Nosso dado ecoa; parceiros ausentes = 0xFFFF.
        assert_eq!(r16(&sio, SIOMULTI0_ADDR), 0xBEEF);
        assert_eq!(r16(&sio, SIOMULTI0_ADDR + 2), 0xFFFF);
        assert_eq!(r16(&sio, SIOMULTI0_ADDR + 4), 0xFFFF);
        assert_eq!(r16(&sio, SIOMULTI0_ADDR + 6), 0xFFFF);
    }

    #[test]
    fn normal32_clock_interno_recebe_linha_alta() {
        let mut sio = Sio::default();
        w16(&mut sio, SIOMULTI0_ADDR, 0x1111);
        w16(&mut sio, SIOMULTI0_ADDR + 2, 0x2222);
        // Normal 32 (bits 12-13 = 01), clock interno (bit 0), start. Sem IRQ.
        let irq = w16(&mut sio, SIOCNT_ADDR, 0b01 << 12 | 1 | START);
        assert!(!irq, "IRQ desabilitada não deve subir");
        assert_eq!(r16(&sio, SIOCNT_ADDR) & START, 0, "busy deve limpar");
        assert_eq!(r16(&sio, SIOMULTI0_ADDR), 0xFFFF);
        assert_eq!(r16(&sio, SIOMULTI0_ADDR + 2), 0xFFFF);
    }

    #[test]
    fn normal_clock_externo_fica_busy_para_sempre() {
        let mut sio = Sio::default();
        // Normal 8, clock externo (bit 0 = 0), start.
        let irq = w16(&mut sio, SIOCNT_ADDR, START);
        assert!(!irq);
        assert_ne!(r16(&sio, SIOCNT_ADDR) & START, 0, "sem cabo, externo nunca completa");
    }

    #[test]
    fn escrita_de_byte_preserva_o_outro_byte_e_dispara_com_modo_atual() {
        let mut sio = Sio::default();
        // Configura multi + IRQ enable (sem start) com escrita de 16 bits.
        w16(&mut sio, SIOCNT_ADDR, 0b10 << 12 | IRQ_ENABLE);
        // STRB no byte baixo ligando só o start: o byte alto (modo) persiste
        // e a transferência completa no modo multi já configurado.
        let irq = sio.write_u8(SIOCNT_ADDR, START as u8);
        assert!(irq, "start por escrita de byte deve usar o modo já configurado");
        assert_eq!(r16(&sio, SIOMULTI0_ADDR + 2), 0xFFFF);
    }

    #[test]
    fn general_purpose_inerte() {
        let mut sio = Sio::default();
        w16(&mut sio, RCNT_ADDR, 0x8000); // general-purpose
        let irq = w16(&mut sio, SIOCNT_ADDR, 0b10 << 12 | IRQ_ENABLE | START);
        assert!(!irq, "em general-purpose o SIOCNT não dispara nada");
        assert_ne!(r16(&sio, SIOCNT_ADDR) & START, 0);
    }
}
