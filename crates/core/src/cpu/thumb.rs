//! Decoder e executor de instruções THUMB (16 bits).
//! A implementação completa virá após terminarmos ARM.

use crate::bus::Bus;
use super::Cpu;

pub fn execute(cpu: &mut Cpu, _bus: &mut Bus, instr: u16) {
    log::warn!(
        "THUMB: opcode não implementado @ PC={:08X} instr={:04X}",
        cpu.regs.pc().wrapping_sub(4),
        instr
    );
}
