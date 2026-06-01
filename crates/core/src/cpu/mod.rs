//! CPU ARM7TDMI.
//!
//! Pipeline de 3 estágios: Fetch → Decode → Execute. Emulamos o efeito
//! visível: ao executar uma instrução ARM, PC já está adiantado em 8 bytes
//! (duas instruções de 4 bytes pré-buscadas). Em THUMB, adiantado em 4.

pub mod alu;
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

    /// Retorna o SPSR do modo atual (None em User/System).
    pub(crate) fn current_spsr(&self) -> Option<psr::Cpsr> {
        self.cpsr.mode().spsr_index().map(|i| self.spsr[i])
    }

    #[allow(dead_code)]
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

    // ──────── Data Processing ────────

    /// MOV Rd, #imm (S=0). opcode=0xD, I=1.
    /// Cond=AL(0xE), bits 27..26=00, bit 25=1, opcode=1101, S=0, Rn=0(ignorado), Rd=0, imm=8.
    /// Encoding: 0xE3A0_0008  (MOV r0, #8)
    #[test]
    fn mov_immediate() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE3A0_0008]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 8);
    }

    /// ADD r1, r0, #5  →  0xE280_1005
    #[test]
    fn add_immediate() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE280_1005]);
        cpu.regs.set(0, 10);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(1), 15);
    }

    /// SUBS r0, r0, #1  com r0=0 → resultado 0xFFFFFFFF, N=1, Z=0, C=0 (borrow), V=0.
    /// Encoding: 0xE250_0001
    #[test]
    fn subs_sets_flags_negative_and_borrow() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE250_0001]);
        cpu.regs.set(0, 0);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 0xFFFF_FFFF);
        assert!(cpu.cpsr.n());
        assert!(!cpu.cpsr.z());
        assert!(!cpu.cpsr.c()); // borrow → C=0 na convenção ARM
    }

    /// CMP r0, #5 com r0=5 → Z=1, N=0, C=1 (sem borrow).
    /// CMP = opcode 0xA, S=1, sem write. Encoding: 0xE350_0005.
    #[test]
    fn cmp_equal_sets_zero() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE350_0005]);
        cpu.regs.set(0, 5);
        cpu.step(&mut bus);
        assert!(cpu.cpsr.z());
        assert!(!cpu.cpsr.n());
        assert!(cpu.cpsr.c());
        assert_eq!(cpu.regs.get(0), 5, "CMP não deve escrever em Rd");
    }

    /// MOVS r0, r1, LSL #1 — testa barrel shifter com flag C.
    /// I=0, opcode=MOV(0xD), S=1, Rd=0, Rm=1, shift LSL #1.
    /// Encoding: 0xE1B0_0081
    #[test]
    fn movs_lsl_sets_carry() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE1B0_0081]);
        cpu.regs.set(1, 0x8000_0000);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 0);
        assert!(cpu.cpsr.z());
        assert!(cpu.cpsr.c(), "LSL #1 sobre bit 31 deve setar C");
    }

    /// ORR r0, r0, #0xFF — testa lógica + sem alteração de C.
    /// Encoding: 0xE380_00FF
    #[test]
    fn orr_immediate() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE380_00FF]);
        cpu.regs.set(0, 0xFF00);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 0xFFFF);
    }

    /// MRS r0, CPSR — copia CPSR para R0.
    /// Encoding: 0xE10F_0000
    #[test]
    fn mrs_reads_cpsr() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE10F_0000]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), cpu.cpsr.0);
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
