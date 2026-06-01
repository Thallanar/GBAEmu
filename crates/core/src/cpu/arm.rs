//! Decoder e executor de instruções ARM (32 bits).
//!
//! Decodificação por bits[27:20] + bits[7:4]. Por ora implementamos apenas
//! a família **Branch (B / BL)**, que serve como prova do pipeline.
//! Demais famílias serão adicionadas nas próximas iterações da Fase 1:
//! Data Processing, Multiply, Single Data Transfer, Block Data Transfer,
//! SWI, etc.

use crate::bus::Bus;

use super::condition::Condition;
use super::Cpu;

/// Executa uma instrução ARM já buscada (32 bits).
pub fn execute(cpu: &mut Cpu, bus: &mut Bus, instr: u32) {
    // 1. Avalia a condição (bits 31..28).
    let cond = Condition::from_bits(instr >> 28);
    if !cond.evaluate(cpu.cpsr) {
        return;
    }

    // 2. Dispatch grosseiro por bits 27..25.
    let group = (instr >> 25) & 0b111;
    match group {
        0b101 => exec_branch(cpu, instr),
        _ => {
            log::warn!(
                "ARM: opcode não implementado @ PC={:08X} instr={:08X}",
                cpu.regs.pc().wrapping_sub(8),
                instr
            );
        }
    }
    let _ = bus; // bus será usado quando implementarmos loads/stores
}

/// B / BL: bits 27..25 == 101.
/// Bit 24 distingue: 0 = B, 1 = BL.
/// bits 23..0 = offset assinado em palavras (24 bits) → shift left 2 → 26 bits sinal-estendido.
fn exec_branch(cpu: &mut Cpu, instr: u32) {
    let link = (instr & (1 << 24)) != 0;

    // sign-extend de 24 → 32 bits, depois <<2 dá deslocamento em bytes.
    let raw24 = instr & 0x00FF_FFFF;
    let signed = ((raw24 << 8) as i32) >> 8; // ARS para preservar sinal
    let offset = (signed as i32).wrapping_mul(4);

    let pc = cpu.regs.pc(); // já adiantado de 8 pelo pipeline ARM
    let target = pc.wrapping_add(offset as u32);

    if link {
        // LR = endereço da PRÓXIMA instrução (PC-4 porque PC está adiantado +8).
        let return_addr = pc.wrapping_sub(4);
        cpu.regs.set_lr(return_addr);
    }

    cpu.set_pc_arm(target);
}
