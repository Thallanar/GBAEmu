//! Decoder e executor de instruções THUMB (16 bits).
//!
//! THUMB tem 19 formatos. Dispatcheamos por bits[15:13] e refinamos por bits
//! adicionais. Vários formatos reaproveitam helpers de [`super::alu`].

use crate::bus::Bus;

use super::alu::{
    adc_with_flags, add_with_flags, barrel_shift, sbc_with_flags, sub_with_flags, ShiftKind,
};
use super::condition::Condition;
use super::psr::PsrFlags;
use super::{Cpu, Handler};

pub fn execute(cpu: &mut Cpu, bus: &mut Bus, instr: u16) {
    decode(instr)(cpu, bus, instr as u32)
}

/// Resolve a instrução até o handler-folha. Decode hierárquico por prefixo
/// (bits altos), função **apenas dos bits** — cacheável por endereço de ROM
/// (ver `DecodeCache` no `cpu/mod.rs`).
pub(crate) fn decode(instr: u16) -> Handler {
    match instr >> 13 {
        0b000 => {
            if (instr >> 11) & 0b11 == 0b11 {
                h_fmt2
            } else {
                h_fmt1
            }
        }
        0b001 => h_fmt3,
        0b010 => {
            if (instr >> 10) & 0b111111 == 0b010000 {
                h_fmt4
            } else if (instr >> 10) & 0b111111 == 0b010001 {
                fmt5_hi_reg
            } else if (instr >> 11) & 0b11111 == 0b01001 {
                fmt6_pc_relative_load
            } else if (instr >> 9) & 0b1111001 == 0b0101001 {
                fmt8_load_store_sign_ext
            } else {
                fmt7_load_store_reg_offset
            }
        }
        0b011 => fmt9_load_store_imm_offset,
        0b100 => {
            if (instr >> 12) & 1 == 0 {
                fmt10_load_store_halfword
            } else {
                fmt11_sp_relative
            }
        }
        0b101 => {
            if (instr >> 12) & 1 == 0 {
                h_fmt12
            } else if (instr >> 8) & 0b1111 == 0b0000 {
                h_fmt13
            } else {
                fmt14_push_pop
            }
        }
        0b110 => {
            if (instr >> 12) & 1 == 0 {
                fmt15_multi_load_store
            } else if (instr >> 8) & 0b1111 == 0b1111 {
                fmt17_swi
            } else {
                h_fmt16
            }
        }
        0b111 => {
            if (instr >> 12) & 1 == 0 {
                h_fmt18
            } else {
                h_fmt19
            }
        }
        _ => unreachable!(),
    }
}

// Shims: assinatura uniforme de [Handler] pros formatos que não usam o bus.
fn h_fmt1(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt1_move_shifted(cpu, i)
}
fn h_fmt2(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt2_add_sub(cpu, i)
}
fn h_fmt3(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt3_mcas_imm(cpu, i)
}
fn h_fmt4(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt4_alu(cpu, i)
}
fn h_fmt12(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt12_load_address(cpu, i)
}
fn h_fmt13(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt13_add_to_sp(cpu, i)
}
fn h_fmt16(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt16_conditional_branch(cpu, i)
}
fn h_fmt18(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt18_unconditional_branch(cpu, i)
}
fn h_fmt19(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    fmt19_long_branch_link(cpu, i)
}

// ──────────────── Format 1: Move shifted register ────────────────
// 000 op(2) imm5(5) Rs(3) Rd(3)
fn fmt1_move_shifted(cpu: &mut Cpu, i: u32) {
    let op = (i >> 11) & 0b11;
    let imm5 = (i >> 6) & 0x1F;
    let rs = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;

    let kind = match op {
        0 => ShiftKind::Lsl,
        1 => ShiftKind::Lsr,
        2 => ShiftKind::Asr,
        _ => unreachable!(), // 3 seria ADD/SUB (formato 2)
    };
    let v = cpu.regs.get(rs);
    let out = barrel_shift(kind, v, imm5, cpu.cpsr.c(), true);
    cpu.regs.set(rd, out.value);
    cpu.cpsr.set_nz(out.value);
    cpu.cpsr.set_flag(PsrFlags::C, out.carry);
}

// ──────────────── Format 2: Add/subtract ────────────────
// 00011 I op Rn/imm3(3) Rs(3) Rd(3)
fn fmt2_add_sub(cpu: &mut Cpu, i: u32) {
    let imm = (i & (1 << 10)) != 0;
    let op = (i & (1 << 9)) != 0; // 0=ADD, 1=SUB
    let operand2 = if imm { (i >> 6) & 7 } else { cpu.regs.get(((i >> 6) & 7) as usize) };
    let rs = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;
    let a = cpu.regs.get(rs);

    let out = if op { sub_with_flags(a, operand2) } else { add_with_flags(a, operand2) };
    cpu.regs.set(rd, out.value);
    cpu.cpsr.set_nz(out.value);
    cpu.cpsr.set_flag(PsrFlags::C, out.carry);
    cpu.cpsr.set_flag(PsrFlags::V, out.overflow);
}

// ──────────────── Format 3: MOV/CMP/ADD/SUB imm ────────────────
// 001 op(2) Rd(3) imm8(8)
fn fmt3_mcas_imm(cpu: &mut Cpu, i: u32) {
    let op = (i >> 11) & 0b11;
    let rd = ((i >> 8) & 7) as usize;
    let imm = i & 0xFF;
    let a = cpu.regs.get(rd);

    match op {
        0 => { // MOV
            cpu.regs.set(rd, imm);
            cpu.cpsr.set_nz(imm);
        }
        1 => { // CMP
            let out = sub_with_flags(a, imm);
            cpu.cpsr.set_nz(out.value);
            cpu.cpsr.set_flag(PsrFlags::C, out.carry);
            cpu.cpsr.set_flag(PsrFlags::V, out.overflow);
        }
        2 => { // ADD
            let out = add_with_flags(a, imm);
            cpu.regs.set(rd, out.value);
            cpu.cpsr.set_nz(out.value);
            cpu.cpsr.set_flag(PsrFlags::C, out.carry);
            cpu.cpsr.set_flag(PsrFlags::V, out.overflow);
        }
        _ => { // SUB
            let out = sub_with_flags(a, imm);
            cpu.regs.set(rd, out.value);
            cpu.cpsr.set_nz(out.value);
            cpu.cpsr.set_flag(PsrFlags::C, out.carry);
            cpu.cpsr.set_flag(PsrFlags::V, out.overflow);
        }
    }
}

// ──────────────── Format 4: ALU operations ────────────────
// 010000 op(4) Rs(3) Rd(3)
fn fmt4_alu(cpu: &mut Cpu, i: u32) {
    let op = (i >> 6) & 0xF;
    let rs = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;
    let a = cpu.regs.get(rd);
    let b = cpu.regs.get(rs);
    let c_in = cpu.cpsr.c();

    let (write, value, set_cv) = match op {
        0x0 => (true,  a & b, None),                                       // AND
        0x1 => (true,  a ^ b, None),                                       // EOR
        0x2 => (true, shift_op(ShiftKind::Lsl, a, b & 0xFF, c_in, cpu), None), // LSL by reg
        0x3 => (true, shift_op(ShiftKind::Lsr, a, b & 0xFF, c_in, cpu), None), // LSR by reg
        0x4 => (true, shift_op(ShiftKind::Asr, a, b & 0xFF, c_in, cpu), None), // ASR by reg
        0x5 => { let o = adc_with_flags(a, b, c_in); (true, o.value, Some((o.carry, o.overflow))) } // ADC
        0x6 => { let o = sbc_with_flags(a, b, c_in); (true, o.value, Some((o.carry, o.overflow))) } // SBC
        0x7 => (true, shift_op(ShiftKind::Ror, a, b & 0xFF, c_in, cpu), None), // ROR by reg
        0x8 => (false, a & b, None),                                       // TST
        0x9 => { let o = sub_with_flags(0, b); (true, o.value, Some((o.carry, o.overflow))) } // NEG
        0xA => { let o = sub_with_flags(a, b); (false, o.value, Some((o.carry, o.overflow))) } // CMP
        0xB => { let o = add_with_flags(a, b); (false, o.value, Some((o.carry, o.overflow))) } // CMN
        0xC => (true,  a | b, None),                                       // ORR
        0xD => (true,  a.wrapping_mul(b), None),                           // MUL
        0xE => (true,  a & !b, None),                                      // BIC
        0xF => (true,  !b, None),                                          // MVN
        _ => unreachable!(),
    };

    if write {
        cpu.regs.set(rd, value);
    }
    cpu.cpsr.set_nz(value);
    if let Some((c, v)) = set_cv {
        cpu.cpsr.set_flag(PsrFlags::C, c);
        cpu.cpsr.set_flag(PsrFlags::V, v);
    }
}

fn shift_op(kind: ShiftKind, v: u32, amount: u32, c_in: bool, cpu: &mut Cpu) -> u32 {
    let out = barrel_shift(kind, v, amount, c_in, false);
    cpu.cpsr.set_flag(PsrFlags::C, out.carry);
    out.value
}

// ──────────────── Format 5: Hi register / BX ────────────────
// 010001 op(2) H1 H2 Rs(3) Rd(3)
fn fmt5_hi_reg(cpu: &mut Cpu, _bus: &mut Bus, i: u32) {
    let op = (i >> 8) & 0b11;
    let h1 = (i & (1 << 7)) != 0;
    let h2 = (i & (1 << 6)) != 0;
    let rs = (((i >> 3) & 7) as usize) | if h2 { 8 } else { 0 };
    let rd = ((i & 7) as usize) | if h1 { 8 } else { 0 };

    let b = cpu.regs.get(rs);
    let a = cpu.regs.get(rd);

    match op {
        0 => { // ADD (não atualiza flags)
            let v = a.wrapping_add(b);
            if rd == 15 { cpu.set_pc_thumb(v); } else { cpu.regs.set(rd, v); }
        }
        1 => { // CMP (atualiza flags)
            let o = sub_with_flags(a, b);
            cpu.cpsr.set_nz(o.value);
            cpu.cpsr.set_flag(PsrFlags::C, o.carry);
            cpu.cpsr.set_flag(PsrFlags::V, o.overflow);
        }
        2 => { // MOV (não atualiza flags)
            if rd == 15 { cpu.set_pc_thumb(b); } else { cpu.regs.set(rd, b); }
        }
        _ => { // BX
            let thumb = b & 1 != 0;
            cpu.cpsr.set_flag(PsrFlags::T, thumb);
            if thumb {
                cpu.set_pc_thumb(b & !1);
            } else {
                cpu.set_pc_arm(b & !3);
            }
        }
    }
}

// ──────────────── Format 6: PC-relative load ────────────────
// 01001 Rd(3) imm8(8)  →  Rd = mem[(PC & ~3) + imm8*4]
fn fmt6_pc_relative_load(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let rd = ((i >> 8) & 7) as usize;
    let imm = (i & 0xFF) * 4;
    let pc = cpu.regs.pc() & !3;
    let v = bus.read_u32(pc.wrapping_add(imm));
    cpu.regs.set(rd, v);
}

// ──────────────── Format 7: Load/store with register offset ────────────────
// 0101 L B 0 Ro(3) Rb(3) Rd(3)
fn fmt7_load_store_reg_offset(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let load = (i & (1 << 11)) != 0;
    let byte = (i & (1 << 10)) != 0;
    let ro = ((i >> 6) & 7) as usize;
    let rb = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;
    let addr = cpu.regs.get(rb).wrapping_add(cpu.regs.get(ro));

    if load {
        let v = if byte {
            bus.read_u8(addr) as u32
        } else {
            let aligned = addr & !3;
            let v = bus.read_u32(aligned);
            v.rotate_right((addr & 3) * 8)
        };
        cpu.regs.set(rd, v);
    } else {
        let v = cpu.regs.get(rd);
        if byte {
            bus.write_u8(addr, v as u8);
        } else {
            bus.write_u32(addr & !3, v);
        }
    }
}

// ──────────────── Format 8: Load/store sign-extended byte/halfword ────────────────
// 0101 H S 1 Ro(3) Rb(3) Rd(3)
fn fmt8_load_store_sign_ext(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let h = (i & (1 << 11)) != 0;
    let s = (i & (1 << 10)) != 0;
    let ro = ((i >> 6) & 7) as usize;
    let rb = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;
    let addr = cpu.regs.get(rb).wrapping_add(cpu.regs.get(ro));

    let v = match (s, h) {
        (false, false) => { // STRH
            let v = cpu.regs.get(rd);
            bus.write_u16(addr & !1, v as u16);
            return;
        }
        (false, true)  => { // LDRH
            let aligned = addr & !1;
            let v = bus.read_u16(aligned) as u32;
            if addr & 1 != 0 { v.rotate_right(8) } else { v }
        }
        (true, false)  => bus.read_u8(addr) as i8 as i32 as u32, // LDRSB
        (true, true)   => {
            if addr & 1 != 0 {
                bus.read_u8(addr) as i8 as i32 as u32
            } else {
                bus.read_u16(addr) as i16 as i32 as u32
            }
        }
    };
    cpu.regs.set(rd, v);
}

// ──────────────── Format 9: Load/store with immediate offset ────────────────
// 011 B L imm5(5) Rb(3) Rd(3)
fn fmt9_load_store_imm_offset(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let byte = (i & (1 << 12)) != 0;
    let load = (i & (1 << 11)) != 0;
    let imm5 = (i >> 6) & 0x1F;
    let rb = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;

    // Para word, offset é imm5*4. Para byte, imm5.
    let offset = if byte { imm5 } else { imm5 * 4 };
    let addr = cpu.regs.get(rb).wrapping_add(offset);

    if load {
        let v = if byte {
            bus.read_u8(addr) as u32
        } else {
            let aligned = addr & !3;
            let v = bus.read_u32(aligned);
            v.rotate_right((addr & 3) * 8)
        };
        cpu.regs.set(rd, v);
    } else {
        let v = cpu.regs.get(rd);
        if byte {
            bus.write_u8(addr, v as u8);
        } else {
            bus.write_u32(addr & !3, v);
        }
    }
}

// ──────────────── Format 10: Load/store halfword ────────────────
// 1000 L imm5(5) Rb(3) Rd(3)
fn fmt10_load_store_halfword(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let load = (i & (1 << 11)) != 0;
    let imm5 = (i >> 6) & 0x1F;
    let rb = ((i >> 3) & 7) as usize;
    let rd = (i & 7) as usize;
    let addr = cpu.regs.get(rb).wrapping_add(imm5 * 2);

    if load {
        let aligned = addr & !1;
        let v = bus.read_u16(aligned) as u32;
        cpu.regs.set(rd, if addr & 1 != 0 { v.rotate_right(8) } else { v });
    } else {
        bus.write_u16(addr & !1, cpu.regs.get(rd) as u16);
    }
}

// ──────────────── Format 11: SP-relative load/store ────────────────
// 1001 L Rd(3) imm8(8)
fn fmt11_sp_relative(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let load = (i & (1 << 11)) != 0;
    let rd = ((i >> 8) & 7) as usize;
    let offset = (i & 0xFF) * 4;
    let addr = cpu.regs.sp().wrapping_add(offset);

    if load {
        let v = bus.read_u32(addr & !3);
        cpu.regs.set(rd, v.rotate_right((addr & 3) * 8));
    } else {
        bus.write_u32(addr & !3, cpu.regs.get(rd));
    }
}

// ──────────────── Format 12: Load address ────────────────
// 1010 SP Rd(3) imm8(8)   →   Rd = (SP|PC) + imm8*4
fn fmt12_load_address(cpu: &mut Cpu, i: u32) {
    let use_sp = (i & (1 << 11)) != 0;
    let rd = ((i >> 8) & 7) as usize;
    let offset = (i & 0xFF) * 4;
    let base = if use_sp { cpu.regs.sp() } else { cpu.regs.pc() & !2 };
    cpu.regs.set(rd, base.wrapping_add(offset));
}

// ──────────────── Format 13: Add offset to SP ────────────────
// 10110000 S imm7(7)
fn fmt13_add_to_sp(cpu: &mut Cpu, i: u32) {
    let sub = (i & (1 << 7)) != 0;
    let offset = (i & 0x7F) * 4;
    let sp = cpu.regs.sp();
    let v = if sub { sp.wrapping_sub(offset) } else { sp.wrapping_add(offset) };
    cpu.regs.set(13, v);
}

// ──────────────── Format 14: Push/pop ────────────────
// 1011 L 10 R reg_list(8)   L: 0=push, 1=pop. R: extra register (LR no push, PC no pop)
fn fmt14_push_pop(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let load = (i & (1 << 11)) != 0;
    let extra = (i & (1 << 8)) != 0;
    let list = (i & 0xFF) as u8;

    if load {
        // POP: lê em ordem crescente de R0..R7, depois R15 se extra.
        let mut sp = cpu.regs.sp();
        for r in 0..8 {
            if list & (1 << r) != 0 {
                cpu.regs.set(r as usize, bus.read_u32(sp));
                sp = sp.wrapping_add(4);
            }
        }
        if extra {
            let pc = bus.read_u32(sp);
            sp = sp.wrapping_add(4);
            // Em ARMv4T, bit 0 do PC popado indica estado (mas em THUMB sempre fica THUMB).
            cpu.set_pc_thumb(pc & !1);
        }
        cpu.regs.set(13, sp);
    } else {
        // PUSH: ordem crescente em endereço decrescente — escreve do mais baixo
        // (R0) no endereço mais baixo (SP-4*count).
        let count = list.count_ones() + extra as u32;
        let mut sp = cpu.regs.sp().wrapping_sub(count * 4);
        let start = sp;
        for r in 0..8 {
            if list & (1 << r) != 0 {
                bus.write_u32(sp, cpu.regs.get(r as usize));
                sp = sp.wrapping_add(4);
            }
        }
        if extra {
            bus.write_u32(sp, cpu.regs.lr());
        }
        cpu.regs.set(13, start);
    }
}

// ──────────────── Format 15: Multiple load/store ────────────────
// 1100 L Rb(3) reg_list(8)
fn fmt15_multi_load_store(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    let load = (i & (1 << 11)) != 0;
    let rb = ((i >> 8) & 7) as usize;
    let list = (i & 0xFF) as u8;
    let mut addr = cpu.regs.get(rb);

    // Quirk ARMv4: lista vazia transfere R15 e adianta Rb em 0x40.
    if list == 0 {
        if load {
            let v = bus.read_u32(addr);
            cpu.set_pc_thumb(v);
        } else {
            bus.write_u32(addr, cpu.regs.pc().wrapping_add(2));
        }
        cpu.regs.set(rb, addr.wrapping_add(0x40));
        return;
    }

    let final_addr = addr.wrapping_add(list.count_ones() * 4);
    let lowest = list.trailing_zeros() as usize;

    for r in 0..8 {
        if list & (1 << r) == 0 {
            continue;
        }
        if load {
            cpu.regs.set(r as usize, bus.read_u32(addr));
        } else {
            // Quirk STM: base na lista e não é o menor → grava o valor com writeback.
            let v = if r as usize == rb && r as usize != lowest {
                final_addr
            } else {
                cpu.regs.get(r as usize)
            };
            bus.write_u32(addr, v);
        }
        addr = addr.wrapping_add(4);
    }
    // Writeback sempre, exceto se LDM e rb está na lista.
    if !(load && list & (1 << rb) != 0) {
        cpu.regs.set(rb, final_addr);
    }
}

// ──────────────── Format 16: Conditional branch ────────────────
// 1101 cond(4) offset(8 signed)*2
fn fmt16_conditional_branch(cpu: &mut Cpu, i: u32) {
    let cond = Condition::from_bits((i >> 8) & 0xF);
    if !cond.evaluate(cpu.cpsr) {
        return;
    }
    let raw = i & 0xFF;
    let signed = ((raw << 24) as i32) >> 24;
    let offset = signed.wrapping_mul(2);
    let target = cpu.regs.pc().wrapping_add(offset as u32);
    cpu.set_pc_thumb(target);
}

// ──────────────── Format 17: SWI ────────────────
fn fmt17_swi(cpu: &mut Cpu, bus: &mut Bus, i: u32) {
    use super::psr::CpuMode;
    // Com BIOS HLE, emulamos a função em Rust e continuamos na próxima instrução
    // (sem trocar modo/PC). Número da função: bits 7..0.
    if bus.hle_bios {
        super::bios::dispatch(cpu, bus, (i & 0xFF) as u8);
        return;
    }

    let return_addr = cpu.regs.pc().wrapping_sub(2);
    let old_cpsr = cpu.cpsr;
    cpu.cpsr.set_mode(CpuMode::Supervisor);
    cpu.cpsr.set_flag(PsrFlags::T, false);
    cpu.cpsr.set_flag(PsrFlags::I, true);
    cpu.regs.switch_mode(CpuMode::Supervisor);
    if let Some(idx) = CpuMode::Supervisor.spsr_index() {
        cpu.spsr[idx] = old_cpsr;
    }
    cpu.regs.set_lr(return_addr);
    cpu.set_pc_arm(0x0000_0008);
}

// ──────────────── Format 18: Unconditional branch ────────────────
// 11100 offset(11 signed)*2
fn fmt18_unconditional_branch(cpu: &mut Cpu, i: u32) {
    let raw = i & 0x7FF;
    let signed = ((raw << 21) as i32) >> 21;
    let offset = signed.wrapping_mul(2);
    let target = cpu.regs.pc().wrapping_add(offset as u32);
    cpu.set_pc_thumb(target);
}

// ──────────────── Format 19: Long branch with link ────────────────
// É CODIFICADO em DUAS instruções de 16 bits:
//   1ª: 11110 offset_hi(11)   →  LR = PC + (offset_hi << 12, sign-ext)
//   2ª: 11111 offset_lo(11)   →  PC = LR + offset_lo*2; LR = old_PC | 1
fn fmt19_long_branch_link(cpu: &mut Cpu, i: u32) {
    let h = (i >> 11) & 1;
    if h == 0 {
        // primeira metade: ajusta LR.
        let raw = i & 0x7FF;
        let signed = ((raw << 21) as i32) >> 21;
        let offset = signed.wrapping_shl(12);
        let lr = cpu.regs.pc().wrapping_add(offset as u32);
        cpu.regs.set_lr(lr);
    } else {
        // segunda metade: completa o branch.
        let raw = i & 0x7FF;
        let target = cpu.regs.lr().wrapping_add(raw * 2);
        // Endereço de retorno = instrução APÓS esse par (PC-2, porque PC=exec_pc+4).
        let return_addr = cpu.regs.pc().wrapping_sub(2) | 1;
        cpu.regs.set_lr(return_addr);
        cpu.set_pc_thumb(target);
    }
}
