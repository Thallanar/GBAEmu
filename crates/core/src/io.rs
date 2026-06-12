//! I/O registers do GBA (0x04000000 - 0x040003FE).
//!
//! Este módulo centraliza:
//!   - Registradores de interrupção (IE, IF, IME)
//!   - Timers 0-3 (TM0CNT..TM3CNT + reload)
//!   - (futuramente) PPU regs, DMA, sound, joypad
//!
//! Acessos de 8/16/32 bits vão por aqui; o bus apenas faz delegação para
//! [`Io::read_u8`] e companhia.

use crate::joypad::Joypad;
use crate::sio::Sio;
use crate::timer::Timers;

pub const IE_ADDR: u32 = 0x0400_0200;
pub const IF_ADDR: u32 = 0x0400_0202;
pub const IME_ADDR: u32 = 0x0400_0208;
pub const KEYINPUT_ADDR: u32 = 0x0400_0130;
pub const KEYCNT_ADDR: u32 = 0x0400_0132;

/// Flags de interrupção (uma a uma — IE/IF compartilham o mesmo layout).
#[allow(dead_code)]
pub mod irq_bits {
    pub const VBLANK: u16  = 1 << 0;
    pub const HBLANK: u16  = 1 << 1;
    pub const VCOUNT: u16  = 1 << 2;
    pub const TIMER0: u16  = 1 << 3;
    pub const TIMER1: u16  = 1 << 4;
    pub const TIMER2: u16  = 1 << 5;
    pub const TIMER3: u16  = 1 << 6;
    pub const SERIAL: u16  = 1 << 7;
    pub const DMA0: u16    = 1 << 8;
    pub const DMA1: u16    = 1 << 9;
    pub const DMA2: u16    = 1 << 10;
    pub const DMA3: u16    = 1 << 11;
    pub const KEYPAD: u16  = 1 << 12;
    pub const GAMEPAK: u16 = 1 << 13;
}

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Io {
    /// Interrupt Enable.
    pub ie: u16,
    /// Interrupt Flag (request). Write-1-to-clear semantics.
    pub iflag: u16,
    /// Interrupt Master Enable (apenas bit 0).
    pub ime: bool,
    pub timers: Timers,
    pub joypad: Joypad,
    pub sio: Sio,
}

impl Io {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        match addr {
            // Timers
            0x0400_0100..=0x0400_010F => self.timers.read_u8(addr),
            // Serial (SIOMULTI/SIOCNT/SIOMLT_SEND e RCNT/IR)
            0x0400_0120..=0x0400_012F | 0x0400_0134..=0x0400_0137 => self.sio.read_u8(addr),
            // Joypad (KEYINPUT read-only, KEYCNT r/w)
            KEYINPUT_ADDR => self.joypad.keyinput() as u8,
            a if a == KEYINPUT_ADDR + 1 => (self.joypad.keyinput() >> 8) as u8,
            KEYCNT_ADDR => self.joypad.keycnt as u8,
            a if a == KEYCNT_ADDR + 1 => (self.joypad.keycnt >> 8) as u8,
            // IRQ
            IE_ADDR     => self.ie as u8,
            a if a == IE_ADDR + 1 => (self.ie >> 8) as u8,
            IF_ADDR     => self.iflag as u8,
            a if a == IF_ADDR + 1 => (self.iflag >> 8) as u8,
            IME_ADDR    => self.ime as u8,
            _ => 0,
        }
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        match addr {
            0x0400_0100..=0x0400_010F => self.timers.write_u8(addr, val),
            // Serial: a escrita pode completar uma transferência "sem cabo"
            // (clock interno) — nesse caso o módulo pede a IRQ serial.
            0x0400_0120..=0x0400_012F | 0x0400_0134..=0x0400_0137 => {
                if self.sio.write_u8(addr, val) {
                    self.raise(irq_bits::SERIAL);
                }
            }
            // KEYINPUT é read-only; KEYCNT é gravável.
            KEYCNT_ADDR => self.joypad.keycnt = (self.joypad.keycnt & 0xFF00) | val as u16,
            a if a == KEYCNT_ADDR + 1 => {
                self.joypad.keycnt = (self.joypad.keycnt & 0x00FF) | ((val as u16) << 8)
            }
            IE_ADDR     => self.ie = (self.ie & 0xFF00) | val as u16,
            a if a == IE_ADDR + 1 => self.ie = (self.ie & 0x00FF) | ((val as u16) << 8),
            // IF é write-1-to-clear.
            IF_ADDR     => self.iflag &= !(val as u16),
            a if a == IF_ADDR + 1 => self.iflag &= !((val as u16) << 8),
            IME_ADDR    => self.ime = val & 1 != 0,
            _ => {}
        }
    }

    /// Há interrupção pendente? (respeita o master enable IME).
    pub fn irq_pending(&self) -> bool {
        self.ime && (self.ie & self.iflag) != 0
    }

    /// Condição para sair do Halt: IE & IF != 0, ignorando o IME.
    /// (O Halt acorda em qualquer IRQ habilitada, mesmo com IME=0.)
    pub fn halt_condition_met(&self) -> bool {
        (self.ie & self.iflag) != 0
    }

    /// Marca uma IRQ no IF (ela só dispara se IE & IME permitirem).
    pub fn raise(&mut self, flag: u16) {
        self.iflag |= flag;
    }
}
