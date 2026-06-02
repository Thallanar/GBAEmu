//! CPU ARM7TDMI.
//!
//! Pipeline de 3 estágios: Fetch → Decode → Execute. Emulamos o efeito
//! visível: ao executar uma instrução ARM, PC já está adiantado em 8 bytes
//! (duas instruções de 4 bytes pré-buscadas). Em THUMB, adiantado em 4.

pub mod alu;
pub mod arm;
pub mod bios;
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
    /// CPU em estado de Halt (SWI Halt/IntrWait): dorme até IE & IF != 0.
    pub halted: bool,
    /// Contadores de telemetria — úteis para smoke testing.
    pub stats: CpuStats,
}

#[derive(Default)]
pub struct CpuStats {
    pub arm_executed: u64,
    pub thumb_executed: u64,
    pub arm_unimplemented: u64,
    pub thumb_unimplemented: u64,
    /// Últimos N opcodes não implementados (pc, instr, is_thumb).
    pub recent_unimplemented: Vec<(u32, u32, bool)>,
}

impl CpuStats {
    pub fn record_unimpl(&mut self, pc: u32, instr: u32, thumb: bool) {
        if thumb {
            self.thumb_unimplemented += 1;
        } else {
            self.arm_unimplemented += 1;
        }
        if self.recent_unimplemented.len() < 32 {
            self.recent_unimplemented.push((pc, instr, thumb));
        }
    }
}

impl Cpu {
    pub fn new() -> Self {
        let mut cpu = Self {
            regs: RegisterFile::new(),
            cpsr: Cpsr::new(),
            spsr: [Cpsr::new(); 5],
            branched: false,
            halted: false,
            stats: CpuStats::default(),
        };
        // Reset state: Supervisor mode, IRQ/FIQ off, ARM, PC=0 (vetor reset).
        cpu.cpsr = Cpsr(CpuMode::Supervisor as u32 | PsrFlags::I.bits() | PsrFlags::F.bits());
        cpu.regs.switch_mode(CpuMode::Supervisor);
        cpu.regs.set_pc(0);
        cpu
    }

    /// Executa uma instrução. Retorna ciclos consumidos (placeholder = 1).
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        // Halt (SWI Halt/IntrWait): dorme queimando ciclos até IE & IF != 0,
        // independentemente do IME. O timer/PPU continuam avançando no `Gba::step`.
        if self.halted {
            if bus.io.halt_condition_met() {
                self.halted = false;
            } else {
                return 1;
            }
        }

        // Atende IRQ pendente ANTES de executar (se permitido pelo CPSR).
        if bus.io.irq_pending() && !self.cpsr.irq_disabled() {
            self.enter_irq();
            return 1;
        }

        if self.cpsr.thumb() {
            self.step_thumb(bus)
        } else {
            self.step_arm(bus)
        }
    }

    /// Entrada na exceção IRQ.
    /// Salva CPSR em SPSR_irq, PC = 0x18 (trampolim da BIOS), modo IRQ.
    ///
    /// `LR_irq = PC + 4`: no momento do check, `pc()` aponta para a próxima
    /// instrução (N). O trampolim retorna com `SUBS PC, LR, #4` → N, tanto em
    /// ARM quanto em THUMB (o bit T é restaurado do SPSR).
    fn enter_irq(&mut self) {
        let return_addr = self.regs.pc().wrapping_add(4);
        let old_cpsr = self.cpsr;
        self.cpsr.set_mode(CpuMode::Irq);
        self.cpsr.set_flag(PsrFlags::T, false);
        self.cpsr.set_flag(PsrFlags::I, true);
        self.regs.switch_mode(CpuMode::Irq);
        if let Some(idx) = CpuMode::Irq.spsr_index() {
            self.spsr[idx] = old_cpsr;
        }
        self.regs.set_lr(return_addr);
        self.set_pc_arm(0x0000_0018);
    }

    fn step_arm(&mut self, bus: &mut Bus) -> u32 {
        let exec_pc = self.regs.pc();
        let instr = bus.read_u32(exec_pc);
        self.stats.arm_executed += 1;

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
        self.stats.thumb_executed += 1;
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

    // ──────── Multiply ────────

    /// MUL r0, r1, r2  →  encoding: cond=E, 0000 00 00, Rd=0, Rn=0, Rs=2, 1001, Rm=1
    /// = 0xE000_0291
    #[test]
    fn mul_basic() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE000_0291]);
        cpu.regs.set(1, 7);
        cpu.regs.set(2, 6);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 42);
    }

    /// MLA r0, r1, r2, r3 (r0 = r1*r2 + r3)
    /// Encoding: Cond=E, 000000 A=1 S=0, Rd=0, Rn=3, Rs=2, 1001, Rm=1 → 0xE020_3291
    #[test]
    fn mla_accumulates() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE020_3291]);
        cpu.regs.set(1, 5);
        cpu.regs.set(2, 6);
        cpu.regs.set(3, 7);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 37);
    }

    /// UMULL r0, r1, r2, r3  → low=r0, high=r1, r2*r3 unsigned
    /// Encoding: cond=E, 0000 100 0 (U=0 unsigned, A=0, S=0), RdHi=1, RdLo=0, Rs=3, 1001, Rm=2
    /// = 0xE081_0392
    #[test]
    fn umull_basic() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE081_0392]);
        cpu.regs.set(2, 0xFFFF_FFFF);
        cpu.regs.set(3, 2);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 0xFFFF_FFFE);
        assert_eq!(cpu.regs.get(1), 0x0000_0001);
    }

    // ──────── LDR / STR ────────

    /// STR r1, [r0, #4]; LDR r2, [r0, #4]  — escreve e lê de volta.
    /// STR: cond=E, 01 01 1000, Rn=0, Rd=1, off=4 → 0xE580_1004
    /// LDR: cond=E, 01 01 1001, Rn=0, Rd=2, off=4 → 0xE590_2004
    #[test]
    fn str_then_ldr_roundtrip() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE580_1004, 0xE590_2004]);
        cpu.regs.set(0, 0x0300_0000); // base aponta para IWRAM
        cpu.regs.set(1, 0xDEAD_BEEF);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(2), 0xDEAD_BEEF);
    }

    /// STRB / LDRB — escreve um byte e lê de volta.
    /// STRB r1, [r0]  → 0xE5C0_1000
    /// LDRB r2, [r0]  → 0xE5D0_2000
    #[test]
    fn strb_then_ldrb() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE5C0_1000, 0xE5D0_2000]);
        cpu.regs.set(0, 0x0300_0010);
        cpu.regs.set(1, 0xAB);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(2), 0xAB);
    }

    /// LDR com endereço desalinhado: addr=0x...01 → ROR de 8 bits.
    /// STR r1, [r0]; LDR r2, [r0, #1] (sem writeback)
    /// LDR: 0xE590_2001
    #[test]
    fn ldr_unaligned_rotates() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE580_1000, 0xE590_2001]);
        cpu.regs.set(0, 0x0300_0020);
        cpu.regs.set(1, 0x1234_5678);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        // Ler em offset 1: ROR(0x12345678, 8) = 0x78123456
        assert_eq!(cpu.regs.get(2), 0x7812_3456);
    }

    // ──────── Halfword Transfer ────────

    /// STRH r1, [r0]; LDRH r2, [r0]
    /// STRH: cond=E, 000 P=1 U=1 I=1 W=0 L=0, Rn=0, Rd=1, 0000_1011_0000 → 0xE1C0_10B0
    /// LDRH: cond=E, 000 P=1 U=1 I=1 W=0 L=1, Rn=0, Rd=2, 0000_1011_0000 → 0xE1D0_20B0
    #[test]
    fn strh_then_ldrh() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE1C0_10B0, 0xE1D0_20B0]);
        cpu.regs.set(0, 0x0300_0030);
        cpu.regs.set(1, 0xFFFF_BEEF);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        // LDRH só carrega 16 bits baixos, zero-extend.
        assert_eq!(cpu.regs.get(2), 0x0000_BEEF);
    }

    /// LDRSB r2, [r0] (signed byte). Byte 0xFF → 0xFFFFFFFF após sign-extend.
    /// STRB r1, [r0]; LDRSB r2, [r0]
    /// LDRSB encoding: cond=E, 000 P=1 U=1 I=1 W=0 L=1, Rn=0, Rd=2, 0000_1101_0000 → 0xE1D0_20D0
    #[test]
    fn ldrsb_sign_extends() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE5C0_1000, 0xE1D0_20D0]);
        cpu.regs.set(0, 0x0300_0040);
        cpu.regs.set(1, 0xFF);
        cpu.step(&mut bus); // STRB
        cpu.step(&mut bus); // LDRSB
        assert_eq!(cpu.regs.get(2), 0xFFFF_FFFF);
    }

    // ──────── Block Data Transfer (LDM/STM) ────────

    /// STMIA r0!, {r1, r2, r3}  e depois LDMIA r0!, {r4, r5, r6}.
    /// STM: cond=E, 100 P=0 U=1 S=0 W=1 L=0, Rn=0, list={r1,r2,r3}=0x000E → 0xE8A0_000E
    /// LDM: cond=E, 100 P=0 U=1 S=0 W=1 L=1, Rn=0, list={r4,r5,r6}=0x0070 → 0xE8B0_0070
    #[test]
    fn stmia_then_ldmia() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE8A0_000E, 0xE8B0_0070]);
        cpu.regs.set(0, 0x0300_0080);
        cpu.regs.set(1, 0x1111);
        cpu.regs.set(2, 0x2222);
        cpu.regs.set(3, 0x3333);
        cpu.step(&mut bus); // STM!
        assert_eq!(cpu.regs.get(0), 0x0300_008C); // 12 bytes adiantados
        // Volta o base e recarrega em r4,r5,r6:
        cpu.regs.set(0, 0x0300_0080);
        cpu.step(&mut bus); // LDM!
        assert_eq!(cpu.regs.get(4), 0x1111);
        assert_eq!(cpu.regs.get(5), 0x2222);
        assert_eq!(cpu.regs.get(6), 0x3333);
        assert_eq!(cpu.regs.get(0), 0x0300_008C);
    }

    /// LDM com lista vazia (quirk ARMv4): transfere R15 e ajusta Rn em 0x40.
    /// LDMIA r0!, {} → 0xE8B0_0000
    #[test]
    fn ldm_empty_list_loads_pc_and_adds_0x40() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE8B0_0000]);
        cpu.regs.set(0, 0x0300_0000);
        bus.write_u32(0x0300_0000, 0x0800_1234); // destino do PC (alinhado em ARM)
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc(), 0x0800_1234, "R15 carregado do endereço base");
        assert_eq!(cpu.regs.get(0), 0x0300_0040, "Rn += 0x40");
    }

    /// Quirk STM: base na lista, mas não é o menor registrador → grava o valor
    /// já com writeback. STMDB r1!, {r0-r3} (STMFD) → 0xE921_000F.
    #[test]
    fn stm_base_in_list_not_lowest_stores_writeback() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE921_000F]);
        cpu.regs.set(0, 0xA);
        cpu.regs.set(1, 0x0300_0040); // base (também na lista, em r1)
        cpu.regs.set(2, 0xC);
        cpu.regs.set(3, 0xD);
        cpu.step(&mut bus);
        // STMFD com 4 regs: final = base - 0x10. r1 grava o valor final.
        let final_base = 0x0300_0040u32 - 0x10;
        assert_eq!(cpu.regs.get(1), final_base, "writeback de r1");
        // Memória: r0..r3 nas posições crescentes a partir de final_base.
        assert_eq!(bus.read_u32(final_base), 0xA, "r0 (menor) inalterado");
        assert_eq!(bus.read_u32(final_base + 4), final_base, "r1 grava o valor com writeback");
        assert_eq!(bus.read_u32(final_base + 8), 0xC);
        assert_eq!(bus.read_u32(final_base + 12), 0xD);
    }

    // ──────── Single Data Swap ────────

    /// SWP r2, r1, [r0] — troca word: r2 = [r0]; [r0] = r1.
    /// Encoding: cond=E, 00010 B=0 00, Rn=0, Rd=2, 0000 1001, Rm=1 → 0xE1002091
    #[test]
    fn swp_word_exchanges() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE100_2091]);
        cpu.regs.set(0, 0x0300_0040); // endereço (IWRAM)
        cpu.regs.set(1, 0xCAFE_BABE); // valor a gravar
        bus.write_u32(0x0300_0040, 0x1234_5678); // valor antigo na memória
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(2), 0x1234_5678, "Rd recebe o valor antigo");
        assert_eq!(bus.read_u32(0x0300_0040), 0xCAFE_BABE, "memória recebe Rm");
    }

    /// SWPB r2, r1, [r0] — troca byte.
    /// Encoding: cond=E, 00010 B=1 00, Rn=0, Rd=2, 0000 1001, Rm=1 → 0xE1402091
    #[test]
    fn swpb_byte_exchanges() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE140_2091]);
        cpu.regs.set(0, 0x0300_0050);
        cpu.regs.set(1, 0xAB);
        bus.write_u8(0x0300_0050, 0xCD);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(2), 0xCD, "Rd recebe o byte antigo");
        assert_eq!(bus.read_u8(0x0300_0050), 0xAB, "memória recebe o byte de Rm");
    }

    /// SWP com Rd == Rm: troca registrador com memória de forma atômica.
    /// SWP r1, r1, [r0] → 0xE1001091
    #[test]
    fn swp_same_reg_swaps_with_memory() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE100_1091]);
        cpu.regs.set(0, 0x0300_0060);
        cpu.regs.set(1, 0x1111_2222);
        bus.write_u32(0x0300_0060, 0xAAAA_BBBB);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(1), 0xAAAA_BBBB);
        assert_eq!(bus.read_u32(0x0300_0060), 0x1111_2222);
    }

    // ──────── SWI ────────

    /// SWI #0 com BIOS oficial → entra em Supervisor, PC=0x08, LR_svc = exec_pc+4.
    /// Encoding: cond=E, 1111 0000 ... → 0xEF00_0000
    #[test]
    fn swi_enters_supervisor_mode() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xEF00_0000]);
        bus.hle_bios = false; // testa o caminho do vetor de exceção real
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc(), 0x08);
        assert_eq!(cpu.cpsr.mode(), CpuMode::Supervisor);
        assert_eq!(cpu.regs.lr(), 0x0200_0004);
    }

    /// SWI 0x06 (Div) com BIOS HLE: 10 / 3 → r0=3 (quociente), r1=1 (resto).
    /// O fluxo continua na próxima instrução (sem trocar modo/PC).
    /// Encoding ARM: 0xEF06_0000 (comment = 0x06 nos bits 23..16).
    #[test]
    fn hle_swi_div() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xEF06_0000]);
        cpu.regs.set(0, 10);
        cpu.regs.set(1, 3);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 3, "quociente");
        assert_eq!(cpu.regs.get(1), 1, "resto");
        assert_eq!(cpu.regs.get(3), 3, "|quociente|");
        assert_eq!(cpu.regs.pc(), 0x0200_0004, "continua na próxima instrução");
        assert_eq!(cpu.cpsr.mode(), CpuMode::Supervisor, "não troca de modo");
    }

    /// SWI 0x02 (Halt) com HLE seguido de IRQ acorda a CPU e despacha.
    #[test]
    fn hle_swi_halt_then_wakes_on_irq() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xEF02_0000, 0xE320_F000]);
        cpu.step(&mut bus); // SWI Halt → cpu.halted = true
        assert!(cpu.halted);

        // Sem IRQ pendente: continua dormindo, PC não avança.
        let pc = cpu.regs.pc();
        cpu.step(&mut bus);
        assert!(cpu.halted);
        assert_eq!(cpu.regs.pc(), pc);

        // Levanta uma IRQ habilitada: acorda e (com IME+I ok) despacha ao vetor.
        bus.io.ie = 0x0001;
        bus.io.iflag = 0x0001;
        bus.io.ime = true;
        cpu.cpsr.set_flag(psr::PsrFlags::I, false);
        cpu.step(&mut bus); // sai do halt e entra em IRQ
        assert!(!cpu.halted);
        assert_eq!(cpu.regs.pc(), 0x18);
    }

    // ════════════════ THUMB ════════════════
    //
    // Para os testes THUMB, escrevemos as instruções como halfwords na EWRAM
    // e mudamos a CPU para estado THUMB.

    fn make_gba_thumb(words: &[u16]) -> (Cpu, Bus) {
        let mut bus = Bus::new();
        for (i, w) in words.iter().enumerate() {
            bus.write_u16(0x0200_0000 + (i as u32) * 2, *w);
        }
        let mut cpu = Cpu::new();
        cpu.cpsr.set_flag(psr::PsrFlags::T, true);
        cpu.regs.set_pc(0x0200_0000);
        (cpu, bus)
    }

    /// Fmt 3: MOV r0, #42  → 0010 0 000 00101010 = 0x202A
    #[test]
    fn thumb_mov_imm() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0x202A]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(0), 42);
    }

    /// Fmt 2: ADD r2, r0, r1 (op=ADD, I=0) → 0001100 001 000 010 = 0x1842
    #[test]
    fn thumb_add_reg() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0x1842]);
        cpu.regs.set(0, 10);
        cpu.regs.set(1, 20);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(2), 30);
    }

    /// Fmt 1: LSL r1, r0, #4  → 00000 00100 000 001 = 0x0101
    #[test]
    fn thumb_lsl_imm() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0x0101]);
        cpu.regs.set(0, 0xAB);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.get(1), 0xAB0);
    }

    /// Fmt 5: BX r0 — pula para endereço em r0, com troca de estado pelo bit 0.
    /// op=BX(3), H1=0, H2=0, Rs=0, Rd=0 → 010001 11 0 0 000 000 = 0x4700
    #[test]
    fn thumb_bx_to_arm() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0x4700]);
        cpu.regs.set(0, 0x0200_0100); // bit 0 = 0 → ARM
        cpu.step(&mut bus);
        assert!(!cpu.cpsr.thumb(), "Deve estar em ARM agora");
        assert_eq!(cpu.regs.pc(), 0x0200_0100);
    }

    /// Fmt 14: PUSH {r0, r1}; POP {r2, r3}
    /// PUSH {r0,r1}: 1011 010 0 00000011 = 0xB403
    /// POP  {r2,r3}: 1011 110 0 00001100 = 0xBC0C
    #[test]
    fn thumb_push_pop() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0xB403, 0xBC0C]);
        cpu.regs.set(13, 0x0300_7F00); // SP
        cpu.regs.set(0, 0xAAAA);
        cpu.regs.set(1, 0xBBBB);
        cpu.step(&mut bus); // PUSH
        cpu.step(&mut bus); // POP
        assert_eq!(cpu.regs.get(2), 0xAAAA);
        assert_eq!(cpu.regs.get(3), 0xBBBB);
        assert_eq!(cpu.regs.get(13), 0x0300_7F00);
    }

    /// Fmt 18: B +4 (unconditional). offset_11 = 0x002 → +4 bytes (2 instr à frente).
    /// 11100 00000000010 = 0xE002
    #[test]
    fn thumb_unconditional_branch() {
        let (mut cpu, mut bus) = make_gba_thumb(&[0xE002]);
        cpu.step(&mut bus);
        // PC era 0x0200_0000, ficou +4 (do pipeline) +4 (offset) = 0x0200_0008
        assert_eq!(cpu.regs.pc(), 0x0200_0008);
    }

    // ──────── IRQ ────────

    /// Quando IE & IF != 0 e IME=1 e CPSR.I=0, a CPU entra em IRQ mode
    /// no próximo step (vetor 0x18).
    #[test]
    fn irq_dispatches_to_vector() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE320_F000]); // NOP (MSR cpsr_c, #0 — no-op safe)
        // Habilita IRQ globalmente e no CPSR.
        bus.io.ime = true;
        bus.io.ie = 0x0008; // TIMER0
        bus.io.iflag = 0x0008;
        cpu.cpsr.set_flag(psr::PsrFlags::I, false);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc(), 0x18);
        assert_eq!(cpu.cpsr.mode(), CpuMode::Irq);
        assert!(cpu.cpsr.irq_disabled(), "I bit deve ser setado ao entrar na exceção");
    }

    #[test]
    fn irq_blocked_when_cpsr_i_set() {
        let (mut cpu, mut bus) = make_gba_with_rom(&[0xE320_F000]);
        bus.io.ime = true;
        bus.io.ie = 0x0001;
        bus.io.iflag = 0x0001;
        cpu.cpsr.set_flag(psr::PsrFlags::I, true);
        cpu.step(&mut bus);
        assert_ne!(cpu.regs.pc(), 0x18, "IRQ não deve disparar com I=1");
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
