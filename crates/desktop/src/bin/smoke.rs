//! Smoke test harness: roda ROMs de teste do jsmolka e reporta estatísticas
//! da CPU (instruções executadas, opcodes não implementados).
//!
//! Uso:
//!   cargo run --release -p auroragba-desktop --bin smoke -- <rom.gba> [ciclos]
//!
//! A intenção NÃO é validar correção (precisa de PPU para isso), e sim:
//!   - Garantir que a CPU não trava em opcode desconhecido
//!   - Coletar uma lista de instruções que ainda não decodificamos
//!   - Confirmar mistura de ARM/THUMB durante execução real

use std::path::PathBuf;

use auroragba_core::joypad::Button;
use auroragba_core::Gba;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path: PathBuf = args.next().expect("uso: smoke <rom.gba> [ciclos]").into();
    let cycles: u64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);

    let rom = std::fs::read(&path)?;
    println!("ROM: {} ({} bytes)", path.display(), rom.len());

    let mut gba = Gba::new();
    gba.load_rom(rom);

    // Direct boot: estado pós-BIOS + entrada na ROM (0x08000000). SWI por HLE.
    gba.cpu.setup_direct_boot();
    gba.cpu.regs.set_pc(0x0800_0000);

    // Trace de descarrilhamento: AURORA_TRACE=1 roda do boot e, no primeiro PC
    // que cai na região 0 acima da BIOS real (>= 0x4000), despeja as últimas
    // instruções — revela o branch que pulou pro endereço errado.
    if std::env::var("AURORA_TRACE").is_ok() {
        const CAP: usize = 260;
        let mut ring: std::collections::VecDeque<(u32, bool, u32, u32, u32, u32)> =
            std::collections::VecDeque::with_capacity(CAP);
        for _ in 0..60_000_000u64 {
            let pc = gba.cpu.regs.pc();
            let is_thumb = (gba.cpu.cpsr.0 >> 5) & 1 == 1;
            let instr = if is_thumb {
                gba.bus.read_u16(pc) as u32
            } else {
                gba.bus.read_u32(pc)
            };
            let sp = gba.cpu.regs.get(13);
            let lr = gba.cpu.regs.get(14);
            if ring.len() == CAP {
                ring.pop_front();
            }
            ring.push_back((pc, is_thumb, gba.cpu.cpsr.0, instr, sp, lr));

            // Derail: PC na região 0 mas além da BIOS real (16 KB).
            if (pc >> 24) & 0xF == 0 && pc >= 0x4000 {
                println!(
                    "\n──── DERAIL detectado em PC={pc:08X} ({} passos) ────",
                    ring.len()
                );
                println!("(pc / estado / mode / sp / lr / instr)");
                for (p, t, c, i, sp, lr) in &ring {
                    let st = if *t { "T" } else { "A" };
                    let mode = c & 0x1F;
                    if *t {
                        println!(
                            "  {p:08X} {st} m{mode:02X} sp={sp:08X} lr={lr:08X}  {:04X}",
                            *i as u16
                        );
                    } else {
                        println!("  {p:08X} {st} m{mode:02X} sp={sp:08X} lr={lr:08X}  {i:08X}");
                    }
                }
                return Ok(());
            }
            gba.step();
        }
        println!("Nenhum derail em 60M passos.");
        return Ok(());
    }

    println!("Rodando até {} instruções...", cycles);
    // AURORA_MASH: tapeia A pra avançar telas (bateria/intro/menus) e chegar
    // num frame interessante pra dump.
    let mash = std::env::var("AURORA_MASH").is_ok();
    let mut steps = 0u64;
    while steps < cycles {
        if mash {
            gba.bus
                .io
                .joypad
                .set_button(Button::A, (steps / 8).is_multiple_of(2));
        }
        gba.step();
        steps += 1;
    }

    // AURORA_VDUMP: despeja VRAM/paleta/OAM + registradores de BG pra análise.
    if let Ok(dir) = std::env::var("AURORA_VDUMP") {
        std::fs::write(format!("{dir}/vram.bin"), *gba.bus.vram)?;
        std::fs::write(format!("{dir}/pal.bin"), *gba.bus.palette)?;
        std::fs::write(format!("{dir}/oam.bin"), *gba.bus.oam)?;
        let p = &gba.bus.ppu;
        println!("DISPCNT={:04X}", p.dispcnt);
        for b in 0..4 {
            println!(
                "BG{b}: cnt={:04X} hofs={} vofs={}",
                p.bgcnt[b], p.bg_hofs[b], p.bg_vofs[b]
            );
        }
        println!("VRAM/pal/oam salvos em {dir}");
        return Ok(());
    }

    // Dump determinístico: se AURORA_DUMP setado, despeja o frame EXATAMENTE
    // aqui (no fim do loop principal) e sai, sem rodar o diagnóstico que mexeria
    // na tela.
    if let Ok(out) = std::env::var("AURORA_DUMP") {
        use std::io::Write;
        println!(
            "DISPCNT={:04X} BGxCNT={:04X} {:04X} {:04X} {:04X}",
            gba.bus.ppu.dispcnt,
            gba.bus.ppu.bgcnt[0],
            gba.bus.ppu.bgcnt[1],
            gba.bus.ppu.bgcnt[2],
            gba.bus.ppu.bgcnt[3]
        );
        let fb = &gba.bus.ppu.framebuffer;
        let mut f = std::fs::File::create(&out)?;
        write!(f, "P6\n{} {}\n255\n", 240, 160)?;
        for px in fb.chunks_exact(4) {
            f.write_all(&px[0..3])?;
        }
        println!("Framebuffer salvo em {out}");
        return Ok(());
    }

    let s = &gba.cpu.stats;
    println!("\n──── Estatísticas ────");
    println!("ARM executados:           {}", s.arm_executed);
    println!("THUMB executados:         {}", s.thumb_executed);
    println!("ARM não implementados:    {}", s.arm_unimplemented);
    println!("THUMB não implementados:  {}", s.thumb_unimplemented);

    if !s.recent_unimplemented.is_empty() {
        println!("\nPrimeiras instruções não implementadas:");
        for (pc, instr, thumb) in s.recent_unimplemented.iter().take(16) {
            if *thumb {
                println!("  THUMB @ {:08X} = {:04X}", pc, *instr as u16);
            } else {
                println!("  ARM   @ {:08X} = {:08X}", pc, instr);
            }
        }
    }

    println!("\nPC final: {:08X}", gba.cpu.regs.pc());

    // ───── Diagnóstico de boot: onde a CPU está presa? ─────
    // Amostra PCs ao longo de uma janela: conjunto + região + extremos.
    let regiao = |pc: u32| match (pc >> 24) & 0xF {
        0x0 => "BIOS",
        0x2 => "EWRAM",
        0x3 => "IWRAM",
        0x8..=0xD => "ROM",
        _ => "?",
    };
    let mut loop_pcs = std::collections::BTreeSet::new();
    let mut region_hist = std::collections::BTreeMap::<&str, u64>::new();
    let (mut pc_min, mut pc_max) = (u32::MAX, 0u32);
    for _ in 0..200_000 {
        let pc = gba.cpu.regs.pc();
        loop_pcs.insert(pc);
        *region_hist.entry(regiao(pc)).or_default() += 1;
        pc_min = pc_min.min(pc);
        pc_max = pc_max.max(pc);
        gba.step();
    }
    println!("\n──── Diagnóstico ────");
    println!(
        "PCs distintos em 200k passos: {}  (faixa {:08X}..{:08X})",
        loop_pcs.len(),
        pc_min,
        pc_max
    );
    print!("Tempo de execução por região:");
    for (r, c) in &region_hist {
        print!("  {r}={c}");
    }
    println!();
    if loop_pcs.len() <= 32 {
        print!("Loop nos PCs:");
        for pc in &loop_pcs {
            print!(" {:08X}[{}]", pc, regiao(*pc));
        }
        println!();
    }
    println!(
        "CPSR: modo={:?}  I(irq_disabled)={}  raw={:08X}",
        gba.cpu.cpsr.mode(),
        gba.cpu.cpsr.irq_disabled(),
        gba.cpu.cpsr.0
    );

    let dispcnt = gba.bus.ppu.dispcnt;
    println!(
        "DISPCNT={:04X}  forced_blank(bit7)={}  modo(bits0-2)={}",
        dispcnt,
        (dispcnt >> 7) & 1,
        dispcnt & 0b111
    );
    println!(
        "IME={}  IE={:04X}  IF={:04X}  (IE&IF)={:04X}  halted={}",
        gba.bus.io.ime,
        gba.bus.io.ie,
        gba.bus.io.iflag,
        gba.bus.io.ie & gba.bus.io.iflag,
        gba.cpu.halted
    );

    // Resumo do framebuffer: número de cores distintas (sanidade da renderização).
    let fb = &gba.bus.ppu.framebuffer;
    let mut colors = std::collections::HashSet::new();
    for px in fb.chunks_exact(4) {
        colors.insert([px[0], px[1], px[2]]);
    }
    println!("Framebuffer: {} cores distintas", colors.len());

    println!(
        "BGxCNT: {:04X} {:04X} {:04X} {:04X}",
        gba.bus.ppu.bgcnt[0], gba.bus.ppu.bgcnt[1], gba.bus.ppu.bgcnt[2], gba.bus.ppu.bgcnt[3]
    );

    // Dump opcional para PPM se AURORA_DUMP estiver setado.
    if let Ok(out) = std::env::var("AURORA_DUMP") {
        use std::io::Write;
        let mut f = std::fs::File::create(&out)?;
        write!(f, "P6\n{} {}\n255\n", 240, 160)?;
        for px in fb.chunks_exact(4) {
            f.write_all(&px[0..3])?;
        }
        println!("Framebuffer salvo em {out}");
    }
    Ok(())
}
