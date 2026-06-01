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

    // Jogos começam executando da ROM (0x08000000). Set PC manualmente,
    // já que ainda não temos BIOS HLE para fazer o boot real.
    gba.cpu.regs.set_pc(0x0800_0000);

    println!("Rodando até {} instruções...", cycles);
    let mut steps = 0u64;
    while steps < cycles {
        gba.step();
        steps += 1;
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
    Ok(())
}
