//! CPSR — Current Program Status Register (32 bits).
//!
//! Layout (bits altos → baixos):
//!   31 30 29 28  27 .. 8   7 6 5  4 3 2 1 0
//!   N  Z  C  V   (reserved) I F T  M[4..0]
//!
//! - **N**: Negative   (result < 0)
//! - **Z**: Zero
//! - **C**: Carry
//! - **V**: Overflow (signed)
//! - **I**: IRQ disable
//! - **F**: FIQ disable
//! - **T**: Thumb state (1 = THUMB, 0 = ARM)
//! - **M**: Modo do processador (5 bits)

use bitflags::bitflags;

bitflags! {
    /// Flags do CPSR. Operações aritméticas atualizam N/Z/C/V.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PsrFlags: u32 {
        const N = 1 << 31;
        const Z = 1 << 30;
        const C = 1 << 29;
        const V = 1 << 28;
        const I = 1 << 7;
        const F = 1 << 6;
        const T = 1 << 5;
    }
}

/// Modos de operação do ARM7TDMI (campo M[4:0] do CPSR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub enum CpuMode {
    User = 0b10000,
    Fiq = 0b10001,
    Irq = 0b10010,
    Supervisor = 0b10011,
    Abort = 0b10111,
    Undefined = 0b11011,
    System = 0b11111,
}

impl CpuMode {
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits & 0x1F {
            0b10000 => Some(Self::User),
            0b10001 => Some(Self::Fiq),
            0b10010 => Some(Self::Irq),
            0b10011 => Some(Self::Supervisor),
            0b10111 => Some(Self::Abort),
            0b11011 => Some(Self::Undefined),
            0b11111 => Some(Self::System),
            _ => None,
        }
    }

    /// Índice no banco de SPSR (User/System não tem SPSR próprio).
    pub fn spsr_index(self) -> Option<usize> {
        match self {
            Self::Fiq => Some(0),
            Self::Irq => Some(1),
            Self::Supervisor => Some(2),
            Self::Abort => Some(3),
            Self::Undefined => Some(4),
            Self::User | Self::System => None,
        }
    }
}

/// Wrapper tipado em volta do CPSR. Útil para evitar manipular bits "soltos".
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Cpsr(pub u32);

impl Cpsr {
    pub const fn new() -> Self {
        // Default: System mode, IRQ/FIQ desabilitados, ARM state.
        Self(CpuMode::System as u32 | PsrFlags::I.bits() | PsrFlags::F.bits())
    }

    #[inline]
    pub fn n(self) -> bool {
        self.0 & PsrFlags::N.bits() != 0
    }
    #[inline]
    pub fn z(self) -> bool {
        self.0 & PsrFlags::Z.bits() != 0
    }
    #[inline]
    pub fn c(self) -> bool {
        self.0 & PsrFlags::C.bits() != 0
    }
    #[inline]
    pub fn v(self) -> bool {
        self.0 & PsrFlags::V.bits() != 0
    }
    #[inline]
    pub fn thumb(self) -> bool {
        self.0 & PsrFlags::T.bits() != 0
    }
    #[inline]
    pub fn irq_disabled(self) -> bool {
        self.0 & PsrFlags::I.bits() != 0
    }

    pub fn set_flag(&mut self, f: PsrFlags, value: bool) {
        if value {
            self.0 |= f.bits();
        } else {
            self.0 &= !f.bits();
        }
    }

    pub fn set_nz(&mut self, result: u32) {
        self.set_flag(PsrFlags::N, (result as i32) < 0);
        self.set_flag(PsrFlags::Z, result == 0);
    }

    pub fn mode(self) -> CpuMode {
        // Em CPU inicializada corretamente, sempre cai num modo válido.
        CpuMode::from_bits(self.0).unwrap_or(CpuMode::System)
    }

    pub fn set_mode(&mut self, mode: CpuMode) {
        self.0 = (self.0 & !0x1F) | (mode as u32);
    }
}

impl Default for Cpsr {
    fn default() -> Self {
        Self::new()
    }
}
