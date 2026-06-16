//! Cabo de link entre duas instâncias — protocolo + máquina de estados
//! **portáteis** (Fase Link, L1). Transporte-agnóstico: a sincronização vive
//! aqui atrás do trait [`Transport`], e cada frontend (desktop/TCP, Android/…)
//! pluga o seu meio sem tocar na lógica de link.
//!
//! Modelo: o parent (ID 0) é o master, como no hardware — ele gera o clock. Ele
//! roda a CPU até o jogo ARMAR uma transferência (escrever START) ou até o fim
//! do frame, e SÓ AÍ troca uma mensagem pela rede. Cada mensagem leva `delta` =
//! ciclos que o master avançou; o child roda exatamente esse `delta`, espelhando
//! o tempo do master. Assim **não há teto de transferências por frame**: o
//! handshake (1/VBlank) e o CONN_ESTABLISHED (dirigido pelo Timer3, bem mais
//! rápido) cabem do mesmo jeito — cada transferência do jogo vira uma troca pela
//! rede no instante exato em que acontece.
//!
//! O `exchange`/`recv` é bloqueante: o master escreve a rodada e lê a resposta;
//! o child lê a rodada e responde. Isso trava os dois a ≤1 rodada de distância.
//! Parceiro não pronto lê linha alta (0xFFFF), igual ao cabo de verdade. Os dois
//! lados completam a transferência com os `send` da MESMA rodada (amostrados
//! antes do handler de conclusão), então veem a mesma mesa no mesmo ponto.
//!
//! Wire format (sem dependências): HELLO de 8 bytes (`AGBL` + versão + papel +
//! 2 reservados) e mensagens de 12 bytes (`seq` u32 LE | `delta` u32 LE |
//! flags u8 | `send` u16 LE | reservado u8).

use std::io::{self, Write};

use auroragba_core::Gba;

/// Meio de transporte do link: troca de blocos de bytes ponto-a-ponto e
/// bloqueante, na ordem em que foram escritos. O TCP do desktop é um caso; o
/// trait abstrai pra um futuro frontend Android (mesmo crate, mesmo wire).
///
/// Há um blanket impl pra qualquer `io::Read + io::Write` (cobre `TcpStream`,
/// pipes em memória nos testes, etc.), então a maioria dos transportes não
/// precisa implementar nada à mão.
pub trait Transport {
    /// Escreve `buf` inteiro (ou falha).
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
    /// Lê exatamente `buf.len()` bytes (ou falha), bloqueando até completar.
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()>;
}

impl<T: io::Read + io::Write> Transport for T {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        io::Write::write_all(self, buf)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        io::Read::read_exact(self, buf)
    }
}

/// Destino do trace da sessão (a conversa serial completa, pra depurar o
/// protocolo de trade). É política do frontend decidir SE e PRA ONDE tracear —
/// o crate só escreve linhas aqui quando `Some`. `None` = zero I/O.
pub type TraceSink = Option<Box<dyn Write + Send>>;

/// Ciclos por frame do GBA — o teto de cada `run_frame`.
pub const FRAME_CYCLES: u32 = 280_896;

/// Fatia curta de ciclos rodada com o busy (SIOCNT bit 7) ainda aceso, ENTRE
/// armar a transferência e concluí-la. Dá uma janela observável pra um detector
/// poll-based ver a borda busy→clear. ~1/128 de frame (~16 µs): centenas de
/// iterações de poll, sem atrasar a IRQ serial do parceiro.
const BUSY_WINDOW_CYCLES: u32 = FRAME_CYCLES / 128;

const HELLO_MAGIC: &[u8; 4] = b"AGBL";
// Bump: o wire mudou (12 bytes, com delta+frame_end) e o protocolo virou
// master-clocked event-driven — incompatível com a v1 de lockstep por quantum.
const PROTO_VERSION: u8 = 2;

const FLAG_READY: u8 = 1 << 0;
const FLAG_START: u8 = 1 << 1;
const FLAG_FRAME_END: u8 = 1 << 2;

/// Mensagem de uma rodada do link event-driven. O master dita o ritmo:
///   - `delta`: ciclos que o master avançou desde a rodada anterior — o child
///     roda EXATAMENTE isso, espelhando o tempo do master (lockstep);
///   - `start`: o master armou uma transferência NESTA rodada (ele parou no
///     instante exato do START); os dois completam com os `send` desta rodada;
///   - `frame_end`: última rodada do frame do master — o child sai do loop;
///   - `ready`: o emissor está em modo multi-player (vira o SD do outro);
///   - `send`: o SIOMLT_SEND local, amostrado ANTES do handler de conclusão.
///
/// O child responde com `delta`/`start`/`frame_end` zerados (só `ready`/`send`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg {
    pub seq: u32,
    pub delta: u32,
    pub ready: bool,
    pub start: bool,
    pub frame_end: bool,
    pub send: u16,
}

impl Msg {
    fn to_wire(self) -> [u8; 12] {
        let mut w = [0u8; 12];
        w[..4].copy_from_slice(&self.seq.to_le_bytes());
        w[4..8].copy_from_slice(&self.delta.to_le_bytes());
        w[8] = ((self.ready as u8) * FLAG_READY)
            | ((self.start as u8) * FLAG_START)
            | ((self.frame_end as u8) * FLAG_FRAME_END);
        w[9..11].copy_from_slice(&self.send.to_le_bytes());
        w
    }

    fn from_wire(w: [u8; 12]) -> Msg {
        Msg {
            seq: u32::from_le_bytes(w[..4].try_into().unwrap()),
            delta: u32::from_le_bytes(w[4..8].try_into().unwrap()),
            ready: w[8] & FLAG_READY != 0,
            start: w[8] & FLAG_START != 0,
            frame_end: w[8] & FLAG_FRAME_END != 0,
            send: u16::from_le_bytes(w[9..11].try_into().unwrap()),
        }
    }
}

/// Sessão de link estabelecida (transporte + nosso lugar na mesa).
pub struct LinkSession<T: Transport> {
    transport: T,
    /// 0 = parent (gera o clock), 1 = child.
    pub id: u8,
    seq: u32,
    /// Último estado de "pronto" (local, parceiro) — pra logar só transições.
    last_ready: (bool, bool),
    /// Trace da sessão — a conversa serial completa pra depurar o protocolo de
    /// trade. `None` (o normal) = trace desligado, zero I/O. Quem decide o
    /// destino é o frontend (ver [`TraceSink`]).
    trace: TraceSink,
    /// Últimos registradores crus (siocnt, send, rcnt) — pra logar só mudanças.
    last_regs: (u16, u16, u16),
    /// Último estado de IRQ (ie, ime, iflag) — pra logar só mudanças.
    last_irq: (u16, bool, u16),
}

impl<T: Transport> LinkSession<T> {
    /// Estabelece a sessão sobre um `transport` já conectado, trocando o HELLO
    /// (magic + versão + papel). `id` = 0 (parent/master) ou 1 (child). O
    /// `trace` é opcional e definido pelo frontend (ver [`TraceSink`]).
    pub fn establish(transport: T, id: u8, trace: TraceSink) -> io::Result<LinkSession<T>> {
        let role = if id == 0 { "parent" } else { "child" };
        let mut s = LinkSession {
            transport,
            id,
            seq: 0,
            last_ready: (false, false),
            trace,
            last_regs: (0, 0, 0),
            last_irq: (0, false, 0),
        };
        s.trace(format_args!("# sessão de link iniciada (papel={role})"));
        let mut hello = [0u8; 8];
        hello[..4].copy_from_slice(HELLO_MAGIC);
        hello[4] = PROTO_VERSION;
        hello[5] = id;
        s.transport.write_all(&hello)?;
        let mut peer = [0u8; 8];
        s.transport.read_exact(&mut peer)?;
        if &peer[..4] != HELLO_MAGIC || peer[4] != PROTO_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "parceiro não é um AuroraGBA compatível",
            ));
        }
        if peer[5] == id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "os dois lados reivindicam o mesmo papel no link",
            ));
        }
        Ok(s)
    }

    /// Rodada do master: envia a nossa, recebe a resposta do child (bloqueante —
    /// trava os dois a ≤1 rodada de distância). Confere o `seq` ecoado.
    fn exchange(&mut self, out: Msg) -> io::Result<Msg> {
        self.transport.write_all(&out.to_wire())?;
        let inc = self.recv()?;
        if inc.seq != out.seq {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("link fora de fase: esperava seq {}, veio {}", out.seq, inc.seq),
            ));
        }
        Ok(inc)
    }

    /// Lê uma mensagem do parceiro (bloqueante).
    fn recv(&mut self) -> io::Result<Msg> {
        let mut buf = [0u8; 12];
        self.transport.read_exact(&mut buf)?;
        Ok(Msg::from_wire(buf))
    }

    /// Escreve uma mensagem pro parceiro.
    fn send_msg(&mut self, out: Msg) -> io::Result<()> {
        self.transport.write_all(&out.to_wire())
    }

    /// Roda um frame da emulação sincronizando o serial. O master dirige; o
    /// child espelha. Cada lado decide pelo `id`.
    pub fn run_frame(&mut self, gba: &mut Gba) -> io::Result<()> {
        if self.id == 0 {
            self.run_frame_master(gba)
        } else {
            self.run_frame_child(gba)
        }
    }

    /// Master (parent, ID 0): roda a CPU até o jogo armar uma transferência ou
    /// até o fim do frame, e troca pela rede NESSE instante. Repete até fechar
    /// o frame — quantas transferências o jogo pedir (sem teto por frame).
    fn run_frame_master(&mut self, gba: &mut Gba) -> io::Result<()> {
        let mut total = 0u32;
        loop {
            // Roda até o jogo escrever START (para no instante exato) ou gastar
            // o que falta do frame.
            let budget = FRAME_CYCLES.saturating_sub(total);
            let (ran, armed) = gba.run_until_transfer(budget);
            total += ran;
            let frame_end = total >= FRAME_CYCLES;

            self.trace_regs_irq(gba);

            // Valor a enviar amostrado AGORA — antes da conclusão (que roda o
            // handler e zera/consome o SIOMLT_SEND). Ver nota no child.
            let (ready, _pending, send) = gba.link_status();
            let out = Msg { seq: self.seq, delta: ran, ready, start: armed, frame_end, send };
            let inc = self.exchange(out)?;

            gba.link_set_partner_ready(inc.ready);
            self.trace_ready(out.ready, inc.ready);
            self.seq = self.seq.wrapping_add(1);

            if armed {
                // Child ausente/não pronto = linha alta, como no cabo real.
                let child = if inc.ready { inc.send } else { 0xFFFF };
                self.trace(format_args!(
                    "q{}: TROCA parent={send:04X} child={child:04X}",
                    out.seq
                ));
                // Acende o busy (idempotente — o jogo já escreveu START), deixa
                // uma janela curta visível e conclui (dados + IRQ serial). O
                // handler serial corre no `ran` da próxima rodada.
                gba.link_begin_transfer();
                gba.run_cycles(BUSY_WINDOW_CYCLES);
                gba.link_complete_multi([send, child, 0xFFFF, 0xFFFF]);
            }

            if frame_end {
                return Ok(());
            }
        }
    }

    /// Child (ID != 0): não gera clock. Espelha o avanço do master (roda o
    /// `delta` de cada rodada) e, quando o master sinaliza `start`, completa a
    /// transferência com o mesmo par de `send` da rodada.
    fn run_frame_child(&mut self, gba: &mut Gba) -> io::Result<()> {
        loop {
            let inc = self.recv()?;
            if inc.seq != self.seq {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("link fora de fase: esperava seq {}, veio {}", self.seq, inc.seq),
                ));
            }
            // Espelha o tempo do master.
            gba.run_cycles(inc.delta);

            self.trace_regs_irq(gba);

            // Nosso `send` amostrado ANTES do handler de conclusão (abaixo). No
            // HW o master lê o registrador do escravo no instante do início da
            // transferência, ANTES de o handler do escravo rodar; amostrar
            // depois pegava o 0000 pós-handler e derrubava o playerCount do
            // master (que precisa de SLAVE_HANDSHAKE em todos os slots).
            let (ready, _pending, send) = gba.link_status();
            let out = Msg {
                seq: self.seq,
                delta: 0,
                ready,
                start: false,
                frame_end: false,
                send,
            };
            self.send_msg(out)?;

            gba.link_set_partner_ready(inc.ready);
            self.trace_ready(out.ready, inc.ready);
            self.seq = self.seq.wrapping_add(1);

            if inc.start {
                let parent = inc.send;
                self.trace(format_args!(
                    "q{}: TROCA parent={parent:04X} child={send:04X}",
                    out.seq
                ));
                gba.link_begin_transfer();
                gba.run_cycles(BUSY_WINDOW_CYCLES);
                gba.link_complete_multi([parent, send, 0xFFFF, 0xFFFF]);
            }

            if inc.frame_end {
                return Ok(());
            }
        }
    }

    /// Loga os registradores seriais crus e o estado de IRQ, só nas mudanças
    /// (revela o jogo preparando valores e mexendo no controle).
    fn trace_regs_irq(&mut self, gba: &Gba) {
        if self.trace.is_none() {
            return; // sem trace: nem calcula os registradores
        }
        let seq = self.seq;
        let regs = gba.link_regs();
        if regs != self.last_regs {
            self.trace(format_args!(
                "q{seq}: regs siocnt={:04X} send={:04X} rcnt={:04X}",
                regs.0, regs.1, regs.2
            ));
            self.last_regs = regs;
        }
        // Só o que importa pro link: IE, IME e o BIT SERIAL do IF (o IF inteiro
        // muda todo frame por VBlank — ruído). Serial armada = IME & IE bit 7.
        let raw = gba.link_irq_state();
        let irq = (raw.0, raw.1, raw.2 & 0x80);
        if irq != self.last_irq {
            self.trace(format_args!(
                "q{seq}: irq ie={:04X} ime={} if_serial={} (serial_armada={})",
                irq.0,
                irq.1 as u8,
                (irq.2 != 0) as u8,
                irq.1 && (irq.0 & 0x80) != 0
            ));
            self.last_irq = irq;
        }
    }

    /// Loga transições do "pronto" (local, parceiro).
    fn trace_ready(&mut self, local: bool, partner: bool) {
        if (local, partner) != self.last_ready {
            let seq = self.seq;
            self.trace(format_args!("q{seq}: pronto local={local} parceiro={partner}"));
            self.last_ready = (local, partner);
        }
    }

    /// Anexa uma linha ao trace da sessão (no-op se o sink for `None`).
    fn trace(&mut self, args: std::fmt::Arguments) {
        if let Some(w) = &mut self.trace {
            let _ = writeln!(w, "{args}");
            let _ = w.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auroragba_core::sio::{RCNT_ADDR, SIOCNT_ADDR, SIOMLT_SEND_ADDR, SIOMULTI0_ADDR};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    fn r16(gba: &mut Gba, addr: u32) -> u16 {
        gba.bus.read_u16(addr)
    }

    /// Configura o jogo "na mão" (como o código do jogo faria): modo
    /// multi-player com IRQ habilitada e um valor pra enviar.
    fn setup_multi(gba: &mut Gba, send: u16) {
        gba.bus.write_u16(RCNT_ADDR, 0);
        gba.bus.write_u16(SIOMLT_SEND_ADDR, send);
        gba.bus.write_u16(SIOCNT_ADDR, 0b10 << 12 | 1 << 14);
    }

    /// ROM mínima válida: header com game code + um loop infinito (B .) em
    /// 0x08000000, pra CPU ter o que executar enquanto o teste dirige o SIO.
    fn make_gba() -> Gba {
        let mut rom = vec![0u8; 0x200];
        // B . (branch para si): EA FF FF FE little-endian = FE FF FF EA
        rom[0..4].copy_from_slice(&[0xFE, 0xFF, 0xFF, 0xEA]);
        rom[0xAC..0xB0].copy_from_slice(b"TEST");
        let mut gba = Gba::new();
        gba.load_rom(rom);
        gba.reset();
        gba
    }

    #[test]
    fn lockstep_troca_dados_pelo_tcp() {
        // Porta efêmera: bind explícito pra descobrir a porta antes do accept.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let parent = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut s = LinkSession::establish(stream, 0, None).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 0);
            setup_multi(&mut gba, 0xAAAA);
            // Jogo do parent dispara o start (pré-armado): o master para nesse
            // instante já na 1ª rodada do frame e troca pela rede.
            gba.bus.write_u16(SIOCNT_ADDR, 0b10 << 12 | 1 << 14 | 1 << 7);
            s.run_frame(&mut gba).unwrap();
            gba
        });

        let child = thread::spawn(move || {
            let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            stream.set_nodelay(true).unwrap();
            let mut s = LinkSession::establish(stream, 1, None).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 1);
            setup_multi(&mut gba, 0xBBBB);
            s.run_frame(&mut gba).unwrap();
            gba
        });

        let mut p = parent.join().unwrap();
        let mut c = child.join().unwrap();

        // Os dois lados viram a MESMA mesa: parent=0xAAAA, child=0xBBBB.
        for gba in [&mut p, &mut c] {
            assert_eq!(r16(gba, SIOMULTI0_ADDR), 0xAAAA);
            assert_eq!(r16(gba, SIOMULTI0_ADDR + 2), 0xBBBB);
            assert_eq!(r16(gba, SIOMULTI0_ADDR + 4), 0xFFFF);
            assert_eq!(r16(gba, SIOMULTI0_ADDR + 6), 0xFFFF);
            // Busy limpo + IRQ serial pedida (IF bit 7) nos dois lados.
            assert_eq!(r16(gba, SIOCNT_ADDR) & (1 << 7), 0);
            assert_ne!(r16(gba, 0x0400_0202) & (1 << 7), 0, "IF serial deveria subir");
        }
        // IDs da mesa: parent lê 0, child lê 1 (bits 4-5); SD=1 nos dois
        // (parceiro pronto); SI: parent=0, child=1.
        assert_eq!((r16(&mut p, SIOCNT_ADDR) >> 4) & 0b11, 0);
        assert_eq!((r16(&mut c, SIOCNT_ADDR) >> 4) & 0b11, 1);
        assert_ne!(r16(&mut p, SIOCNT_ADDR) & 0b1000, 0);
        assert_ne!(r16(&mut c, SIOCNT_ADDR) & 0b1000, 0);
        assert_eq!(r16(&mut p, SIOCNT_ADDR) & 0b0100, 0);
        assert_ne!(r16(&mut c, SIOCNT_ADDR) & 0b0100, 0);
    }

    #[test]
    fn start_com_parceiro_fora_do_multi_le_linha_alta() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let parent = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut s = LinkSession::establish(stream, 0, None).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 0);
            setup_multi(&mut gba, 0x1234);
            gba.bus.write_u16(SIOCNT_ADDR, 0b10 << 12 | 1 << 14 | 1 << 7);
            s.run_frame(&mut gba).unwrap();
            gba
        });

        let child = thread::spawn(move || {
            let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            stream.set_nodelay(true).unwrap();
            let mut s = LinkSession::establish(stream, 1, None).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 1);
            // Child NÃO entra em multi (RCNT/SIOCNT default).
            s.run_frame(&mut gba).unwrap();
            gba
        });

        let mut p = parent.join().unwrap();
        let _c = child.join().unwrap();

        // Parent completou lendo linha alta do child ausente; SD = 0.
        assert_eq!(r16(&mut p, SIOMULTI0_ADDR), 0x1234);
        assert_eq!(r16(&mut p, SIOMULTI0_ADDR + 2), 0xFFFF);
        assert_eq!(r16(&mut p, SIOCNT_ADDR) & (1 << 7), 0);
        assert_eq!(r16(&mut p, SIOCNT_ADDR) & 0b1000, 0, "SD deve ler 0");
    }

    #[test]
    fn wire_format_roundtrip() {
        let m = Msg {
            seq: 0xDEAD_BEEF,
            delta: 0x0004_4940,
            ready: true,
            start: false,
            frame_end: true,
            send: 0x55AA,
        };
        assert_eq!(Msg::from_wire(m.to_wire()), m);
        let m = Msg { seq: 7, delta: 0, ready: false, start: true, frame_end: false, send: 0 };
        assert_eq!(Msg::from_wire(m.to_wire()), m);
    }
}
