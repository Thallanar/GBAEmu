//! CPU ARM7TDMI — modos ARM (32-bit) e THUMB (16-bit).
//!
//! Implementação será preenchida na Fase 1 do roadmap.

/// Modos de operação do processador ARM7TDMI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuMode {
    User,
    Fiq,
    Irq,
    Supervisor,
    Abort,
    Undefined,
    System,
}

/// Estado da CPU. 16 registradores visíveis + CPSR + banked registers.
pub struct Cpu {
    /// Registradores R0..R15 (R15 = PC).
    pub regs: [u32; 16],
    /// Current Program Status Register.
    pub cpsr: u32,
    /// Saved PSRs por modo.
    pub spsr: [u32; 5],
    pub mode: CpuMode,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: [0; 16],
            cpsr: 0,
            spsr: [0; 5],
            mode: CpuMode::System,
        }
    }

    /// Executa uma instrução. Retorna ciclos consumidos.
    pub fn step(&mut self) -> u32 {
        // TODO: decode ARM/THUMB e dispatch
        1
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
