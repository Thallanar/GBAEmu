//! Estrutura de topo que junta CPU, bus, PPU e APU.

use crate::bus::Bus;
use crate::cpu::Cpu;

/// Instância completa do emulador.
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Gba {
    pub cpu: Cpu,
    pub bus: Bus,
    /// Ciclos já decorridos mas ainda não entregues à PPU (batch por evento).
    /// Reconstruído na carga de save state (`skip`): zero força um tick imediato
    /// no próximo `step`, que recalcula a contagem — sempre seguro.
    #[cfg_attr(feature = "save-states", serde(skip))]
    ppu_pending: u32,
    /// Ciclos que faltam até o próximo evento de fase da PPU. Quando chega a ≤0,
    /// chamamos [`Ppu::tick`] com `ppu_pending` acumulado de uma vez.
    #[cfg_attr(feature = "save-states", serde(skip))]
    ppu_countdown: i64,
}

impl Gba {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(),
            ppu_pending: 0,
            ppu_countdown: 0,
        }
    }

    /// Carrega uma ROM na cartridge.
    pub fn load_rom(&mut self, rom: Vec<u8>) {
        self.bus.cartridge.load(rom);
        // ROM nova ⇒ cache de decode novo, dimensionado pra ela.
        self.cpu.cache = crate::cpu::DecodeCache::sized(self.bus.cartridge.rom.len());
    }

    /// "Power-cycle": reinicia CPU, bus e periféricos do zero, mas **preserva o
    /// cartucho** (ROM + Flash). Equivale a desligar e ligar o console — o save
    /// na memória de backup sobrevive. É o primitivo usado pelo Shiny Hunter
    /// para o soft-reset entre tentativas.
    pub fn reset(&mut self) {
        // Tira o cartucho fora antes de recriar o bus, depois devolve. O cache
        // de decode segue o mesmo princípio: deriva só da ROM, que não muda no
        // power-cycle — preservá-lo mantém o soft-reset do hunter aquecido.
        let cartridge = std::mem::take(&mut self.bus.cartridge);
        let cache = std::mem::take(&mut self.cpu.cache);
        self.bus = Bus::new();
        self.bus.cartridge = cartridge;
        self.cpu = Cpu::new();
        self.cpu.cache = cache;
        self.cpu.setup_direct_boot();
        self.cpu.regs.set_pc(0x0800_0000);
        self.ppu_pending = 0;
        self.ppu_countdown = 0;
    }

    /// Executa uma única instrução. Retorna ciclos consumidos.
    /// Após cada instrução, avança timers + PPU e propaga IRQs.
    pub fn step(&mut self) -> u32 {
        let cycles = self.cpu.step(&mut self.bus);

        self.bus.apu.tick(cycles);
        // Timers em batch: só ticam de fato no próximo overflow (e fazem
        // catch-up nos acessos a registrador). O Direct Sound e as IRQs de timer
        // são tratados dentro do flush, então não há mais `timer_irqs` aqui.
        self.bus.step_timers(cycles);
        self.refill_sound_fifos();

        // PPU em batch: entre dois eventos de fase a PPU não muda nada
        // observável, então só a chamamos quando a contagem regressiva
        // (`ppu_countdown`) zera, acumulando os ciclos em `ppu_pending`. É
        // ciclo-exato (ver `Ppu::cycles_until_event`) e tira ~280k chamadas/frame.
        self.ppu_pending += cycles;
        self.ppu_countdown -= cycles as i64;
        let mut ppu_irqs = 0;
        if self.ppu_countdown <= 0 {
            let pending = self.ppu_pending;
            self.ppu_pending = 0;
            // Borrows disjuntos: ppu, vram e palette são campos distintos do bus.
            let ppu_result = {
                let bus = &mut self.bus;
                bus.ppu.tick(pending, &*bus.vram, &*bus.palette, &*bus.oam)
            };
            self.ppu_countdown = self.bus.ppu.cycles_until_event() as i64;
            ppu_irqs = ppu_result.irqs;

            // DMA disparado por VBlank/HBlank (a transferência precisa do bus
            // inteiro, então roda fora do borrow da PPU acima).
            if ppu_result.entered_vblank {
                self.bus.run_dma_timing(crate::dma::Timing::VBlank);
            }
            if ppu_result.entered_hblank {
                self.bus.run_dma_timing(crate::dma::Timing::HBlank);
            }
        }

        let key_irq = if self.bus.io.joypad.irq_pending() {
            crate::io::irq_bits::KEYPAD
        } else {
            0
        };

        let all = ppu_irqs | key_irq;
        if all != 0 {
            self.bus.io.raise(all);
        }
        cycles
    }

    /// Reabastece as FIFOs do Direct Sound via DMA "special". DMA1/DMA2 em modo
    /// special com destino numa FIFO transferem 4 words (16 amostras) sempre que
    /// a FIFO cai à metade. Origem incrementa, destino é fixo, e o canal repete
    /// (não desabilita).
    fn refill_sound_fifos(&mut self) {
        for ch in 1..=2usize {
            let c = self.bus.dma.channels[ch];
            if !c.enabled() || c.timing() != crate::dma::Timing::Special {
                continue;
            }
            let fifo = match c.int_dst {
                0x0400_00A0 => 0,
                0x0400_00A4 => 1,
                _ => continue,
            };
            if !self.bus.apu.fifo_needs_refill(fifo) {
                continue;
            }
            let mut src = c.int_src;
            let dst = c.int_dst;
            for _ in 0..4 {
                let w = self.bus.read_u32(src);
                self.bus.write_u32(dst, w); // roteado para a FIFO do APU
                src = src.wrapping_add(4);
            }
            self.bus.dma.channels[ch].int_src = src;
        }
    }

    /// Executa um frame inteiro (~280896 ciclos). Placeholder.
    pub fn run_frame(&mut self) {
        let mut cycles = 0u32;
        while cycles < 280_896 {
            cycles += self.step();
        }
    }

    /// Executa ~`target` ciclos (para no fim da instrução que cruzar a meta) e
    /// devolve quantos rodaram de fato. É o passo do lockstep do link (Fase
    /// Link, etapa b): as instâncias avançam em quanta e sincronizam o serial
    /// na fronteira de cada um.
    pub fn run_cycles(&mut self, target: u32) -> u32 {
        let mut cycles = 0u32;
        while cycles < target {
            cycles += self.step();
        }
        cycles
    }

    /// Roda até o jogo ARMAR uma transferência multi-player (o master escreve
    /// START e o SIO marca `pending_start`) ou até gastar `target` ciclos —
    /// o que vier primeiro. Devolve `(ciclos rodados, armou?)`.
    ///
    /// É o primitivo do link event-driven (Fase Link, etapa c): em vez de
    /// sincronizar numa fronteira de quantum fixa, o master para EXATAMENTE no
    /// instante em que o jogo dispara cada transferência (Timer3 no
    /// CONN_ESTABLISHED pede mais de 8/frame) e troca pela rede ali — sem teto
    /// de transferências por frame. Verifica antes de cada passo e na entrada,
    /// pra pegar um start já armado por uma escrita anterior.
    pub fn run_until_transfer(&mut self, target: u32) -> (u32, bool) {
        if self.bus.io.sio.link.pending_start {
            return (0, true);
        }
        let mut cycles = 0u32;
        while cycles < target {
            cycles += self.step();
            if self.bus.io.sio.link.pending_start {
                return (cycles, true);
            }
        }
        (cycles, false)
    }

    // ─────────────────────────── Link (etapa b) ───────────────────────────
    // O host (app desktop) dirige a sessão: configura o link, espelha o
    // "pronto" do parceiro e aplica a troca decidida na fronteira do quantum.
    // O core só guarda estado e levanta a IRQ serial — rede é problema do app.

    /// Liga/desliga a sessão de link. `id` 0 = parent (gera o clock).
    pub fn link_configure(&mut self, active: bool, id: u8) {
        self.bus.io.sio.link.active = active;
        self.bus.io.sio.link.id = id;
        self.bus.io.sio.link.partner_ready = false;
    }

    /// Espelha o "pronto" (modo multi-player ativo) do parceiro — vira o bit
    /// SD que o jogo local lê no SIOCNT.
    pub fn link_set_partner_ready(&mut self, ready: bool) {
        self.bus.io.sio.link.partner_ready = ready;
    }

    /// Estado local pro quantum: (em multi-player?, start pendente?, valor a enviar).
    pub fn link_status(&self) -> (bool, bool, u16) {
        let sio = &self.bus.io.sio;
        (sio.in_multi(), sio.link.pending_start, sio.send_value())
    }

    /// Registradores seriais crus (pro trace de depuração do link): (siocnt
    /// guardado, siomlt_send, rcnt). Não aplica a máscara de estado-de-linha
    /// da leitura — é o que o jogo escreveu.
    pub fn link_regs(&self) -> (u16, u16, u16) {
        let sio = &self.bus.io.sio;
        (sio.siocnt, sio.siomlt_send, sio.rcnt)
    }

    /// Estado de interrupção (pro trace): (IE, IME ligado?, IF). Diz se a IRQ
    /// serial (bit 7) está armada e se está pendente/sendo atendida.
    pub fn link_irq_state(&self) -> (u16, bool, u16) {
        let io = &self.bus.io;
        (io.ie, io.ime, io.iflag)
    }

    /// Início da transferência multi-player: acende o bit busy (SIOCNT bit 7)
    /// em TODOS — no HW o clock do master seta busy em cada GBA. É o que um
    /// escravo poll-based espera ver (busy→clear) pra detectar a transferência;
    /// sem isso ele fica cego. Marca a pendência como comprometida.
    pub fn link_begin_transfer(&mut self) {
        self.bus.io.sio.begin_transfer();
    }

    /// Conclui a transferência multi-player (um quantum após o início, pra o
    /// busy ficar visível): grava os dados na ordem dos IDs, apaga o busy e
    /// levanta a IRQ serial se habilitada. No-op fora do modo multi-player.
    pub fn link_complete_multi(&mut self, data: [u16; 4]) {
        if self.bus.io.sio.complete_multi(data) {
            self.bus.io.raise(crate::io::irq_bits::SERIAL);
        }
    }
}

impl Default for Gba {
    fn default() -> Self {
        Self::new()
    }
}

// ───────────────────────────── Save states ──────────────────────────────────

/// Versão do formato de save state. Subir quando o layout serializado mudar de
/// forma incompatível (recusamos estados de versão diferente em vez de carregar
/// lixo).
#[cfg(feature = "save-states")]
const STATE_VERSION: u32 = 1;
/// Assinatura mágica no início do arquivo ("AuroRA STAte").
#[cfg(feature = "save-states")]
const STATE_MAGIC: &[u8; 4] = b"ASTA";

/// Erros ao carregar um save state.
#[cfg(feature = "save-states")]
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("arquivo de save state inválido (assinatura ausente)")]
    BadMagic,
    #[error("versão de save state incompatível: {0} (esperado {STATE_VERSION})")]
    BadVersion(u32),
    #[error("save state é de outro jogo (code do estado != ROM atual)")]
    WrongGame,
    #[error("falha ao desserializar o estado: {0}")]
    Decode(#[from] bincode::Error),
}

#[cfg(feature = "save-states")]
impl Gba {
    /// Serializa o estado completo do emulador num blob portátil. O cabeçalho
    /// (`ASTA` + versão + game code) permite validar o arquivo e impedir que um
    /// estado seja carregado por cima de um jogo diferente. A ROM e a BIOS **não**
    /// vão no estado (já estão carregadas; ver `#[serde(skip)]`).
    pub fn save_state(&self) -> Vec<u8> {
        let body = bincode::serialize(self).expect("serialização de Gba não deve falhar");
        let code = self.bus.cartridge.game_code();
        let code_bytes = code.as_bytes();

        let mut out = Vec::with_capacity(body.len() + 16);
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&STATE_VERSION.to_le_bytes());
        out.push(code_bytes.len() as u8);
        out.extend_from_slice(code_bytes);
        out.extend_from_slice(&body);
        out
    }

    /// Restaura o estado a partir de um blob de [`Gba::save_state`]. Valida a
    /// assinatura, a versão e que o jogo bate com a ROM atual. A ROM e a BIOS
    /// vivas são preservadas (o estado não as carrega); o framebuffer é zerado e
    /// repintado no próximo frame.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let mut p = 0usize;
        let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
            let s = bytes.get(*p..*p + n)?;
            *p += n;
            Some(s)
        };

        if take(&mut p, 4) != Some(STATE_MAGIC.as_slice()) {
            return Err(StateError::BadMagic);
        }
        let ver = u32::from_le_bytes(take(&mut p, 4).ok_or(StateError::BadMagic)?.try_into().unwrap());
        if ver != STATE_VERSION {
            return Err(StateError::BadVersion(ver));
        }
        let code_len = *take(&mut p, 1).ok_or(StateError::BadMagic)?.first().unwrap() as usize;
        let code = take(&mut p, code_len).ok_or(StateError::BadMagic)?;
        if code != self.bus.cartridge.game_code().as_bytes() {
            return Err(StateError::WrongGame);
        }

        let mut restored: Gba = bincode::deserialize(&bytes[p..])?;
        // A ROM e a BIOS foram puladas na serialização (campos `skip`): devolve as
        // vivas pro estado restaurado antes de assumi-lo.
        restored.bus.cartridge.rom = std::mem::take(&mut self.bus.cartridge.rom);
        restored.bus.bios = std::mem::take(&mut self.bus.bios);
        *self = restored;
        Ok(())
    }
}

#[cfg(all(test, feature = "save-states"))]
mod state_tests {
    use super::*;

    /// ROM sintética: game code "TEST" no offset 0xAC e o marcador "SRAM_V" pra
    /// detectar um save SRAM de 32 KB (sem precisar de uma ROM real no disco).
    fn fake_rom(code: &[u8; 4]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[0xAC..0xB0].copy_from_slice(code);
        rom[0x100..0x106].copy_from_slice(b"SRAM_V");
        rom
    }

    fn gba_with_rom(code: &[u8; 4]) -> Gba {
        let mut gba = Gba::new();
        gba.load_rom(fake_rom(code));
        gba
    }

    #[test]
    fn round_trip_restores_state() {
        let mut gba = gba_with_rom(b"TEST");
        // Semeia estado distinto em CPU, RAMs e memória de save.
        gba.cpu.regs.set(3, 0xDEAD_BEEF);
        gba.bus.ewram[0x1234] = 0x42;
        gba.bus.iwram[0x0010] = 0x99;
        gba.bus.ppu.dispcnt = 0x1234;
        gba.bus.io.ie = 0xABCD;
        gba.bus.write_u8(0x0E00_0050, 0x7E); // SRAM

        let snapshot = gba.save_state();

        // Bagunça tudo depois de salvar.
        gba.cpu.regs.set(3, 0);
        gba.bus.ewram[0x1234] = 0;
        gba.bus.iwram[0x0010] = 0;
        gba.bus.ppu.dispcnt = 0;
        gba.bus.io.ie = 0;
        gba.bus.write_u8(0x0E00_0050, 0);

        gba.load_state(&snapshot).expect("load deve funcionar");

        assert_eq!(gba.cpu.regs.get(3), 0xDEAD_BEEF);
        assert_eq!(gba.bus.ewram[0x1234], 0x42);
        assert_eq!(gba.bus.iwram[0x0010], 0x99);
        assert_eq!(gba.bus.ppu.dispcnt, 0x1234);
        assert_eq!(gba.bus.io.ie, 0xABCD);
        assert_eq!(gba.bus.read_u8(0x0E00_0050), 0x7E);
        // A ROM viva foi preservada (não vai no estado).
        assert_eq!(gba.bus.cartridge.game_code(), "TEST");
        assert!(!gba.bus.cartridge.rom.is_empty());
        // E a BIOS embutida continua lá (também pulada no estado).
        assert!(!gba.bus.bios.is_empty());
    }

    #[test]
    fn rejects_state_from_other_game() {
        let snapshot = gba_with_rom(b"AAAA").save_state();
        let mut other = gba_with_rom(b"BBBB");
        assert!(matches!(
            other.load_state(&snapshot),
            Err(StateError::WrongGame)
        ));
    }

    #[test]
    fn rejects_garbage() {
        let mut gba = gba_with_rom(b"TEST");
        assert!(matches!(
            gba.load_state(b"not a save state"),
            Err(StateError::BadMagic)
        ));
    }

    #[test]
    fn deterministic_run_matches_after_restore() {
        // Salvar, rodar, restaurar e rodar de novo deve dar o mesmo estado —
        // confirma que o snapshot captura tudo que influencia a emulação.
        let mut gba = gba_with_rom(b"TEST");
        for _ in 0..4 {
            gba.run_frame();
        }
        let snapshot = gba.save_state();
        let after_more = {
            let mut g = gba.save_state();
            let mut tmp: Gba = Gba::new();
            tmp.load_rom(fake_rom(b"TEST"));
            tmp.load_state(&g).unwrap();
            for _ in 0..4 {
                tmp.run_frame();
            }
            g = tmp.save_state();
            g
        };
        gba.load_state(&snapshot).unwrap();
        for _ in 0..4 {
            gba.run_frame();
        }
        assert_eq!(gba.save_state(), after_more);
    }
}
