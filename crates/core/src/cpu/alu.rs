//! ALU helpers: barrel shifter e flag computation para data processing.

/// Resultado de uma operação shift: valor + carry-out (alimenta o flag C
/// quando S=1 e a operação é lógica).
#[derive(Debug, Clone, Copy)]
pub struct ShiftOut {
    pub value: u32,
    pub carry: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ShiftKind {
    Lsl, // Logical Shift Left
    Lsr, // Logical Shift Right
    Asr, // Arithmetic Shift Right (preserva sinal)
    Ror, // Rotate Right
}

impl ShiftKind {
    pub fn from_bits(b: u32) -> Self {
        match b & 0b11 {
            0 => Self::Lsl,
            1 => Self::Lsr,
            2 => Self::Asr,
            _ => Self::Ror,
        }
    }
}

/// Aplica shift sobre `value` por `amount` posições.
///
/// `carry_in` é o valor atual do flag C — usado como fallback quando o shift
/// não consegue gerar um novo carry (ex.: LSL #0).
///
/// `imm_form`: distingue "shift por imediato" vs "shift por registrador".
/// Essa distinção MUDA a semântica de shift==0 nos casos LSR/ASR/ROR
/// (no encoding por imediato, #0 significa #32 ou RRX, conforme o tipo).
pub fn barrel_shift(
    kind: ShiftKind,
    value: u32,
    amount: u32,
    carry_in: bool,
    imm_form: bool,
) -> ShiftOut {
    match kind {
        ShiftKind::Lsl => lsl(value, amount, carry_in),
        ShiftKind::Lsr => lsr(value, amount, carry_in, imm_form),
        ShiftKind::Asr => asr(value, amount, carry_in, imm_form),
        ShiftKind::Ror => ror(value, amount, carry_in, imm_form),
    }
}

fn lsl(v: u32, amt: u32, carry_in: bool) -> ShiftOut {
    match amt {
        0 => ShiftOut { value: v, carry: carry_in },
        1..=31 => ShiftOut {
            value: v << amt,
            carry: (v >> (32 - amt)) & 1 != 0,
        },
        32 => ShiftOut { value: 0, carry: v & 1 != 0 },
        _ => ShiftOut { value: 0, carry: false },
    }
}

fn lsr(v: u32, amt: u32, carry_in: bool, imm_form: bool) -> ShiftOut {
    // Encoding imediato: amt==0 significa LSR #32.
    let effective = if amt == 0 && imm_form { 32 } else { amt };
    match effective {
        0 => ShiftOut { value: v, carry: carry_in }, // só ocorre em "register form"
        1..=31 => ShiftOut {
            value: v >> effective,
            carry: (v >> (effective - 1)) & 1 != 0,
        },
        32 => ShiftOut { value: 0, carry: v & 0x8000_0000 != 0 },
        _ => ShiftOut { value: 0, carry: false },
    }
}

fn asr(v: u32, amt: u32, carry_in: bool, imm_form: bool) -> ShiftOut {
    let effective = if amt == 0 && imm_form { 32 } else { amt };
    let sv = v as i32;
    match effective {
        0 => ShiftOut { value: v, carry: carry_in },
        1..=31 => ShiftOut {
            value: (sv >> effective) as u32,
            carry: (sv >> (effective - 1)) & 1 != 0,
        },
        _ => {
            // amt >= 32: resultado é todo-1 ou todo-0 conforme o sinal.
            let filled = if sv < 0 { 0xFFFF_FFFF } else { 0 };
            ShiftOut { value: filled, carry: sv < 0 }
        }
    }
}

fn ror(v: u32, amt: u32, carry_in: bool, imm_form: bool) -> ShiftOut {
    // Encoding imediato: amt==0 significa RRX (rotate right extended).
    if amt == 0 && imm_form {
        return rrx(v, carry_in);
    }
    if amt == 0 {
        return ShiftOut { value: v, carry: carry_in };
    }
    let amt = amt % 32;
    if amt == 0 {
        // ROR por múltiplo de 32 (em register form): valor inalterado,
        // carry = bit 31.
        return ShiftOut { value: v, carry: v & 0x8000_0000 != 0 };
    }
    ShiftOut {
        value: v.rotate_right(amt),
        carry: (v >> (amt - 1)) & 1 != 0,
    }
}

/// RRX: rotate right por 1, com C entrando no bit 31.
fn rrx(v: u32, carry_in: bool) -> ShiftOut {
    let carry_out = v & 1 != 0;
    let cin = if carry_in { 0x8000_0000 } else { 0 };
    ShiftOut { value: (v >> 1) | cin, carry: carry_out }
}

// ───────────────────── ALU arithmetic ─────────────────────

/// Resultado de uma soma com flags C e V.
#[derive(Debug, Clone, Copy)]
pub struct ArithOut {
    pub value: u32,
    pub carry: bool,
    pub overflow: bool,
}

pub fn add_with_flags(a: u32, b: u32) -> ArithOut {
    let (value, carry) = a.overflowing_add(b);
    // Overflow signed: ocorre quando os sinais de a e b são iguais e o resultado tem sinal oposto.
    let overflow = ((a ^ value) & (b ^ value) & 0x8000_0000) != 0;
    ArithOut { value, carry, overflow }
}

pub fn adc_with_flags(a: u32, b: u32, carry_in: bool) -> ArithOut {
    let cin = u32::from(carry_in);
    let (s1, c1) = a.overflowing_add(b);
    let (value, c2) = s1.overflowing_add(cin);
    let carry = c1 || c2;
    let overflow = ((a ^ value) & (b ^ value) & 0x8000_0000) != 0;
    ArithOut { value, carry, overflow }
}

/// Subtração ARM: a - b. Convenção do ARM: C = NOT borrow (ou seja,
/// C=1 quando NÃO houve borrow).
pub fn sub_with_flags(a: u32, b: u32) -> ArithOut {
    let (value, borrow) = a.overflowing_sub(b);
    let overflow = ((a ^ b) & (a ^ value) & 0x8000_0000) != 0;
    ArithOut { value, carry: !borrow, overflow }
}

/// SBC: a - b - !C. Equivalente a a + ~b + C.
pub fn sbc_with_flags(a: u32, b: u32, carry_in: bool) -> ArithOut {
    adc_with_flags(a, !b, carry_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsl_basic() {
        let r = barrel_shift(ShiftKind::Lsl, 0x0000_00FF, 4, false, true);
        assert_eq!(r.value, 0x0000_0FF0);
        assert!(!r.carry);
    }

    #[test]
    fn lsl_carry_out() {
        let r = barrel_shift(ShiftKind::Lsl, 0x8000_0000, 1, false, true);
        assert_eq!(r.value, 0);
        assert!(r.carry);
    }

    #[test]
    fn lsr_imm_zero_means_32() {
        let r = barrel_shift(ShiftKind::Lsr, 0x8000_0000, 0, false, true);
        assert_eq!(r.value, 0);
        assert!(r.carry);
    }

    #[test]
    fn asr_preserves_sign() {
        let r = barrel_shift(ShiftKind::Asr, 0x8000_0000, 1, false, true);
        assert_eq!(r.value, 0xC000_0000);
        assert!(!r.carry);
    }

    #[test]
    fn ror_basic() {
        let r = barrel_shift(ShiftKind::Ror, 0x0000_000F, 4, false, true);
        assert_eq!(r.value, 0xF000_0000);
        assert!(r.carry);
    }

    #[test]
    fn rrx_with_carry_in() {
        let r = barrel_shift(ShiftKind::Ror, 0x0000_0001, 0, true, true);
        assert_eq!(r.value, 0x8000_0000);
        assert!(r.carry);
    }

    #[test]
    fn add_overflow_signed() {
        let r = add_with_flags(0x7FFF_FFFF, 1);
        assert_eq!(r.value, 0x8000_0000);
        assert!(!r.carry);
        assert!(r.overflow);
    }

    #[test]
    fn sub_borrow_means_c_zero() {
        let r = sub_with_flags(1, 2);
        assert_eq!(r.value, 0xFFFF_FFFF);
        assert!(!r.carry); // borrow ocorreu → C=0
    }

    #[test]
    fn sub_no_borrow_c_one() {
        let r = sub_with_flags(5, 3);
        assert_eq!(r.value, 2);
        assert!(r.carry);
    }
}
