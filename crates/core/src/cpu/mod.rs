//! CPU ARM7TDMI.
//!
//! Pipeline de 3 estágios: Fetch → Decode → Execute. Emulamos o efeito
//! visível: ao executar uma instrução ARM, PC já está adiantado em 8 bytes
//! (duas instruções de 4 bytes pré-buscadas). Em THUMB, adiantado em 4.

pub mod arm;
pub mod condition;
pub mod psr;
pub mod registers;
pub mod thumb;

use crate::bus::Bus;
use psr::{Cpsr, PsrFlags};
use registers::RegisterFile;

pub use psr::CpuMode;

pub struct Cpu {
    pub regs: RegisterFile,
    pub cpsr: Cpsr,
    /// SPSR bancado por modo (5 slots: FIQ, IRQ, SVC, ABT, UND).
    pub spsr: [Cpsr; 5],
    /// Sinaliza ao step() que uma instrução causou branch e PC já está no destino.
    pub(crate) branched: bool,
}

impl Cpu {
    pub fn new() -> Self {
        let mut cpu = Self {
            regs: RegisterFile::new(),
            cpsr: Cpsr::new(),
            spsr: [Cpsr::new(); 5],
            branched: false,
        };
        // Reset state: Supervisor mode, IRQ/FIQ off, ARM, PC=0 (vetor reset).
        cpu.cpsr = Cpsr(CpuMode::Supervisor as u32 | PsrFlags::I.bits() | PsrFlags::F.bits());
        cpu.regs.switch_mode(CpuMode::Supervisor);
        cpu.regs.set_pc(0);
        cpu
    }

    /// Executa uma instrução. Retorna ciclos consumidos (placeholder = 1).
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.cpsr.thumb() {
            self.step_thumb(bus)
        } else {
            self.step_arm(bus)
        }
    }

    fn step_arm(&mut self, bus: &mut Bus) -> u32 {
        let exec_pc = self.regs.pc();
        let instr = bus.read_u32(exec_pc);

        // Pré-adianta PC em +8 ANTES de execute, simulando o pipeline:
        // quando a instrução ler PC, vê exec_pc + 8.
        self.regs.set_pc(exec_pc.wrapping_add(8));
        arm::execute(self, bus, instr);

        // Se não houve branch, queremos terminar com PC = exec_pc + 4.
        if !self.branched {
            self.regs.set_pc(exec_pc.wrapping_add(4));
        }
        self.branched = false;
        1
    }

    fn step_thumb(&mut self, bus: &mut Bus) -> u32 {
        let exec_pc = self.regs.pc();
        let instr = bus.read_u16(exec_pc);
        self.regs.set_pc(exec_pc.wrapping_add(4));
        thumb::execute(self, bus, instr);
        if !self.branched {
            self.regs.set_pc(exec_pc.wrapping_add(2));
        }
        self.branched = false;
        1
    }

    /// Branch ARM: alinha em 4 e marca branch para o step pular o "recuo".
    pub(crate) fn set_pc_arm(&mut self, target: u32) {
        self.regs.set_pc(target & !0x3);
        self.branched = true;
    }

    pub(crate) fn set_pc_thumb(&mut self, target: u32) {
        self.regs.set_pc(target & !0x1);
        self.branched = true;
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gba_with_rom(rom_words: &[u32]) -> (Cpu, Bus) {
        let mut bus = Bus::new();
        // Coloca o código na EWRAM (0x02000000) para evitar dependência da ROM.
        for (i, w) in rom_words.iter().enumerate() {
            bus.write_u32(0x0200_0000 + (i as u32) * 4, *w);
        }
        let mut cpu = Cpu::new();
        cpu.regs.set_pc(0x0200_0000);
        (cpu, bus)
    }

    #[test]
    fn branch_forward_unconditional() {
        // B +8 (3 instruções à frente)  → encoding: EA 00 00 00 com offset 0
        // offset_24 = 0 → destino = PC(que vale exec_pc+8) + 0 = exec_pc+8
        // Cond=AL (0xE), bits 27..25 = 101, bit 24 = 0
        let b = 0xEA_00_00_00;
        let (mut cpu, mut bus) = make_gba_with_rom(&[b]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc(), 0x0200_0008, "PC deve apontar para exec_pc+8");
    }

    #[test]
    fn branch_link_saves_lr() {
        // BL +0 (link). Cond=AL, bit 24 = 1.
        let bl = 0xEB_00_00_00;
        let (mut cpu, mut bus) = make_gba_with_rom(&[bl]);
        cpu.step(&mut bus);
        // LR deve guardar endereço da próxima instrução (exec_pc + 4).
        assert_eq!(cpu.regs.lr(), 0x0200_0004);
    }

    #[test]
    fn branch_condition_false_skips() {
        // BEQ +8, mas Z=0 → não toma o branch, PC = exec_pc + 4.
        // Cond=EQ (0x0), bits 27..25 = 101.
        let beq = 0x0A_00_00_00;
        let (mut cpu, mut bus) = make_gba_with_rom(&[beq]);
        assert!(!cpu.cpsr.z());
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc(), 0x0200_0004);
    }
}
