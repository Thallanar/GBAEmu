//! Bootstrap TCP do link cable no desktop (Fase Link, L1/L2a).
//!
//! O protocolo e a máquina de estados moram no crate portátil `auroragba_link`,
//! atrás do trait `Transport`. Aqui plugamos o transporte concreto do desktop —
//! `TcpStream` — a política de trace (variável de ambiente + arquivo em `/tmp`)
//! e a orquestração da conexão **em thread de fundo** ([`PendingLink`]), pra a
//! UI não congelar enquanto o `accept`/`connect` bloqueia esperando o parceiro.

use std::fs::File;
use std::io::{self, BufWriter};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use auroragba_link::TraceSink;

/// Variável de ambiente que liga o trace da sessão em `/tmp` (depuração do
/// protocolo de link). Sem ela, nenhum arquivo é aberto e o trace é no-op.
const TRACE_ENV: &str = "AURORAGBA_LINK_TRACE";

/// Porta TCP padrão do link.
pub const DEFAULT_PORT: u16 = 7777;

/// De quanto em quanto tempo a thread de conexão checa o cancelamento (e, no
/// caso do join, tenta de novo enquanto o host não sobe).
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Sessão de link do desktop: a portátil sobre um `TcpStream`.
pub type LinkSession = auroragba_link::LinkSession<TcpStream>;

/// Conexão de link em andamento numa thread de fundo — não bloqueia a UI.
///
/// `host`/`join` disparam a thread e devolvem na hora; a UI faz [`poll`] a cada
/// frame até a sessão ficar pronta (ou falhar). [`cancel`] aborta a espera.
///
/// [`poll`]: PendingLink::poll
/// [`cancel`]: PendingLink::cancel
pub struct PendingLink {
    rx: Receiver<io::Result<LinkSession>>,
    cancel: Arc<AtomicBool>,
    /// `true` = estamos hospedando (esperando alguém conectar); `false` =
    /// conectando num host. Só pra a UI escolher o texto certo.
    pub hosting: bool,
}

impl PendingLink {
    /// Hospeda em `port` numa thread; resolve quando um parceiro conecta. Host =
    /// parent (master), ID 0.
    pub fn host(port: u16) -> PendingLink {
        Self::spawn(true, move |cancel| connect_host(port, &cancel))
    }

    /// Conecta no host `addr` ("ip:porta") numa thread; tenta de novo enquanto o
    /// host não sobe (ou até cancelar). Quem entra é o child, ID 1.
    pub fn join(addr: String) -> PendingLink {
        Self::spawn(false, move |cancel| connect_join(&addr, &cancel))
    }

    fn spawn(
        hosting: bool,
        f: impl FnOnce(Arc<AtomicBool>) -> io::Result<LinkSession> + Send + 'static,
    ) -> PendingLink {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread_cancel = cancel.clone();
        thread::spawn(move || {
            let _ = tx.send(f(thread_cancel));
        });
        PendingLink { rx, cancel, hosting }
    }

    /// Verifica se a conexão terminou. `None` = ainda em andamento.
    pub fn poll(&self) -> Option<io::Result<LinkSession>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(io::Error::other("thread de conexão encerrou sem responder")))
            }
        }
    }

    /// Pede pra thread de fundo abortar a espera (best-effort; o accept/connect
    /// reage no próximo `POLL_INTERVAL`).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Bloqueia até a conexão terminar. Usado pela inicialização via flag de
    /// linha de comando, onde travar até o parceiro aparecer é o esperado.
    pub fn wait(self) -> io::Result<LinkSession> {
        self.rx
            .recv()
            .unwrap_or_else(|_| Err(io::Error::other("thread de conexão sumiu")))
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "conexão cancelada")
}

/// Hospeda e espera o parceiro, checando o cancelamento entre tentativas (o
/// listener fica não-bloqueante pra o accept não prender a thread pra sempre).
fn connect_host(port: u16, cancel: &AtomicBool) -> io::Result<LinkSession> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    listener.set_nonblocking(true)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Volta a bloquear: o protocolo de link conta com leituras que
                // esperam o parceiro. Nagle off (mensagens curtas e frequentes).
                stream.set_nonblocking(false)?;
                stream.set_nodelay(true)?;
                return LinkSession::establish(stream, 0, trace_sink(0));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL_INTERVAL),
            Err(e) => return Err(e),
        }
    }
}

/// Conecta no host, tentando de novo enquanto ele não está ouvindo (ou até
/// cancelar). Assim os dois lados podem entrar em qualquer ordem.
fn connect_join(addr: &str, cancel: &AtomicBool) -> io::Result<LinkSession> {
    // Resolve o endereço uma vez — erro de sintaxe/DNS falha na hora, sem ficar
    // re-tentando um destino que nunca vai existir.
    let target = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "endereço sem destino"))?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        match TcpStream::connect_timeout(&target, POLL_INTERVAL) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                return LinkSession::establish(stream, 1, trace_sink(1));
            }
            // Host ainda não subiu (recusado/timeout): espera e tenta de novo.
            Err(_) => thread::sleep(POLL_INTERVAL),
        }
    }
}

/// Abre o arquivo de trace em `/tmp` — só quando `AURORAGBA_LINK_TRACE` está
/// setada; fora isso devolve `None` (zero I/O).
fn trace_sink(id: u8) -> TraceSink {
    std::env::var_os(TRACE_ENV)?;
    let role = if id == 0 { "parent" } else { "child" };
    File::create(format!("/tmp/auroragba-link-{role}.log"))
        .map(|f| Box::new(BufWriter::new(f)) as Box<dyn io::Write + Send>)
        .map_err(|e| eprintln!("link: não consegui abrir o trace ({e})"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Descobre uma porta TCP livre (bind efêmero que é solto na hora).
    fn free_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Faz poll até a conexão resolver (ou estoura ~2 s e falha o teste).
    fn wait_ready(p: &PendingLink) -> io::Result<LinkSession> {
        for _ in 0..200 {
            if let Some(r) = p.poll() {
                return r;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("conexão de link não resolveu a tempo");
    }

    #[test]
    fn pending_link_host_e_join_conectam() {
        let port = free_port();
        let host = PendingLink::host(port);
        let join = PendingLink::join(format!("127.0.0.1:{port}"));
        // O join tenta de novo enquanto o host não sobe, então a ordem não
        // importa: os dois acabam estabelecendo a sessão com papéis opostos.
        let h = wait_ready(&host).expect("host devia conectar");
        let j = wait_ready(&join).expect("join devia conectar");
        assert_eq!(h.id, 0, "host é o parent (ID 0)");
        assert_eq!(j.id, 1, "join é o child (ID 1)");
    }

    #[test]
    fn pending_link_cancela_host_ocioso() {
        let host = PendingLink::host(free_port());
        host.cancel();
        for _ in 0..100 {
            match host.poll() {
                Some(Err(e)) => {
                    assert_eq!(
                        e.kind(),
                        io::ErrorKind::Interrupted,
                        "cancelar devia devolver Interrupted"
                    );
                    return;
                }
                Some(Ok(_)) => panic!("cancelado, mas conectou mesmo assim"),
                None => {}
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("cancelamento não surtiu efeito");
    }
}
