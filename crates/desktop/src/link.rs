//! Lockstep do link cable entre duas instâncias (Fase Link, etapa b).
//!
//! Modelo: as duas instâncias avançam em **quanta** de ciclos e sincronizam o
//! serial na fronteira de cada um — nenhuma fica mais de um quantum à frente
//! da outra (o `exchange` é bloqueante nos dois lados). A cada fronteira os
//! lados trocam uma mensagem pequena com:
//!   - `ready`: o jogo local está em modo multi-player (vira o SD do outro);
//!   - `start`: (só faz sentido vindo do parent) o jogo escreveu start neste
//!     quantum — a transferência acontece NESTA fronteira, nos dois lados,
//!     com os `send` desta mesma rodada (determinismo: ambos veem os mesmos
//!     dados no mesmo ponto da emulação);
//!   - `send`: o SIOMLT_SEND local.
//!
//! O parent (host TCP, ID 0) gera o clock, como no hardware. Parceiro não
//! pronto lê linha alta (0xFFFF), igual ao cabo de verdade. Latência de
//! transferência = até 1 quantum (~2 ms) — folgado pro protocolo de link do
//! Gen 3, que é todo em handshakes.
//!
//! Wire format (sem dependências): HELLO de 8 bytes (`AGBL` + versão + papel + 2 reservados) e mensagens de 8 bytes (`seq` u32 LE | flags u8 | `send` u16 LE | reservado u8).

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

use auroragba_core::Gba;

/// Ciclos por quantum: 1/8 de frame (~2,1 ms). Pequeno o bastante pro
/// handshake serial do Gen 3, grande o bastante pra ~480 trocas TCP/s não
/// pesarem em localhost/LAN.
pub const QUANTUM_CYCLES: u32 = 280_896 / 8;

const HELLO_MAGIC: &[u8; 4] = b"AGBL";
const PROTO_VERSION: u8 = 1;

const FLAG_READY: u8 = 1 << 0;
const FLAG_START: u8 = 1 << 1;

/// Mensagem trocada na fronteira de cada quantum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg {
    pub seq: u32,
    pub ready: bool,
    pub start: bool,
    pub send: u16,
}

impl Msg {
    fn to_wire(self) -> [u8; 8] {
        let mut w = [0u8; 8];
        w[..4].copy_from_slice(&self.seq.to_le_bytes());
        w[4] = ((self.ready as u8) * FLAG_READY) | ((self.start as u8) * FLAG_START);
        w[5..7].copy_from_slice(&self.send.to_le_bytes());
        w
    }

    fn from_wire(w: [u8; 8]) -> Msg {
        Msg {
            seq: u32::from_le_bytes(w[..4].try_into().unwrap()),
            ready: w[4] & FLAG_READY != 0,
            start: w[4] & FLAG_START != 0,
            send: u16::from_le_bytes(w[5..7].try_into().unwrap()),
        }
    }
}

/// Sessão de lockstep estabelecida (transporte + nosso lugar na mesa).
pub struct LinkSession {
    stream: TcpStream,
    /// 0 = parent (host TCP, gera o clock), 1 = child.
    pub id: u8,
    seq: u32,
}

impl LinkSession {
    /// Hospeda em `port` e espera o parceiro (bloqueante). Host = parent.
    pub fn host(port: u16) -> io::Result<LinkSession> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let (stream, _) = listener.accept()?;
        LinkSession::handshake(stream, 0)
    }

    /// Conecta no host `addr` ("ip:porta"). Quem entra é o child.
    pub fn join(addr: &str) -> io::Result<LinkSession> {
        let stream = TcpStream::connect(addr)?;
        LinkSession::handshake(stream, 1)
    }

    fn handshake(stream: TcpStream, id: u8) -> io::Result<LinkSession> {
        // Mensagens de 8 bytes a ~480 Hz: Nagle só atrapalha.
        stream.set_nodelay(true)?;
        let mut s = LinkSession { stream, id, seq: 0 };
        let mut hello = [0u8; 8];
        hello[..4].copy_from_slice(HELLO_MAGIC);
        hello[4] = PROTO_VERSION;
        hello[5] = id;
        s.stream.write_all(&hello)?;
        let mut peer = [0u8; 8];
        s.stream.read_exact(&mut peer)?;
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

    /// Troca de fronteira: envia a nossa, recebe a do parceiro (bloqueante —
    /// é isso que mantém as instâncias a ≤1 quantum de distância).
    fn exchange(&mut self, out: Msg) -> io::Result<Msg> {
        self.stream.write_all(&out.to_wire())?;
        let mut buf = [0u8; 8];
        self.stream.read_exact(&mut buf)?;
        let inc = Msg::from_wire(buf);
        if inc.seq != out.seq {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("lockstep fora de fase: esperava seq {}, veio {}", out.seq, inc.seq),
            ));
        }
        Ok(inc)
    }

    /// Roda um quantum da emulação e sincroniza o serial na fronteira.
    pub fn run_quantum(&mut self, gba: &mut Gba) -> io::Result<()> {
        gba.run_cycles(QUANTUM_CYCLES);

        let (ready, pending_start, send) = gba.link_status();
        let out = Msg {
            seq: self.seq,
            ready,
            start: self.id == 0 && pending_start,
            send,
        };
        let inc = self.exchange(out)?;
        self.seq = self.seq.wrapping_add(1);

        gba.link_set_partner_ready(inc.ready);

        // A transferência deste quantum (decidida pelo parent) acontece AGORA
        // nos dois lados, com os `send` desta rodada.
        let started = if self.id == 0 { out.start } else { inc.start };
        if started {
            let (parent_send, child_send, child_ready) = if self.id == 0 {
                (out.send, inc.send, inc.ready)
            } else {
                (inc.send, out.send, true) // se recebemos start, nós (child) participamos
            };
            // Child ausente/não pronto = linha alta, como no cabo real.
            let child = if child_ready { child_send } else { 0xFFFF };
            gba.link_complete_multi([parent_send, child, 0xFFFF, 0xFFFF]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auroragba_core::sio::{RCNT_ADDR, SIOCNT_ADDR, SIOMLT_SEND_ADDR, SIOMULTI0_ADDR};
    use std::net::TcpListener;
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
            let mut s = LinkSession::handshake(stream, 0).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 0);
            setup_multi(&mut gba, 0xAAAA);
            // Quantum 1: ambos só ficam prontos (SD do parent sobe).
            s.run_quantum(&mut gba).unwrap();
            // Jogo do parent dispara o start.
            gba.bus.write_u16(SIOCNT_ADDR, 0b10 << 12 | 1 << 14 | 1 << 7);
            // Quantum 2: a troca acontece na fronteira.
            s.run_quantum(&mut gba).unwrap();
            gba
        });

        let child = thread::spawn(move || {
            let mut s = LinkSession::join(&format!("127.0.0.1:{port}")).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 1);
            setup_multi(&mut gba, 0xBBBB);
            s.run_quantum(&mut gba).unwrap();
            s.run_quantum(&mut gba).unwrap();
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
            let mut s = LinkSession::handshake(stream, 0).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 0);
            setup_multi(&mut gba, 0x1234);
            gba.bus.write_u16(SIOCNT_ADDR, 0b10 << 12 | 1 << 14 | 1 << 7);
            s.run_quantum(&mut gba).unwrap();
            gba
        });

        let child = thread::spawn(move || {
            let mut s = LinkSession::join(&format!("127.0.0.1:{port}")).unwrap();
            let mut gba = make_gba();
            gba.link_configure(true, 1);
            // Child NÃO entra em multi (RCNT/SIOCNT default).
            s.run_quantum(&mut gba).unwrap();
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
        let m = Msg { seq: 0xDEAD_BEEF, ready: true, start: false, send: 0x55AA };
        assert_eq!(Msg::from_wire(m.to_wire()), m);
        let m = Msg { seq: 7, ready: false, start: true, send: 0 };
        assert_eq!(Msg::from_wire(m.to_wire()), m);
    }
}
