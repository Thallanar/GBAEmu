//! Avaliação dos 16 códigos de condição do ARM (4 bits altos da instrução).

use super::psr::Cpsr;

/// Códigos de condição (cond[3:0] em uma instrução ARM).
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum Condition {
    Eq = 0x0, // Z=1                     equal
    Ne = 0x1, // Z=0                     not equal
    Cs = 0x2, // C=1                     unsigned higher or same
    Cc = 0x3, // C=0                     unsigned lower
    Mi = 0x4, // N=1                     negative
    Pl = 0x5, // N=0                     positive or zero
    Vs = 0x6, // V=1                     overflow
    Vc = 0x7, // V=0                     no overflow
    Hi = 0x8, // C=1 & Z=0               unsigned higher
    Ls = 0x9, // C=0 | Z=1               unsigned lower or same
    Ge = 0xA, // N==V                    signed >=
    Lt = 0xB, // N!=V                    signed <
    Gt = 0xC, // Z=0 & N==V              signed >
    Le = 0xD, // Z=1 | N!=V              signed <=
    Al = 0xE, // always
    Nv = 0xF, // never (UNPREDICTABLE em ARMv5+; em ARMv4, instruções específicas)
}

impl Condition {
    pub fn from_bits(b: u32) -> Self {
        // Safety: máscara de 4 bits cobre todos os 16 variants.
        unsafe { std::mem::transmute((b & 0xF) as u8) }
    }

    pub fn evaluate(self, cpsr: Cpsr) -> bool {
        let (n, z, c, v) = (cpsr.n(), cpsr.z(), cpsr.c(), cpsr.v());
        match self {
            Self::Eq => z,
            Self::Ne => !z,
            Self::Cs => c,
            Self::Cc => !c,
            Self::Mi => n,
            Self::Pl => !n,
            Self::Vs => v,
            Self::Vc => !v,
            Self::Hi => c && !z,
            Self::Ls => !c || z,
            Self::Ge => n == v,
            Self::Lt => n != v,
            Self::Gt => !z && (n == v),
            Self::Le => z || (n != v),
            Self::Al => true,
            Self::Nv => false,
        }
    }
}
