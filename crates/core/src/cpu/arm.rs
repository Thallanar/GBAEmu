//! Decoder e executor de instruções ARM (32 bits).
//!
//! Implementado:
//!   - Branch (B / BL)
//!   - Data Processing (16 opcodes) com barrel shifter
//!   - PSR Transfer (MRS / MSR)
//!   - Multiply (MUL / MLA / UMULL / UMLAL / SMULL / SMLAL)
//!   - Single Data Transfer (LDR / STR, word e byte)
//!
//! Pendente: Halfword transfer, LDM/STM, SWI.

use crate::bus::Bus;

use super::alu::{
    adc_with_flags, add_with_flags, barrel_shift, sbc_with_flags, sub_with_flags, ShiftKind,
};
use super::condition::Condition;
use super::psr::{Cpsr, CpuMode, PsrFlags};
use super::{Cpu, Handler};

pub fn execute(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    execute_decoded(cpu, bus, instr, decode(instr));
}

/// Executa um handler já resolvido pelo [decode]. A condição (bits 28+) é a
/// única parte que depende do estado da CPU, então fica aqui, fora do cache.
#[inline]
pub(crate) fn execute_decoded(cpu: &mut Cpu, bus: &mut Bus, instr: u32, handler: Handler) {
    if Condition::from_bits(instr >> 28).evaluate(cpu.cpsr) {
        handler(cpu, bus, instr);
    }
}

/// Resolve a instrução até o handler-folha. É função **apenas dos bits** da
/// instrução (nunca do estado da CPU) — por isso o resultado é cacheável por
/// endereço de ROM (ver `DecodeCache` no `cpu/mod.rs`).
pub(crate) fn decode(instr: u32) -> Handler {
    match (instr >> 25) & 0b111 {
        0b101 => h_branch,
        0b000 | 0b001 => decode_group_000(instr),
        0b010 | 0b011 => exec_single_data_transfer,
        0b100 => exec_block_data_transfer,
        0b111 => {
            // SWI: bits 27..24 = 1111. Coprocessor (não usado no GBA) também cai aqui.
            if (instr >> 24) & 0xF == 0xF {
                exec_swi
            } else {
                h_coprocessor
            }
        }
        _ => h_unimpl,
    }
}

// Shims: dão a assinatura uniforme de [Handler] aos handlers que não usam o
// bus. O compilador os dissolve (chamada direta atrás do function pointer).
fn h_branch(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_branch(cpu, instr)
}
fn h_branch_exchange(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_branch_exchange(cpu, instr)
}
fn h_multiply(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_multiply(cpu, instr)
}
fn h_multiply_long(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_multiply_long(cpu, instr)
}
fn h_psr_transfer(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_psr_transfer(cpu, instr)
}
fn h_data_processing(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    exec_data_processing(cpu, instr)
}
fn h_coprocessor(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    let pc = cpu.regs.pc().wrapping_sub(8);
    log::warn!("ARM: coprocessor não suportado @ {:08X}", instr);
    cpu.stats.record_unimpl(pc, instr, false);
}
fn h_unimpl(cpu: &mut Cpu, _bus: &mut Bus, instr: u32) {
    let pc = cpu.regs.pc().wrapping_sub(8);
    log::warn!("ARM: opcode não implementado @ PC={:08X} instr={:08X}", pc, instr);
    cpu.stats.record_unimpl(pc, instr, false);
}

// ─────────────────────────── Branch ───────────────────────────

/// BX Rn — branch and exchange. Se bit 0 do destino = 1, entra em THUMB.
fn exec_branch_exchange(cpu: &mut Cpu, instr: u32) {
    let rn = (instr & 0xF) as usize;
    let target = cpu.regs.get(rn);
    let thumb = target & 1 != 0;
    cpu.cpsr.set_flag(PsrFlags::T, thumb);
    if thumb {
        cpu.set_pc_thumb(target & !1);
    } else {
        cpu.set_pc_arm(target & !3);
    }
}

fn exec_branch(cpu: &mut Cpu, instr: u32) {
    let link = (instr & (1 << 24)) != 0;
    let raw24 = instr & 0x00FF_FFFF;
    let signed = ((raw24 << 8) as i32) >> 8;
    let offset = signed.wrapping_mul(4);

    let pc = cpu.regs.pc();
    let target = pc.wrapping_add(offset as u32);

    if link {
        cpu.regs.set_lr(pc.wrapping_sub(4));
    }
    cpu.set_pc_arm(target);
}

// ──────────────────── Grupo 000/001 (mistura) ────────────────────
//
// Esse grupo contém Data Processing, PSR Transfer, Multiply e
// Halfword transfer. Resolvemos por bits[27:25]=000 e padrões em bits[7:4].

fn decode_group_000(instr: u32) -> Handler {
    let imm_operand = (instr & (1 << 25)) != 0;

    // BX Rn: cond | 0001_0010_1111_1111_1111_0001 | Rn
    // Padrão: bits[27:4] == 0x12FFF1
    if (instr & 0x0FFF_FFF0) == 0x012F_FF10 {
        return h_branch_exchange;
    }

    // Multiply: bits[27:22]=000000, bits[7:4]=1001 (não-imediato).
    if !imm_operand && (instr & 0x0F00_00F0) == 0x0000_0090 {
        let bit23 = (instr & (1 << 23)) != 0;
        return if !bit23 { h_multiply } else { h_multiply_long };
    }

    // Single Data Swap (SWP/SWPB): bits[27:23]=00010, bits[21:20]=00,
    // bits[11:8]=0000, bits[7:4]=1001 (bit 22 = B fica de fora da máscara).
    // Precisa vir ANTES do fallback de PSR/data-processing, pois o opcode
    // efetivo (1000, S=0) colide com o padrão de MRS/MSR.
    if (instr & 0x0FB0_0FF0) == 0x0100_0090 {
        return exec_swap;
    }

    // Halfword/signed-byte transfer: bit 7 e bit 4 ligados, com bits[6:5] != 00.
    // (bits[6:5]==00 com bit 7=1 já foi capturado pelo multiply acima.)
    if !imm_operand && (instr & 0x90) == 0x90 {
        let sh = (instr >> 5) & 0b11;
        if sh != 0 {
            return exec_halfword_transfer;
        }
    }

    let opcode = (instr >> 21) & 0xF;
    let set_flags = (instr & (1 << 20)) != 0;
    if (0x8..=0xB).contains(&opcode) && !set_flags {
        h_psr_transfer
    } else {
        h_data_processing
    }
}

// ────────────────────── Data Processing ──────────────────────

fn exec_data_processing(cpu: &mut Cpu, instr: u32) {
    let imm_operand = (instr & (1 << 25)) != 0;
    let opcode = (instr >> 21) & 0xF;
    let set_flags = (instr & (1 << 20)) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;

    let carry_in = cpu.cpsr.c();
    let (op2, shifter_carry) = compute_operand2(cpu, instr, imm_operand, carry_in);

    let mut a = cpu.regs.get(rn);
    // Rn=R15 com shift por registrador → lê PC+12 (+4 a mais do +8 normal).
    if rn == 15 && !imm_operand && (instr & (1 << 4)) != 0 {
        a = a.wrapping_add(4);
    }

    use OpResult::*;
    let result = match opcode {
        0x0 => Logical(a & op2),                          // AND
        0x1 => Logical(a ^ op2),                          // EOR
        0x2 => Arith(sub_with_flags(a, op2)),             // SUB
        0x3 => Arith(sub_with_flags(op2, a)),             // RSB
        0x4 => Arith(add_with_flags(a, op2)),             // ADD
        0x5 => Arith(adc_with_flags(a, op2, carry_in)),   // ADC
        0x6 => Arith(sbc_with_flags(a, op2, carry_in)),   // SBC
        0x7 => Arith(sbc_with_flags(op2, a, carry_in)),   // RSC
        0x8 => LogicalNoWrite(a & op2),                   // TST
        0x9 => LogicalNoWrite(a ^ op2),                   // TEQ
        0xA => ArithNoWrite(sub_with_flags(a, op2)),      // CMP
        0xB => ArithNoWrite(add_with_flags(a, op2)),      // CMN
        0xC => Logical(a | op2),                          // ORR
        0xD => Logical(op2),                              // MOV
        0xE => Logical(a & !op2),                         // BIC
        0xF => Logical(!op2),                             // MVN
        _ => unreachable!(),
    };

    // Valor a escrever em Rd (None nas variantes de comparação TST/TEQ/CMP/CMN).
    let write_value = match result {
        Logical(v) => {
            if set_flags {
                cpu.cpsr.set_nz(v);
                cpu.cpsr.set_flag(PsrFlags::C, shifter_carry);
            }
            Some(v)
        }
        LogicalNoWrite(v) => {
            if set_flags {
                cpu.cpsr.set_nz(v);
                cpu.cpsr.set_flag(PsrFlags::C, shifter_carry);
            }
            None
        }
        Arith(o) => {
            if set_flags {
                cpu.cpsr.set_nz(o.value);
                cpu.cpsr.set_flag(PsrFlags::C, o.carry);
                cpu.cpsr.set_flag(PsrFlags::V, o.overflow);
            }
            Some(o.value)
        }
        ArithNoWrite(o) => {
            if set_flags {
                cpu.cpsr.set_nz(o.value);
                cpu.cpsr.set_flag(PsrFlags::C, o.carry);
                cpu.cpsr.set_flag(PsrFlags::V, o.overflow);
            }
            None
        }
    };

    // Caso especial: Rd=R15 com S=1 → retorno de exceção. Restauramos o CPSR
    // do SPSR ANTES de fixar o PC, para alinhar de acordo com o bit T restaurado
    // (THUMB alinha em 2; ARM em 4). É assim que `SUBS PC, LR, #4` retorna.
    if rd == 15 && set_flags {
        if let Some(idx) = cpu.cpsr.mode().spsr_index() {
            cpu.cpsr = cpu.spsr[idx];
            cpu.regs.switch_mode(cpu.cpsr.mode());
        }
        if let Some(v) = write_value {
            if cpu.cpsr.thumb() {
                cpu.set_pc_thumb(v);
            } else {
                cpu.set_pc_arm(v);
            }
        }
    } else if let Some(v) = write_value {
        write_rd(cpu, rd, v);
    }
}

enum OpResult {
    Logical(u32),
    LogicalNoWrite(u32),
    Arith(super::alu::ArithOut),
    ArithNoWrite(super::alu::ArithOut),
}

fn write_rd(cpu: &mut Cpu, rd: usize, value: u32) {
    if rd == 15 {
        cpu.set_pc_arm(value);
    } else {
        cpu.regs.set(rd, value);
    }
}

fn compute_operand2(cpu: &Cpu, instr: u32, imm: bool, carry_in: bool) -> (u32, bool) {
    if imm {
        let rotate = ((instr >> 8) & 0xF) * 2;
        let value = (instr & 0xFF).rotate_right(rotate);
        let carry = if rotate == 0 {
            carry_in
        } else {
            value & 0x8000_0000 != 0
        };
        (value, carry)
    } else {
        let rm = (instr & 0xF) as usize;
        let mut rm_val = cpu.regs.get(rm);
        let shift_kind = ShiftKind::from_bits((instr >> 5) & 0b11);
        let by_register = (instr & (1 << 4)) != 0;

        let amount = if by_register {
            let rs = ((instr >> 8) & 0xF) as usize;
            if rm == 15 {
                rm_val = rm_val.wrapping_add(4);
            }
            cpu.regs.get(rs) & 0xFF
        } else {
            (instr >> 7) & 0x1F
        };

        let out = barrel_shift(shift_kind, rm_val, amount, carry_in, !by_register);
        (out.value, out.carry)
    }
}

// ────────────────────── PSR transfer ──────────────────────

fn exec_psr_transfer(cpu: &mut Cpu, instr: u32) {
    let use_spsr = (instr & (1 << 22)) != 0;
    let is_msr = (instr & (1 << 21)) != 0;

    if !is_msr {
        let rd = ((instr >> 12) & 0xF) as usize;
        let value = if use_spsr {
            cpu.current_spsr().map(|p| p.0).unwrap_or(cpu.cpsr.0)
        } else {
            cpu.cpsr.0
        };
        cpu.regs.set(rd, value);
    } else {
        let imm_operand = (instr & (1 << 25)) != 0;
        let operand = if imm_operand {
            let rotate = ((instr >> 8) & 0xF) * 2;
            (instr & 0xFF).rotate_right(rotate)
        } else {
            cpu.regs.get((instr & 0xF) as usize)
        };

        let mut mask: u32 = 0;
        if instr & (1 << 19) != 0 { mask |= 0xFF00_0000; }
        if instr & (1 << 18) != 0 { mask |= 0x00FF_0000; }
        if instr & (1 << 17) != 0 { mask |= 0x0000_FF00; }
        if instr & (1 << 16) != 0 { mask |= 0x0000_00FF; }

        let in_user = cpu.cpsr.mode() == CpuMode::User;
        let effective_mask = if !use_spsr && in_user {
            mask & 0xFF00_0000
        } else {
            mask
        };

        if use_spsr {
            if let Some(idx) = cpu.cpsr.mode().spsr_index() {
                let cur = cpu.spsr[idx].0;
                cpu.spsr[idx] = Cpsr((cur & !effective_mask) | (operand & effective_mask));
            }
        } else {
            let new_val = (cpu.cpsr.0 & !effective_mask) | (operand & effective_mask);
            let old_mode = cpu.cpsr.mode();
            cpu.cpsr = Cpsr(new_val);
            let new_mode = cpu.cpsr.mode();
            if new_mode != old_mode {
                cpu.regs.switch_mode(new_mode);
            }
        }
    }
}

// ────────────────────── Multiply ──────────────────────

/// MUL / MLA — 32-bit multiply (e multiply-accumulate).
/// Formato: cond | 000000 A S | Rd | Rn | Rs | 1001 | Rm
fn exec_multiply(cpu: &mut Cpu, instr: u32) {
    let accumulate = (instr & (1 << 21)) != 0;
    let set_flags = (instr & (1 << 20)) != 0;
    let rd = ((instr >> 16) & 0xF) as usize;
    let rn = ((instr >> 12) & 0xF) as usize;
    let rs = ((instr >> 8) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let mut result = cpu.regs.get(rm).wrapping_mul(cpu.regs.get(rs));
    if accumulate {
        result = result.wrapping_add(cpu.regs.get(rn));
    }
    cpu.regs.set(rd, result);

    if set_flags {
        cpu.cpsr.set_nz(result);
        // C é UNPREDICTABLE em ARMv4; deixamos como está.
    }
}

/// UMULL / UMLAL / SMULL / SMLAL — 64-bit multiply.
/// Formato: cond | 00001 U A S | RdHi | RdLo | Rs | 1001 | Rm
/// U=0 unsigned, U=1 signed; A=accumulate.
fn exec_multiply_long(cpu: &mut Cpu, instr: u32) {
    let signed = (instr & (1 << 22)) != 0;
    let accumulate = (instr & (1 << 21)) != 0;
    let set_flags = (instr & (1 << 20)) != 0;
    let rd_hi = ((instr >> 16) & 0xF) as usize;
    let rd_lo = ((instr >> 12) & 0xF) as usize;
    let rs = ((instr >> 8) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let a = cpu.regs.get(rm);
    let b = cpu.regs.get(rs);

    let mut product: u64 = if signed {
        ((a as i32 as i64).wrapping_mul(b as i32 as i64)) as u64
    } else {
        (a as u64).wrapping_mul(b as u64)
    };

    if accumulate {
        let acc = ((cpu.regs.get(rd_hi) as u64) << 32) | (cpu.regs.get(rd_lo) as u64);
        product = product.wrapping_add(acc);
    }

    cpu.regs.set(rd_lo, product as u32);
    cpu.regs.set(rd_hi, (product >> 32) as u32);

    if set_flags {
        cpu.cpsr.set_flag(PsrFlags::N, product & 0x8000_0000_0000_0000 != 0);
        cpu.cpsr.set_flag(PsrFlags::Z, product == 0);
    }
}

// ────────────────────── Single Data Transfer (LDR/STR) ──────────────────────

/// Formato: cond | 01 I P U B W L | Rn | Rd | offset(12)
///   I: 0=imediato, 1=registrador (com shift)
///   P: 0=post-indexed, 1=pre-indexed
///   U: 0=offset subtraído, 1=somado
///   B: 0=word, 1=byte
///   W: writeback (em pre-index) ou "T" (user-mode em post-index)
///   L: 0=store, 1=load
fn exec_single_data_transfer(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    let imm = (instr & (1 << 25)) == 0; // bit 25: 0=imediato, 1=registrador (invertido vs. data-proc)
    let pre = (instr & (1 << 24)) != 0;
    let up = (instr & (1 << 23)) != 0;
    let byte = (instr & (1 << 22)) != 0;
    let writeback = (instr & (1 << 21)) != 0;
    let load = (instr & (1 << 20)) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;

    let offset = if imm {
        instr & 0xFFF
    } else {
        // Offset = Rm com shift por imediato (sem shift-by-register aqui).
        let rm = (instr & 0xF) as usize;
        let kind = ShiftKind::from_bits((instr >> 5) & 0b11);
        let amount = (instr >> 7) & 0x1F;
        let v = cpu.regs.get(rm);
        barrel_shift(kind, v, amount, cpu.cpsr.c(), true).value
    };

    let base = cpu.regs.get(rn);
    let signed_offset = if up { offset } else { 0u32.wrapping_sub(offset) };
    let addr = if pre {
        base.wrapping_add(signed_offset)
    } else {
        base
    };

    if load {
        let value = if byte {
            bus.read_u8(addr) as u32
        } else {
            // LDR com endereço desalinhado faz ROR para alinhar (quirk do ARMv4).
            let aligned = addr & !0x3;
            let v = bus.read_u32(aligned);
            let rot = (addr & 0x3) * 8;
            v.rotate_right(rot)
        };
        write_rd(cpu, rd, value);
    } else {
        let mut value = cpu.regs.get(rd);
        // STR de R15 escreve PC+12 (já temos PC=exec_pc+8, então +4).
        if rd == 15 {
            value = value.wrapping_add(4);
        }
        if byte {
            bus.write_u8(addr, value as u8);
        } else {
            bus.write_u32(addr & !0x3, value);
        }
    }

    // Writeback / post-index.
    let final_addr = base.wrapping_add(signed_offset);
    let do_writeback = !pre || writeback;
    if do_writeback && !(load && rd == rn) {
        if rn == 15 {
            cpu.set_pc_arm(final_addr);
        } else {
            cpu.regs.set(rn, final_addr);
        }
    }
}

// ────────────────── Halfword / Signed Byte Transfer ──────────────────
//
// cond | 000 P U I W L | Rn | Rd | off_hi(4) | 1 S H 1 | off_lo(4)/Rm
//   I=0: offset = Rm (bits 3..0). bits 11..8 devem ser 0.
//   I=1: offset = (off_hi << 4) | off_lo
//   SH:  01=LDRH/STRH, 10=LDRSB, 11=LDRSH

fn exec_halfword_transfer(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    let pre = (instr & (1 << 24)) != 0;
    let up = (instr & (1 << 23)) != 0;
    let imm = (instr & (1 << 22)) != 0;
    let writeback = (instr & (1 << 21)) != 0;
    let load = (instr & (1 << 20)) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let sh = (instr >> 5) & 0b11;

    let offset = if imm {
        ((instr >> 4) & 0xF0) | (instr & 0xF)
    } else {
        cpu.regs.get((instr & 0xF) as usize)
    };

    let base = cpu.regs.get(rn);
    let signed_offset = if up { offset } else { 0u32.wrapping_sub(offset) };
    let addr = if pre { base.wrapping_add(signed_offset) } else { base };

    if load {
        let value: u32 = match sh {
            0b01 => {
                // LDRH: unsigned halfword.
                // Endereço desalinhado em halfword: ROR de 8 bits.
                let aligned = addr & !1;
                let v = bus.read_u16(aligned) as u32;
                if addr & 1 != 0 { v.rotate_right(8) } else { v }
            }
            0b10 => {
                // LDRSB: signed byte.
                bus.read_u8(addr) as i8 as i32 as u32
            }
            0b11 => {
                // LDRSH: signed halfword. Se desalinhado, vira LDRSB!
                if addr & 1 != 0 {
                    bus.read_u8(addr) as i8 as i32 as u32
                } else {
                    bus.read_u16(addr) as i16 as i32 as u32
                }
            }
            _ => 0,
        };
        write_rd(cpu, rd, value);
    } else if sh == 0b01 {
        // STRH (apenas SH=01 é válido para store nesse formato).
        let v = cpu.regs.get(rd);
        bus.write_u16(addr & !1, v as u16);
    }

    let final_addr = base.wrapping_add(signed_offset);
    let do_writeback = !pre || writeback;
    if do_writeback && !(load && rd == rn) {
        cpu.regs.set(rn, final_addr);
    }
}

// ────────────────────── Block Data Transfer (LDM/STM) ──────────────────────
//
// cond | 100 P U S W L | Rn | register_list (16 bits)
//   P: pre/post indexed
//   U: up/down
//   S: PSR transfer / force user-mode banking (não totalmente implementado aqui)
//   W: writeback
//   L: load/store
//
// Sempre processado em ordem crescente de registrador (R0 mais baixo na memória),
// independentemente de U.

fn exec_block_data_transfer(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    let pre = (instr & (1 << 24)) != 0;
    let up = (instr & (1 << 23)) != 0;
    let psr_or_user = (instr & (1 << 22)) != 0; // S bit
    let writeback = (instr & (1 << 21)) != 0;
    let load = (instr & (1 << 20)) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let raw_list = (instr & 0xFFFF) as u16;

    // Quirk ARMv4: lista vazia transfere apenas R15 e ajusta Rn em ±0x40
    // (como se houvesse 16 registradores). A lógica de endereço abaixo, com
    // count=16, posiciona o R15 no slot inicial corretamente.
    let (list, count) = if raw_list == 0 {
        (0x8000u16, 16u32)
    } else {
        (raw_list, raw_list.count_ones())
    };

    let base = cpu.regs.get(rn);
    let final_addr = if up {
        base.wrapping_add(count * 4)
    } else {
        base.wrapping_sub(count * 4)
    };

    // Endereço inicial (menor): se U=0, é final_addr; se U=1, é base.
    let mut addr = if up { base } else { final_addr };
    // Ajuste por pre/post: equivale a deslocar +4 nos casos pre/up e post/down.
    if up == pre {
        addr = addr.wrapping_add(4);
    }
    // (Acima: para U=1,P=1 (IB) começamos em base+4; para U=0,P=0 (DA) começamos em final_addr.)
    // Reforçando os 4 casos:
    //   IA (U=1,P=0): start = base
    //   IB (U=1,P=1): start = base+4
    //   DA (U=0,P=0): start = final
    //   DB (U=0,P=1): start = final+4 ... mas DB também é "start = base - 4*count + 4 = final + 4"
    // A lógica acima cobre todos.

    // Semântica do bit S:
    //   - LDM com R15 na lista: restaura CPSR ← SPSR (retorno de exceção).
    //   - LDM/STM sem R15: força transferência do banco de registradores User.
    let r15_in_list = (list & 0x8000) != 0;
    let restore_cpsr = load && psr_or_user && r15_in_list;
    let force_user = psr_or_user && !r15_in_list;

    // Para o banco User, trocamos temporariamente para o modo System (que
    // compartilha R8..R14 com User). O endereço e o writeback continuam usando
    // o Rn do modo atual (calculados antes/depois da troca).
    let original_mode = cpu.cpsr.mode();
    if force_user {
        cpu.regs.switch_mode(CpuMode::System);
    }

    // Menor registrador da lista (para o quirk de STM com o base na lista).
    let lowest = list.trailing_zeros() as usize;

    for i in 0..16 {
        if list & (1 << i) == 0 {
            continue;
        }
        if load {
            let v = bus.read_u32(addr);
            if i == 15 {
                cpu.set_pc_arm(v);
            } else {
                cpu.regs.set(i, v);
            }
        } else {
            let mut v = cpu.regs.get(i);
            if i == 15 {
                v = v.wrapping_add(4); // PC+12
            } else if i == rn && writeback && i != lowest {
                // Quirk STM: o base está na lista mas não é o menor registrador
                // → grava-se o valor já com writeback, não o original.
                v = final_addr;
            }
            bus.write_u32(addr, v);
        }
        addr = addr.wrapping_add(4);
    }

    if force_user {
        cpu.regs.switch_mode(original_mode);
    }

    if writeback && !(load && (list & (1 << rn)) != 0) {
        cpu.regs.set(rn, final_addr);
    }

    if restore_cpsr {
        if let Some(idx) = cpu.cpsr.mode().spsr_index() {
            cpu.cpsr = cpu.spsr[idx];
            cpu.regs.switch_mode(cpu.cpsr.mode());
        }
    }
}

// ────────────────────── Single Data Swap ──────────────────────

/// SWP / SWPB — troca atômica entre registrador e memória.
/// Lê [Rn] para Rd e grava Rm em [Rn] (na ordem: read-old, write-new).
fn exec_swap(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    let byte = instr & (1 << 22) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;

    let addr = cpu.regs.get(rn);
    let store = cpu.regs.get(rm);

    if byte {
        let old = bus.read_u8(addr) as u32;
        bus.write_u8(addr, store as u8);
        cpu.regs.set(rd, old);
    } else {
        // Leitura de word com o quirk de rotação do ARMv4 em endereço desalinhado.
        let aligned = addr & !0x3;
        let old = bus.read_u32(aligned).rotate_right((addr & 0x3) * 8);
        bus.write_u32(aligned, store);
        cpu.regs.set(rd, old);
    }
}

// ────────────────────── SWI ──────────────────────

fn exec_swi(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    // Com BIOS HLE, o efeito da função é emulado em Rust e o fluxo continua na
    // próxima instrução (sem trocar modo/PC). O número da função vem dos bits 23..16.
    if bus.hle_bios {
        let comment = ((instr >> 16) & 0xFF) as u8;
        super::bios::dispatch(cpu, bus, comment);
        return;
    }

    // BIOS oficial: entra em Supervisor mode, salva CPSR em SPSR_svc, PC=0x08.
    let return_addr = cpu.regs.pc().wrapping_sub(4);
    let old_cpsr = cpu.cpsr;

    cpu.cpsr.set_mode(CpuMode::Supervisor);
    cpu.cpsr.set_flag(PsrFlags::T, false); // sempre entra em ARM
    cpu.cpsr.set_flag(PsrFlags::I, true);  // IRQ desabilitado
    cpu.regs.switch_mode(CpuMode::Supervisor);

    if let Some(idx) = CpuMode::Supervisor.spsr_index() {
        cpu.spsr[idx] = old_cpsr;
    }
    cpu.regs.set_lr(return_addr);
    cpu.set_pc_arm(0x0000_0008);
}
