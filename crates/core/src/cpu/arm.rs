//! Decoder e executor de instruções ARM (32 bits).
//!
//! Implementado até agora:
//!   - Branch (B / BL)
//!   - Data Processing (16 opcodes) com barrel shifter
//!   - PSR Transfer (MRS / MSR)
//!
//! Pendente: Multiply, LDR/STR, LDM/STM, SWI, Coprocessor (não usado no GBA).

use crate::bus::Bus;

use super::alu::{
    adc_with_flags, add_with_flags, barrel_shift, sbc_with_flags, sub_with_flags, ShiftKind,
};
use super::condition::Condition;
use super::psr::{Cpsr, CpuMode, PsrFlags};
use super::Cpu;

/// Executa uma instrução ARM já buscada (32 bits).
pub fn execute(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    let cond = Condition::from_bits(instr >> 28);
    if !cond.evaluate(cpu.cpsr) {
        return;
    }

    let group = (instr >> 25) & 0b111;
    match group {
        0b101 => exec_branch(cpu, instr),
        0b000 | 0b001 => exec_data_processing_or_psr(cpu, instr),
        _ => {
            log::warn!(
                "ARM: opcode não implementado @ PC={:08X} instr={:08X}",
                cpu.regs.pc().wrapping_sub(8),
                instr
            );
        }
    }
    let _ = bus; // bus será usado quando implementarmos LDR/STR
}

// ─────────────────────────── Branch ───────────────────────────

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

// ───────────────────── Data Processing + PSR Transfer ─────────────────────

/// O encoding de data-processing colide com MRS/MSR quando S=0 e o opcode
/// está em {TST, TEQ, CMP, CMN} (0x8..0xB). Resolvemos aqui no dispatch.
fn exec_data_processing_or_psr(cpu: &mut Cpu, instr: u32) {
    let opcode = (instr >> 21) & 0xF;
    let set_flags = (instr & (1 << 20)) != 0;

    // Detecção de MRS/MSR: opcode 0x8..=0xB e S=0.
    if (0x8..=0xB).contains(&opcode) && !set_flags {
        exec_psr_transfer(cpu, instr);
        return;
    }
    exec_data_processing(cpu, instr);
}

fn exec_data_processing(cpu: &mut Cpu, instr: u32) {
    let imm_operand = (instr & (1 << 25)) != 0;
    let opcode = (instr >> 21) & 0xF;
    let set_flags = (instr & (1 << 20)) != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;

    // Computa operand2 (com barrel shifter) e o eventual carry-out.
    let carry_in = cpu.cpsr.c();
    let (op2, shifter_carry) = compute_operand2(cpu, instr, imm_operand, carry_in);

    let a = cpu.regs.get(rn);

    // Quando Rn=R15 e o shift é por registrador, ARM lê PC+12 ao invés de +8.
    // Para já cobrir esse caso ao lermos a:
    let a = if rn == 15 && !imm_operand && (instr & (1 << 4)) != 0 {
        a.wrapping_add(4)
    } else {
        a
    };

    use OpResult::*;
    let result = match opcode {
        0x0 => Logical(a & op2),                                  // AND
        0x1 => Logical(a ^ op2),                                  // EOR
        0x2 => Arith(sub_with_flags(a, op2)),                     // SUB
        0x3 => Arith(sub_with_flags(op2, a)),                     // RSB
        0x4 => Arith(add_with_flags(a, op2)),                     // ADD
        0x5 => Arith(adc_with_flags(a, op2, carry_in)),           // ADC
        0x6 => Arith(sbc_with_flags(a, op2, carry_in)),           // SBC
        0x7 => Arith(sbc_with_flags(op2, a, carry_in)),           // RSC
        0x8 => LogicalNoWrite(a & op2),                           // TST
        0x9 => LogicalNoWrite(a ^ op2),                           // TEQ
        0xA => ArithNoWrite(sub_with_flags(a, op2)),              // CMP
        0xB => ArithNoWrite(add_with_flags(a, op2)),              // CMN
        0xC => Logical(a | op2),                                  // ORR
        0xD => Logical(op2),                                      // MOV
        0xE => Logical(a & !op2),                                 // BIC
        0xF => Logical(!op2),                                     // MVN
        _ => unreachable!(),
    };

    // Aplica resultado + flags.
    match result {
        Logical(v) | LogicalNoWrite(v) => {
            let writes = matches!(result, Logical(_));
            if writes {
                write_rd(cpu, rd, v);
            }
            if set_flags {
                cpu.cpsr.set_nz(v);
                cpu.cpsr.set_flag(PsrFlags::C, shifter_carry);
                // V não é alterado por operações lógicas.
            }
        }
        Arith(o) | ArithNoWrite(o) => {
            let writes = matches!(result, Arith(_));
            if writes {
                write_rd(cpu, rd, o.value);
            }
            if set_flags {
                cpu.cpsr.set_nz(o.value);
                cpu.cpsr.set_flag(PsrFlags::C, o.carry);
                cpu.cpsr.set_flag(PsrFlags::V, o.overflow);
            }
        }
    }

    // Caso especial: se Rd=R15 e S=1, CPSR ← SPSR_<mode> (return-from-exception).
    if rd == 15 && set_flags {
        if let Some(idx) = cpu.cpsr.mode().spsr_index() {
            let spsr = cpu.spsr[idx];
            cpu.cpsr = spsr;
            cpu.regs.switch_mode(cpu.cpsr.mode());
        }
    }
}

enum OpResult {
    Logical(u32),
    LogicalNoWrite(u32),
    Arith(super::alu::ArithOut),
    ArithNoWrite(super::alu::ArithOut),
}

/// Escreve em Rd. Se Rd=R15, sinaliza branch para o pipeline.
fn write_rd(cpu: &mut Cpu, rd: usize, value: u32) {
    if rd == 15 {
        cpu.set_pc_arm(value);
    } else {
        cpu.regs.set(rd, value);
    }
}

/// Calcula o operand2 do data-processing.
/// Retorna (valor, carry-out do shifter).
fn compute_operand2(cpu: &Cpu, instr: u32, imm: bool, carry_in: bool) -> (u32, bool) {
    if imm {
        // Operand2 = 8-bit value rotated right by 2*rotate_imm.
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
            // bit 7 deve ser 0 nessa forma (caso contrário, seria multiply).
            let rs = ((instr >> 8) & 0xF) as usize;
            // Se Rm/Rn=R15 em "shift by register", o valor é PC+12 (já tratamos Rn fora).
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

// ─────────────────────────── PSR transfer ───────────────────────────

fn exec_psr_transfer(cpu: &mut Cpu, instr: u32) {
    let use_spsr = (instr & (1 << 22)) != 0;
    let is_msr = (instr & (1 << 21)) != 0;

    if !is_msr {
        // MRS Rd, CPSR/SPSR
        let rd = ((instr >> 12) & 0xF) as usize;
        let value = if use_spsr {
            cpu.current_spsr().map(|p| p.0).unwrap_or(cpu.cpsr.0)
        } else {
            cpu.cpsr.0
        };
        cpu.regs.set(rd, value);
    } else {
        // MSR CPSR/SPSR_<fields>, operand
        let imm_operand = (instr & (1 << 25)) != 0;
        let operand = if imm_operand {
            let rotate = ((instr >> 8) & 0xF) * 2;
            (instr & 0xFF).rotate_right(rotate)
        } else {
            cpu.regs.get((instr & 0xF) as usize)
        };

        // field mask bits 19..16: f(31..24), s(23..16), x(15..8), c(7..0)
        let mut mask: u32 = 0;
        if instr & (1 << 19) != 0 { mask |= 0xFF00_0000; }
        if instr & (1 << 18) != 0 { mask |= 0x00FF_0000; }
        if instr & (1 << 17) != 0 { mask |= 0x0000_FF00; }
        if instr & (1 << 16) != 0 { mask |= 0x0000_00FF; }

        // No modo User, só os bits de flag podem ser escritos no CPSR.
        let in_user = cpu.cpsr.mode() == CpuMode::User;
        let effective_mask = if !use_spsr && in_user { mask & 0xFF00_0000 } else { mask };

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
