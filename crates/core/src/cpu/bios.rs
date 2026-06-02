//! BIOS HLE (High-Level Emulation).
//!
//! O GBA tem uma BIOS de 16 KB que faz duas coisas relevantes para nós:
//!   1. Responde a chamadas `SWI` (SoftReset, divisão, descompressão, etc.).
//!   2. Despacha IRQs: o vetor em 0x18 salva contexto e pula para o handler
//!      do jogo (ponteiro em 0x03007FFC).
//!
//! Como não podemos distribuir a BIOS oficial, emulamos o comportamento das
//! funções `SWI` diretamente em Rust ("HLE") e fornecemos um trampolim de IRQ
//! mínimo em assembly ARM embutido (ver [`builtin_bios`]).
//!
//! Referência: GBATEK, seção "BIOS Functions".

use crate::bus::Bus;

use super::psr::{Cpsr, CpuMode};
use super::Cpu;

/// Constrói uma imagem de BIOS de 16 KB contendo apenas o trampolim de IRQ
/// no vetor 0x18. Os demais vetores ficam zerados — os `SWI` são tratados
/// por HLE antes de chegarem ao vetor 0x08.
///
/// Trampolim (executado quando uma IRQ é aceita, PC=0x18):
/// ```text
/// 0x18: stmfd sp!, {r0-r3,r12,lr}   ; salva contexto na pilha IRQ
/// 0x1C: mov   r0, #0x04000000
/// 0x20: add   lr, pc, #0            ; lr = 0x28 (retorno após o handler)
/// 0x24: ldr   pc, [r0, #-4]         ; pula para [0x03FFFFFC] (handler do jogo)
/// 0x28: ldmfd sp!, {r0-r3,r12,lr}   ; restaura contexto
/// 0x2C: subs  pc, lr, #4            ; retorna e restaura CPSR do SPSR_irq
/// ```
pub fn builtin_bios() -> Vec<u8> {
    let mut bios = vec![0u8; 0x4000];
    const TRAMPOLINE: [u32; 6] = [
        0xE92D_500F, // stmfd sp!, {r0-r3,r12,lr}
        0xE3A0_0301, // mov   r0, #0x04000000
        0xE28F_E000, // add   lr, pc, #0
        0xE510_F004, // ldr   pc, [r0, #-4]
        0xE8BD_500F, // ldmfd sp!, {r0-r3,r12,lr}
        0xE25E_F004, // subs  pc, lr, #4
    ];
    for (i, word) in TRAMPOLINE.iter().enumerate() {
        let off = 0x18 + i * 4;
        bios[off..off + 4].copy_from_slice(&word.to_le_bytes());
    }
    bios
}

/// Despacha uma chamada `SWI` por HLE. `comment` é o número da função
/// (bits 23..16 em ARM, bits 7..0 em THUMB).
///
/// Diferente da entrada real em exceção, NÃO trocamos de modo nem desviamos
/// o PC: a instrução simplesmente "executa" seu efeito e o fluxo continua na
/// próxima instrução (o pipeline em `step` cuida do avanço do PC).
pub fn dispatch(cpu: &mut Cpu, bus: &mut Bus, comment: u8) {
    match comment {
        0x00 => soft_reset(cpu, bus),
        0x01 => register_ram_reset(cpu, bus),
        0x02 => halt(cpu),
        0x03 => halt(cpu), // Stop/Sleep — tratamos como Halt
        0x04 => intr_wait(cpu),
        0x05 => intr_wait(cpu), // VBlankIntrWait — ver nota em intr_wait
        0x06 => div(cpu),
        0x07 => div_arm(cpu),
        0x08 => sqrt(cpu),
        0x09 => arctan(cpu),
        0x0A => arctan2(cpu),
        0x0B => cpu_set(cpu, bus),
        0x0C => cpu_fast_set(cpu, bus),
        0x0D => cpu.regs.set(0, 0xBAAE_187F), // GetBiosChecksum (BIOS GBA)
        0x0E => bg_affine_set(cpu, bus),
        0x0F => obj_affine_set(cpu, bus),
        0x10 => bit_unpack(cpu, bus),
        0x11 => lz77_uncomp(cpu, bus), // Wram (escrita 8-bit)
        0x12 => lz77_uncomp(cpu, bus), // Vram (idêntico no nosso modelo de memória)
        0x13 => huff_uncomp(cpu, bus),
        0x14 => rl_uncomp(cpu, bus),            // Wram
        0x15 => rl_uncomp(cpu, bus),            // Vram
        0x16 => diff_unfilter(cpu, bus, false), // Diff8bitUnFilterWram
        0x17 => diff_unfilter(cpu, bus, false), // Diff8bitUnFilterVram
        0x18 => diff_unfilter(cpu, bus, true),  // Diff16bitUnFilter
        other => {
            log::warn!("BIOS SWI 0x{other:02X} não implementada (ignorada)");
        }
    }
}

// ───────────────────────── Reset / Halt ─────────────────────────

/// SWI 0x00 — SoftReset. Lê a flag em 0x03007FFA: 0 → retorna à ROM
/// (0x08000000), !=0 → retorna ao início da EWRAM (0x02000000). Limpa a
/// faixa de pilha 0x03007E00..0x03007FFF e reinicia os SPs.
fn soft_reset(cpu: &mut Cpu, bus: &mut Bus) {
    let flag = bus.read_u8(0x0300_7FFA);
    // Limpa os últimos 0x200 bytes da IWRAM (área de pilha/sistema da BIOS).
    for b in bus.iwram[0x7E00..0x8000].iter_mut() {
        *b = 0;
    }
    cpu.setup_direct_boot();
    let target = if flag == 0 { 0x0800_0000 } else { 0x0200_0000 };
    cpu.set_pc_arm(target);
}

/// SWI 0x01 — RegisterRamReset. `r0` é um bitmask das regiões a zerar.
fn register_ram_reset(cpu: &mut Cpu, bus: &mut Bus) {
    let flags = cpu.regs.get(0);
    if flags & 0x01 != 0 {
        bus.ewram.fill(0);
    }
    if flags & 0x02 != 0 {
        // IWRAM, exceto os últimos 0x200 bytes (pilha/sistema).
        bus.iwram[0..0x7E00].fill(0);
    }
    if flags & 0x04 != 0 {
        bus.palette.fill(0);
    }
    if flags & 0x08 != 0 {
        bus.vram.fill(0);
    }
    if flags & 0x10 != 0 {
        bus.oam.fill(0);
    }
    // RegisterRamReset sempre força blank em DISPCNT (bit 7).
    bus.write_u16(0x0400_0000, 0x0080);
}

/// SWI 0x02 — Halt. Para a CPU até a próxima IRQ habilitada (IE & IF).
fn halt(cpu: &mut Cpu) {
    cpu.halted = true;
}

/// SWI 0x04/0x05 — IntrWait / VBlankIntrWait.
///
/// HLE simplificado: tratamos como Halt — a CPU dorme até qualquer IRQ
/// habilitada disparar, o handler do jogo roda (via trampolim) e o fluxo
/// retorna. Para a esmagadora maioria dos jogos (que só deixam VBlank ligada
/// durante a espera) isto é indistinguível do comportamento real.
fn intr_wait(cpu: &mut Cpu) {
    cpu.halted = true;
}

// ───────────────────────── Aritmética ─────────────────────────

/// SWI 0x06 — Div. r0=numerador, r1=denominador (com sinal).
/// Saída: r0=quociente, r1=resto, r3=|quociente|.
fn div(cpu: &mut Cpu) {
    let num = cpu.regs.get(0) as i32;
    let den = cpu.regs.get(1) as i32;
    do_div(cpu, num, den);
}

/// SWI 0x07 — DivArm. Igual a Div mas com operandos trocados (r0=den, r1=num).
fn div_arm(cpu: &mut Cpu) {
    let den = cpu.regs.get(0) as i32;
    let num = cpu.regs.get(1) as i32;
    do_div(cpu, num, den);
}

fn do_div(cpu: &mut Cpu, num: i32, den: i32) {
    if den == 0 {
        // Hardware tem comportamento indefinido; evitamos pânico.
        return;
    }
    let quot = num.wrapping_div(den);
    let rem = num.wrapping_rem(den);
    cpu.regs.set(0, quot as u32);
    cpu.regs.set(1, rem as u32);
    cpu.regs.set(3, quot.unsigned_abs());
}

/// SWI 0x08 — Sqrt. r0 = isqrt(r0) (raiz quadrada inteira).
fn sqrt(cpu: &mut Cpu) {
    let v = cpu.regs.get(0);
    cpu.regs.set(0, (v as f64).sqrt() as u32 & 0xFFFF);
}

/// SWI 0x09 — ArcTan. Aproximação via `f64` (HLE não bit-exata).
/// Entrada: r0 = tan em ponto fixo 1.1.14 (s16). Saída: r0 = ângulo.
fn arctan(cpu: &mut Cpu) {
    let tan = cpu.regs.get(0) as i16 as f64 / 16384.0;
    let angle = tan.atan(); // -pi/2..pi/2
    let units = (angle / std::f64::consts::PI * 32768.0).round() as i32;
    cpu.regs.set(0, (units & 0xFFFF) as u32);
}

/// SWI 0x0A — ArcTan2. Aproximação via `f64`. Saída em 0x0000..0xFFFF
/// (0x4000 = 90°, 0x8000 = 180°, ...).
fn arctan2(cpu: &mut Cpu) {
    let x = cpu.regs.get(0) as i16 as f64;
    let y = cpu.regs.get(1) as i16 as f64;
    let angle = y.atan2(x); // -pi..pi
    let units = (angle / (2.0 * std::f64::consts::PI) * 65536.0).round() as i32;
    cpu.regs.set(0, (units & 0xFFFF) as u32);
}

// ───────────────────────── Cópia de memória ─────────────────────────

/// SWI 0x0B — CpuSet. r0=origem, r1=destino, r2=controle.
/// Bits 0-20: número de unidades; bit 24: fixed source (preenchimento);
/// bit 26: tamanho (0=16-bit, 1=32-bit).
fn cpu_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let mut dst = cpu.regs.get(1);
    let ctrl = cpu.regs.get(2);
    let count = ctrl & 0x000F_FFFF;
    let fill = ctrl & (1 << 24) != 0;
    let word = ctrl & (1 << 26) != 0;

    if word {
        let mut value = bus.read_u32(src);
        for _ in 0..count {
            if !fill {
                value = bus.read_u32(src);
                src = src.wrapping_add(4);
            }
            bus.write_u32(dst, value);
            dst = dst.wrapping_add(4);
        }
    } else {
        let mut value = bus.read_u16(src);
        for _ in 0..count {
            if !fill {
                value = bus.read_u16(src);
                src = src.wrapping_add(2);
            }
            bus.write_u16(dst, value);
            dst = dst.wrapping_add(2);
        }
    }
}

/// SWI 0x0C — CpuFastSet. Sempre 32-bit, em blocos de 8 words.
/// r0=origem, r1=destino, r2: bits 0-20 = nº de words; bit 24 = fill.
fn cpu_fast_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let mut dst = cpu.regs.get(1);
    let ctrl = cpu.regs.get(2);
    // Contagem é arredondada para múltiplo de 8 words.
    let count = (ctrl & 0x000F_FFFF).div_ceil(8) * 8;
    let fill = ctrl & (1 << 24) != 0;

    let mut value = bus.read_u32(src);
    for _ in 0..count {
        if !fill {
            value = bus.read_u32(src);
            src = src.wrapping_add(4);
        }
        bus.write_u32(dst, value);
        dst = dst.wrapping_add(4);
    }
}

// ───────────────────────── Matrizes afins ─────────────────────────

/// SWI 0x0E — BgAffineSet. Calcula os parâmetros PA/PB/PC/PD + dx/dy de
/// rotação/escala de background a partir de um struct de entrada.
fn bg_affine_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let mut dst = cpu.regs.get(1);
    let count = cpu.regs.get(2);

    for _ in 0..count {
        let cx = bus.read_u32(src) as i32; // centro X (1.19.8)
        let cy = bus.read_u32(src + 4) as i32; // centro Y (1.19.8)
        let dispx = bus.read_u16(src + 8) as i16 as i32; // tela X (1.7.8)
        let dispy = bus.read_u16(src + 10) as i16 as i32;
        let sx = bus.read_u16(src + 12) as i16 as i32; // escala X (1.7.8)
        let sy = bus.read_u16(src + 14) as i16 as i32;
        let angle = (bus.read_u16(src + 16) >> 8) as usize; // ângulo (bits 8-15)

        let (sin, cos) = (SIN_LUT[angle] as i32, SIN_LUT[(angle + 64) & 0xFF] as i32);

        let pa = (sx * cos) >> 14;
        let pb = -((sx * sin) >> 14);
        let pc = (sy * sin) >> 14;
        let pd = (sy * cos) >> 14;

        bus.write_u16(dst, pa as u16);
        bus.write_u16(dst + 2, pb as u16);
        bus.write_u16(dst + 4, pc as u16);
        bus.write_u16(dst + 6, pd as u16);

        let start_x = cx - pa * dispx - pb * dispy;
        let start_y = cy - pc * dispx - pd * dispy;
        bus.write_u32(dst + 8, start_x as u32);
        bus.write_u32(dst + 12, start_y as u32);

        src += 20;
        dst += 16;
    }
}

/// SWI 0x0F — ObjAffineSet. Igual ao BgAffineSet mas só produz a matriz
/// PA/PB/PC/PD, com `offset` configurável entre entradas de saída.
fn obj_affine_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let mut dst = cpu.regs.get(1);
    let count = cpu.regs.get(2);
    let offset = cpu.regs.get(3);

    for _ in 0..count {
        let sx = bus.read_u16(src) as i16 as i32;
        let sy = bus.read_u16(src + 2) as i16 as i32;
        let angle = (bus.read_u16(src + 4) >> 8) as usize;

        let (sin, cos) = (SIN_LUT[angle] as i32, SIN_LUT[(angle + 64) & 0xFF] as i32);

        let pa = (sx * cos) >> 14;
        let pb = -((sx * sin) >> 14);
        let pc = (sy * sin) >> 14;
        let pd = (sy * cos) >> 14;

        bus.write_u16(dst, pa as u16);
        bus.write_u16(dst + offset, pb as u16);
        bus.write_u16(dst + offset * 2, pc as u16);
        bus.write_u16(dst + offset * 3, pd as u16);

        src += 6;
        dst += offset * 4;
    }
}

/// Tabela de seno em ponto fixo 1.14 (256 entradas = 360°), `sin(i·2π/256)·16384`.
/// `cos(a) = sin(a + 64)` (64 entradas = quarto de período).
#[rustfmt::skip]
static SIN_LUT: [i16; 256] = [
    0, 402, 804, 1205, 1606, 2006, 2404, 2801, 3196, 3590, 3981, 4370, 4756, 5139, 5520, 5897,
    6270, 6639, 7005, 7366, 7723, 8076, 8423, 8765, 9102, 9434, 9760, 10080, 10394, 10702, 11003, 11297,
    11585, 11866, 12140, 12406, 12665, 12916, 13160, 13395, 13623, 13842, 14053, 14256, 14449, 14635, 14811, 14978,
    15137, 15286, 15426, 15557, 15679, 15791, 15893, 15986, 16069, 16143, 16207, 16261, 16305, 16340, 16364, 16379,
    16384, 16379, 16364, 16340, 16305, 16261, 16207, 16143, 16069, 15986, 15893, 15791, 15679, 15557, 15426, 15286,
    15137, 14978, 14811, 14635, 14449, 14256, 14053, 13842, 13623, 13395, 13160, 12916, 12665, 12406, 12140, 11866,
    11585, 11297, 11003, 10702, 10394, 10080, 9760, 9434, 9102, 8765, 8423, 8076, 7723, 7366, 7005, 6639,
    6270, 5897, 5520, 5139, 4756, 4370, 3981, 3590, 3196, 2801, 2404, 2006, 1606, 1205, 804, 402,
    0, -402, -804, -1205, -1606, -2006, -2404, -2801, -3196, -3590, -3981, -4370, -4756, -5139, -5520, -5897,
    -6270, -6639, -7005, -7366, -7723, -8076, -8423, -8765, -9102, -9434, -9760, -10080, -10394, -10702, -11003, -11297,
    -11585, -11866, -12140, -12406, -12665, -12916, -13160, -13395, -13623, -13842, -14053, -14256, -14449, -14635, -14811, -14978,
    -15137, -15286, -15426, -15557, -15679, -15791, -15893, -15986, -16069, -16143, -16207, -16261, -16305, -16340, -16364, -16379,
    -16384, -16379, -16364, -16340, -16305, -16261, -16207, -16143, -16069, -15986, -15893, -15791, -15679, -15557, -15426, -15286,
    -15137, -14978, -14811, -14635, -14449, -14256, -14053, -13842, -13623, -13395, -13160, -12916, -12665, -12406, -12140, -11866,
    -11585, -11297, -11003, -10702, -10394, -10080, -9760, -9434, -9102, -8765, -8423, -8076, -7723, -7366, -7005, -6639,
    -6270, -5897, -5520, -5139, -4756, -4370, -3981, -3590, -3196, -2801, -2404, -2006, -1606, -1205, -804, -402,
];

// ───────────────────────── Descompressão ─────────────────────────

/// Lê o header de 4 bytes comum aos formatos comprimidos e devolve o tamanho
/// (em bytes) dos dados descomprimidos. `src` é avançado para após o header.
fn read_comp_header(bus: &mut Bus, src: &mut u32) -> usize {
    let header = bus.read_u32(*src);
    *src += 4;
    (header >> 8) as usize
}

/// Escreve a saída descomprimida no destino em unidades de **16 bits**.
///
/// VRAM e paleta não aceitam escrita de byte: um STRB ali dispara o quirk de
/// duplicação (o byte vai pros dois lados do halfword), então escrever a saída
/// byte-a-byte corromperia os gráficos (cada par de bytes colapsaria no 2º). Por
/// isso a BIOS real — e nós — escrevemos a saída em halfwords. Em WRAM o efeito
/// é idêntico a escrever bytes.
fn write_output(bus: &mut Bus, dst: u32, out: &[u8]) {
    let mut i = 0;
    while i + 1 < out.len() {
        let hw = (out[i] as u16) | ((out[i + 1] as u16) << 8);
        bus.write_u16(dst + i as u32, hw);
        i += 2;
    }
    if i < out.len() {
        bus.write_u8(dst + i as u32, out[i]);
    }
}

/// SWI 0x11/0x12 — LZ77UnComp. r0=origem, r1=destino.
fn lz77_uncomp(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let dst = cpu.regs.get(1);
    let out_size = read_comp_header(bus, &mut src);

    let mut out: Vec<u8> = Vec::with_capacity(out_size);
    while out.len() < out_size {
        let flags = bus.read_u8(src);
        src += 1;
        for bit in 0..8 {
            if out.len() >= out_size {
                break;
            }
            if flags & (0x80 >> bit) == 0 {
                // Bloco não comprimido: 1 byte literal.
                out.push(bus.read_u8(src));
                src += 1;
            } else {
                // Bloco comprimido: 2 bytes → comprimento e deslocamento.
                let b0 = bus.read_u8(src) as usize;
                let b1 = bus.read_u8(src + 1) as usize;
                src += 2;
                let length = (b0 >> 4) + 3;
                let disp = ((b0 & 0x0F) << 8 | b1) + 1;
                for _ in 0..length {
                    if out.len() < disp {
                        break;
                    }
                    let byte = out[out.len() - disp];
                    out.push(byte);
                }
            }
        }
    }

    write_output(bus, dst, &out);
}

/// SWI 0x14/0x15 — RLUnComp (run-length).
fn rl_uncomp(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let dst = cpu.regs.get(1);
    let out_size = read_comp_header(bus, &mut src);

    let mut out: Vec<u8> = Vec::with_capacity(out_size);
    while out.len() < out_size {
        let flag = bus.read_u8(src);
        src += 1;
        if flag & 0x80 != 0 {
            // Run comprimido: (flag & 0x7F) + 3 cópias de 1 byte.
            let length = (flag & 0x7F) as usize + 3;
            let byte = bus.read_u8(src);
            src += 1;
            for _ in 0..length {
                out.push(byte);
            }
        } else {
            // Run não comprimido: (flag & 0x7F) + 1 bytes literais.
            let length = (flag & 0x7F) as usize + 1;
            for _ in 0..length {
                out.push(bus.read_u8(src));
                src += 1;
            }
        }
    }

    write_output(bus, dst, &out);
}

/// SWI 0x13 — HuffUnComp. Descomprime dados Huffman (bitstream 4/8 bits).
fn huff_uncomp(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let dst = cpu.regs.get(1);

    let header = bus.read_u32(src);
    let data_bits = header & 0x0F; // tamanho do símbolo em bits (4 ou 8)
    let out_size = (header >> 8) as usize;
    src += 4;

    // Árvore: primeiro byte = (tamanho da tabela em words)*2 + 1 → fim da tabela.
    let tree_base = src;
    let tree_size = (bus.read_u8(tree_base) as u32 + 1) * 2;
    let mut bitstream = tree_base + tree_size;

    let mut out: Vec<u8> = Vec::with_capacity(out_size);
    let mut pending: u32 = 0; // acumulador de nibbles/bytes para a saída
    let mut pending_bits = 0u32;

    let mut node_off = tree_base + 1; // nó raiz
    let mut node = bus.read_u8(node_off);

    while out.len() < out_size {
        let word = bus.read_u32(bitstream);
        bitstream += 4;
        for i in (0..32).rev() {
            let bit = (word >> i) & 1;
            let offset = ((node & 0x3F) as u32 + 1) * 2;
            // Endereço do próximo nó: alinha o nó atual e soma o offset.
            let base = (node_off & !1) + offset + bit;
            let next = bus.read_u8(base);
            let leaf_mask = if bit == 1 { 0x40 } else { 0x80 };
            if node & leaf_mask != 0 {
                // Folha: `next` é o dado.
                pending |= (next as u32) << pending_bits;
                pending_bits += data_bits;
                if pending_bits >= 8 {
                    out.push((pending & 0xFF) as u8);
                    pending >>= 8;
                    pending_bits -= 8;
                }
                node_off = tree_base + 1;
                node = bus.read_u8(node_off);
            } else {
                node_off = base;
                node = next;
            }
            if out.len() >= out_size {
                break;
            }
        }
    }

    write_output(bus, dst, &out);
}

/// SWI 0x10 — BitUnPack. Expande dados de 1/2/4/8 bits para 1/2/4/8/16/32 bits.
fn bit_unpack(cpu: &mut Cpu, bus: &mut Bus) {
    let mut src = cpu.regs.get(0);
    let dst = cpu.regs.get(1);
    let info = cpu.regs.get(2);

    let src_len = bus.read_u16(info) as u32; // bytes de entrada
    let src_width = bus.read_u8(info + 2) as u32; // bits por unidade de origem
    let dst_width = bus.read_u8(info + 3) as u32; // bits por unidade de destino
    let data = bus.read_u32(info + 4);
    let offset = data & 0x7FFF_FFFF;
    let zero_flag = data & 0x8000_0000 != 0;

    let mut out_bits: u64 = 0;
    let mut out_count = 0u32;
    let mut dst_addr = dst;
    let src_mask = (1u32 << src_width) - 1;

    let end = src + src_len;
    while src < end {
        let byte = bus.read_u8(src) as u32;
        src += 1;
        let units = 8 / src_width;
        for u in 0..units {
            let unit = (byte >> (u * src_width)) & src_mask;
            let value = if unit != 0 || zero_flag {
                unit + offset
            } else {
                0
            };
            out_bits |= (value as u64) << out_count;
            out_count += dst_width;
            if out_count >= 32 {
                bus.write_u32(dst_addr, out_bits as u32);
                dst_addr += 4;
                out_bits >>= 32;
                out_count -= 32;
            }
        }
    }
    if out_count > 0 {
        bus.write_u32(dst_addr, out_bits as u32);
    }
}

/// SWI 0x16/0x17/0x18 — Diff(8|16)bitUnFilter. Reconstrói uma série a partir
/// das diferenças sucessivas. `wide` seleciona unidades de 16 bits.
fn diff_unfilter(cpu: &mut Cpu, bus: &mut Bus, wide: bool) {
    let mut src = cpu.regs.get(0);
    let mut dst = cpu.regs.get(1);
    let out_size = read_comp_header(bus, &mut src);

    if wide {
        let mut acc: u16 = 0;
        let mut written = 0;
        while written < out_size {
            acc = acc.wrapping_add(bus.read_u16(src));
            src += 2;
            bus.write_u16(dst, acc);
            dst += 2;
            written += 2;
        }
    } else {
        let mut acc: u8 = 0;
        let mut out = Vec::with_capacity(out_size);
        for _ in 0..out_size {
            acc = acc.wrapping_add(bus.read_u8(src));
            src += 1;
            out.push(acc);
        }
        write_output(bus, dst, &out);
    }
}

// ───────────────────────── Direct boot ─────────────────────────

impl Cpu {
    /// Configura o estado pós-BIOS ("direct boot"): modo System, IRQ
    /// habilitado no CPSR, e os stack pointers canônicos por modo. Não mexe
    /// no PC — quem chama define para onde pular.
    pub fn setup_direct_boot(&mut self) {
        // SPs bancados (valores que a BIOS oficial deixa configurados).
        self.regs.switch_mode(CpuMode::Supervisor);
        self.regs.set(13, 0x0300_7FE0);
        self.regs.switch_mode(CpuMode::Irq);
        self.regs.set(13, 0x0300_7FA0);
        self.regs.switch_mode(CpuMode::System);
        self.regs.set(13, 0x0300_7F00);

        // CPSR: System mode, IRQ/FIQ habilitados (I=0/F=0), ARM. O master enable
        // (IME) começa desligado; o jogo o liga quando quiser receber IRQs.
        self.cpsr = Cpsr(CpuMode::System as u32);
        self.halted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    const SRC: u32 = 0x0200_0000;
    const DST: u32 = 0x0200_1000;

    /// Escreve `bytes` em `addr` na EWRAM.
    fn put(bus: &mut Bus, addr: u32, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            bus.write_u8(addr + i as u32, *b);
        }
    }

    fn setup(src_bytes: &[u8]) -> (Cpu, Bus) {
        let mut bus = Bus::new();
        let cpu = Cpu::new();
        put(&mut bus, SRC, src_bytes);
        (cpu, bus)
    }

    #[test]
    fn lz77_literal_and_backref() {
        // Saída esperada: "AAAA" (0x41 x4).
        // Header: tamanho=4, tipo=1 → (4<<8)|(1<<4) = 0x0410.
        // Stream: flag=0x40 (bit0 literal, bit1 comprimido), literal 0x41,
        // bloco comprimido (len=3, disp=1) → b0=0x00, b1=0x00.
        let mut data = vec![0x10, 0x04, 0x00, 0x00]; // header LE
        data.extend_from_slice(&[0x40, 0x41, 0x00, 0x00]);
        let (mut cpu, mut bus) = setup(&data);
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, DST);
        lz77_uncomp(&mut cpu, &mut bus);
        let out: Vec<u8> = (0..4).map(|i| bus.read_u8(DST + i)).collect();
        assert_eq!(out, vec![0x41, 0x41, 0x41, 0x41]);
    }

    #[test]
    fn rle_compressed_run() {
        // Saída esperada: "XXXXX" (0x58 x5).
        // Header: tamanho=5, tipo=3 → (5<<8)|(3<<4) = 0x0530.
        // Stream: flag=0x82 (comprimido, len=2+3=5), byte=0x58.
        let mut data = vec![0x30, 0x05, 0x00, 0x00];
        data.extend_from_slice(&[0x82, 0x58]);
        let (mut cpu, mut bus) = setup(&data);
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, DST);
        rl_uncomp(&mut cpu, &mut bus);
        let out: Vec<u8> = (0..5).map(|i| bus.read_u8(DST + i)).collect();
        assert_eq!(out, vec![0x58; 5]);
    }

    /// Regressão (glitch gráfico do Emerald): descompressão para a VRAM tem que
    /// preservar bytes distintos. Escrever a saída byte-a-byte dispara o quirk de
    /// STRB (duplica nos dois lados do halfword e sobrescreve), corrompendo os
    /// gráficos. `write_output` escreve em halfwords e evita isso.
    #[test]
    fn lz77_to_vram_preserves_distinct_bytes() {
        const VRAM: u32 = 0x0600_0000;
        // 4 literais distintos: 0x11, 0x22, 0x33, 0x44.
        let mut data = vec![0x10, 0x04, 0x00, 0x00]; // header: size=4, tipo LZ77
        data.extend_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44]); // flag=0 (todos literais)
        let (mut cpu, mut bus) = setup(&data);
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, VRAM);
        lz77_uncomp(&mut cpu, &mut bus);
        let out: Vec<u8> = (0..4).map(|i| bus.read_u8(VRAM + i)).collect();
        assert_eq!(
            out,
            vec![0x11, 0x22, 0x33, 0x44],
            "bytes corrompidos pelo quirk de STRB (byte-write em VRAM)"
        );
    }

    #[test]
    fn cpu_set_copy_halfwords() {
        let (mut cpu, mut bus) = setup(&[]);
        for i in 0..4u32 {
            bus.write_u16(SRC + i * 2, 0x1000 + i as u16);
        }
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, DST);
        cpu.regs.set(2, 4); // 4 unidades, 16-bit, sem fill
        cpu_set(&mut cpu, &mut bus);
        for i in 0..4u32 {
            assert_eq!(bus.read_u16(DST + i * 2), 0x1000 + i as u16);
        }
    }

    #[test]
    fn cpu_set_fill_word() {
        let (mut cpu, mut bus) = setup(&[]);
        bus.write_u32(SRC, 0xDEAD_BEEF);
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, DST);
        // count=3, fill (bit24), 32-bit (bit26).
        cpu.regs.set(2, 3 | (1 << 24) | (1 << 26));
        cpu_set(&mut cpu, &mut bus);
        for i in 0..3u32 {
            assert_eq!(bus.read_u32(DST + i * 4), 0xDEAD_BEEF);
        }
    }

    #[test]
    fn diff8_unfilter_accumulates() {
        // Header: tamanho=4, tipo=8. Deltas: 10, +5, -2, +1 → 10,15,13,14.
        let mut data = vec![0x00, 0x04, 0x00, 0x00];
        data.extend_from_slice(&[10, 5, 0xFE, 1]);
        let (mut cpu, mut bus) = setup(&data);
        cpu.regs.set(0, SRC);
        cpu.regs.set(1, DST);
        diff_unfilter(&mut cpu, &mut bus, false);
        let out: Vec<u8> = (0..4).map(|i| bus.read_u8(DST + i)).collect();
        assert_eq!(out, vec![10, 15, 13, 14]);
    }
}
