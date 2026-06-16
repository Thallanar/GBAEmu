//! Bootstrap TCP do link cable no desktop (Fase Link, L1).
//!
//! O protocolo e a máquina de estados moram no crate portátil `auroragba_link`,
//! atrás do trait `Transport`. Aqui só plugamos o transporte concreto do
//! desktop — `TcpStream` — e a política de trace (variável de ambiente +
//! arquivo em `/tmp`), que é específica do frontend.

use std::fs::File;
use std::io::{self, BufWriter};
use std::net::{TcpListener, TcpStream};

use auroragba_link::TraceSink;

/// Variável de ambiente que liga o trace da sessão em `/tmp` (depuração do
/// protocolo de link). Sem ela, nenhum arquivo é aberto e o trace é no-op.
const TRACE_ENV: &str = "AURORAGBA_LINK_TRACE";

/// Sessão de link do desktop: a portátil sobre um `TcpStream`.
pub type LinkSession = auroragba_link::LinkSession<TcpStream>;

/// Hospeda em `port` e espera o parceiro (bloqueante). Host = parent (master).
pub fn host(port: u16) -> io::Result<LinkSession> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    let (stream, _) = listener.accept()?;
    // Mensagens de ~12 bytes a centenas de Hz: Nagle só atrapalha.
    stream.set_nodelay(true)?;
    LinkSession::establish(stream, 0, trace_sink(0))
}

/// Conecta no host `addr` ("ip:porta"). Quem entra é o child.
pub fn join(addr: &str) -> io::Result<LinkSession> {
    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    LinkSession::establish(stream, 1, trace_sink(1))
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
