//! Register file do ARM7TDMI com banking.
//!
//! Visíveis ao software: R0..R15. Internamente, R8-R14 são "bancados"
//! conforme o modo: FIQ tem R8_fiq..R14_fiq próprios; IRQ/SVC/ABT/UND
//! banking apenas R13 e R14. User e System compartilham o mesmo banco.
//!
//! R15 = PC (sempre compartilhado). R13 = SP, R14 = LR por convenção.

use super::psr::CpuMode;

/// Conjunto de bancos auxiliares por modo. Os "ativos" ficam em
/// [`RegisterFile::active`], e quando o modo muda fazemos swap.
#[derive(Default, Clone)]
struct BankedSet {
    /// R13 (SP) e R14 (LR) bancados.
    sp: u32,
    lr: u32,
}

#[derive(Default, Clone)]
struct FiqBank {
    /// R8..R12 bancados além de SP e LR.
    r8_12: [u32; 5],
    sp: u32,
    lr: u32,
}

pub struct RegisterFile {
    /// Registradores atualmente visíveis (R0..R15).
    pub active: [u32; 16],

    /// Cópia "user mode" de R8..R12 (usada quando entramos em FIQ).
    user_r8_12: [u32; 5],
    /// Banco User/System de R13/R14.
    user_bank: BankedSet,

    /// Banco FIQ (R8..R14_fiq).
    fiq: FiqBank,
    /// Bancos IRQ/SVC/ABT/UND (apenas R13/R14).
    irq: BankedSet,
    svc: BankedSet,
    abt: BankedSet,
    und: BankedSet,

    current_mode: CpuMode,
}

impl RegisterFile {
    pub fn new() -> Self {
        Self {
            active: [0; 16],
            user_r8_12: [0; 5],
            user_bank: BankedSet::default(),
            fiq: FiqBank::default(),
            irq: BankedSet::default(),
            svc: BankedSet::default(),
            abt: BankedSet::default(),
            und: BankedSet::default(),
            current_mode: CpuMode::System,
        }
    }

    /// Lê R[idx]. `idx` 0..=15.
    #[inline]
    pub fn get(&self, idx: usize) -> u32 {
        self.active[idx]
    }

    /// Escreve R[idx].
    #[inline]
    pub fn set(&mut self, idx: usize, value: u32) {
        self.active[idx] = value;
    }

    #[inline] pub fn pc(&self) -> u32 { self.active[15] }
    #[inline] pub fn set_pc(&mut self, v: u32) { self.active[15] = v; }
    #[inline] pub fn lr(&self) -> u32 { self.active[14] }
    #[inline] pub fn set_lr(&mut self, v: u32) { self.active[14] = v; }
    #[inline] pub fn sp(&self) -> u32 { self.active[13] }

    /// Troca o banco ativo para refletir um novo modo.
    /// Chamado quando o CPSR muda de modo (via MSR ou exception entry).
    pub fn switch_mode(&mut self, new: CpuMode) {
        if new == self.current_mode {
            return;
        }
        self.save_current_bank();
        self.load_bank(new);
        self.current_mode = new;
    }

    fn save_current_bank(&mut self) {
        match self.current_mode {
            CpuMode::Fiq => {
                self.fiq.r8_12.copy_from_slice(&self.active[8..13]);
                self.fiq.sp = self.active[13];
                self.fiq.lr = self.active[14];
            }
            CpuMode::Irq => { self.irq.sp = self.active[13]; self.irq.lr = self.active[14]; }
            CpuMode::Supervisor => { self.svc.sp = self.active[13]; self.svc.lr = self.active[14]; }
            CpuMode::Abort => { self.abt.sp = self.active[13]; self.abt.lr = self.active[14]; }
            CpuMode::Undefined => { self.und.sp = self.active[13]; self.und.lr = self.active[14]; }
            CpuMode::User | CpuMode::System => {
                self.user_bank.sp = self.active[13];
                self.user_bank.lr = self.active[14];
            }
        }

        // Ao SAIR de FIQ, restauramos a cópia "user" de R8..R12.
        // Para outros modos, R8..R12 já são os user (não foram modificados).
        if self.current_mode == CpuMode::Fiq {
            // Já salvamos os r8_12 do FIQ acima; nada mais a fazer aqui.
        } else {
            // Mantemos user_r8_12 sincronizado com active.
            self.user_r8_12.copy_from_slice(&self.active[8..13]);
        }
    }

    fn load_bank(&mut self, new: CpuMode) {
        match new {
            CpuMode::Fiq => {
                self.active[8..13].copy_from_slice(&self.fiq.r8_12);
                self.active[13] = self.fiq.sp;
                self.active[14] = self.fiq.lr;
            }
            CpuMode::Irq => {
                self.active[8..13].copy_from_slice(&self.user_r8_12);
                self.active[13] = self.irq.sp;
                self.active[14] = self.irq.lr;
            }
            CpuMode::Supervisor => {
                self.active[8..13].copy_from_slice(&self.user_r8_12);
                self.active[13] = self.svc.sp;
                self.active[14] = self.svc.lr;
            }
            CpuMode::Abort => {
                self.active[8..13].copy_from_slice(&self.user_r8_12);
                self.active[13] = self.abt.sp;
                self.active[14] = self.abt.lr;
            }
            CpuMode::Undefined => {
                self.active[8..13].copy_from_slice(&self.user_r8_12);
                self.active[13] = self.und.sp;
                self.active[14] = self.und.lr;
            }
            CpuMode::User | CpuMode::System => {
                self.active[8..13].copy_from_slice(&self.user_r8_12);
                self.active[13] = self.user_bank.sp;
                self.active[14] = self.user_bank.lr;
            }
        }
    }
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self::new()
    }
}
