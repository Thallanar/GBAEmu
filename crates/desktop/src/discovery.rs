//! Descoberta de parceiros de link na LAN via **UDP broadcast** (Fase Link,
//! L2b). Sem dependências: só `std`.
//!
//! Modelo: quem hospeda um link [`Advertiser`] manda um pacote de anúncio em
//! broadcast (`255.255.255.255`) a cada segundo — "tem um host de AuroraGBA na
//! porta TCP X, nome Y". Quem está ocioso roda um [`Browser`] que escuta numa
//! porta fixa, monta a lista de hosts vistos (o IP vem do remetente, a porta TCP
//! do payload) e expira os que pararam de anunciar. A UI só precisa clicar num
//! da lista pra conectar — sem digitar IP.
//!
//! Escopo: LAN (o broadcast não cruza roteadores). Para o mesmo aparelho,
//! continua valendo digitar `127.0.0.1`. Se o `Browser` não conseguir abrir a
//! porta (ex.: outra instância na mesma máquina já a tomou), a descoberta
//! simplesmente fica indisponível — o caminho manual segue funcionando.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Porta UDP fixa onde os hosts anunciam presença (o link TCP usa a 7777).
pub const DISCOVERY_PORT: u16 = 7778;

const MAGIC: &[u8; 4] = b"AGBD";
const VERSION: u8 = 1;
/// Intervalo entre reanúncios do host.
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(1);
/// Tempo sem reanúncio até um host sumir da lista (alguns ciclos de anúncio).
const PEER_TTL: Duration = Duration::from_secs(4);
/// Limite do nome no payload (mantém o pacote pequeno e previsível).
const MAX_NAME: usize = 64;

/// Um host de link visto na LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Nome amigável anunciado pelo host (só rótulo; pode repetir).
    pub name: String,
    /// IP do host (do remetente) + porta TCP do link (do payload).
    pub addr: SocketAddr,
}

/// Monta o pacote de anúncio: MAGIC + versão + porta TCP (LE) + nome (com tamanho).
fn encode_announce(tcp_port: u16, name: &str) -> Vec<u8> {
    let name = name.as_bytes();
    let n = name.len().min(MAX_NAME);
    let mut buf = Vec::with_capacity(8 + n);
    buf.extend_from_slice(MAGIC);
    buf.push(VERSION);
    buf.extend_from_slice(&tcp_port.to_le_bytes());
    buf.push(n as u8);
    buf.extend_from_slice(&name[..n]);
    buf
}

/// Lê um anúncio: devolve (porta TCP, nome) ou `None` se não for nosso pacote.
fn decode_announce(buf: &[u8]) -> Option<(u16, String)> {
    if buf.len() < 8 || &buf[..4] != MAGIC || buf[4] != VERSION {
        return None;
    }
    let tcp_port = u16::from_le_bytes([buf[5], buf[6]]);
    let n = buf[7] as usize;
    let name = buf.get(8..8 + n)?;
    Some((tcp_port, String::from_utf8_lossy(name).into_owned()))
}

/// Resolve um datagrama recebido num par (endereço de link, nome). O endereço
/// junta o IP de quem mandou com a porta TCP anunciada. Fatorado pra teste.
fn peer_from_datagram(buf: &[u8], src: SocketAddr) -> Option<(SocketAddr, String)> {
    let (tcp_port, name) = decode_announce(buf)?;
    Some((SocketAddr::new(src.ip(), tcp_port), name))
}

/// Dorme `total`, mas acorda cedo se `stop` for sinalizado (pra a thread sair
/// rápido em vez de esperar o intervalo inteiro).
fn sleep_interruptible(stop: &AtomicBool, total: Duration) {
    const STEP: Duration = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < total && !stop.load(Ordering::Relaxed) {
        thread::sleep(STEP);
        slept += STEP;
    }
}

/// Anuncia, em broadcast, que estamos hospedando um link. Roda numa thread até
/// ser dropado (ex.: quando a conexão fecha ou é cancelada).
pub struct Advertiser {
    stop: Arc<AtomicBool>,
}

impl Advertiser {
    /// Começa a anunciar um host na porta TCP `tcp_port` com o rótulo `name`.
    pub fn start(tcp_port: u16, name: String) -> io::Result<Advertiser> {
        // Porta efêmera só pra enviar; o broadcast vai pra DISCOVERY_PORT.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let packet = encode_announce(tcp_port, &name);
        thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let _ = socket.send_to(&packet, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
                sleep_interruptible(&thread_stop, ANNOUNCE_INTERVAL);
            }
        });
        Ok(Advertiser { stop })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Escuta anúncios de hosts na LAN e mantém a lista do que foi visto. Roda numa
/// thread até ser dropado.
pub struct Browser {
    peers: Arc<Mutex<HashMap<SocketAddr, (String, Instant)>>>,
    stop: Arc<AtomicBool>,
}

impl Browser {
    /// Abre a porta de descoberta e começa a escutar. Erro = porta indisponível
    /// (a descoberta fica off; o caminho manual segue valendo).
    pub fn start() -> io::Result<Browser> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))?;
        // Timeout no recv pra a thread checar o `stop` mesmo sem tráfego.
        socket.set_read_timeout(Some(Duration::from_millis(500)))?;
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_peers = peers.clone();
        let thread_stop = stop.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 128];
            while !thread_stop.load(Ordering::Relaxed) {
                match socket.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        if let Some((addr, name)) = peer_from_datagram(&buf[..n], src) {
                            thread_peers
                                .lock()
                                .unwrap()
                                .insert(addr, (name, Instant::now()));
                        }
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Browser { peers, stop })
    }

    /// Lista atual de hosts vistos, já sem os que silenciaram há mais de
    /// `PEER_TTL`. Ordenada (nome, endereço) pra a UI não ficar pulando.
    pub fn peers(&self) -> Vec<Peer> {
        let mut map = self.peers.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, (_, seen)| now.duration_since(*seen) < PEER_TTL);
        let mut peers: Vec<Peer> = map
            .iter()
            .map(|(addr, (name, _))| Peer { name: name.clone(), addr: *addr })
            .collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name).then(a.addr.cmp(&b.addr)));
        peers
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anuncio_roundtrip() {
        let buf = encode_announce(7777, "Sala do Ash");
        assert_eq!(decode_announce(&buf), Some((7777, "Sala do Ash".to_string())));
    }

    #[test]
    fn decode_rejeita_lixo() {
        assert_eq!(decode_announce(b"xx"), None); // curto demais
        assert_eq!(decode_announce(b"NOPEv...."), None); // magic errado
        let mut bad = encode_announce(1, "a");
        bad[4] = 99; // versão incompatível
        assert_eq!(decode_announce(&bad), None);
    }

    #[test]
    fn peer_junta_ip_do_remetente_com_porta_do_payload() {
        // Recebe num socket efêmero (sem tocar na DISCOVERY_PORT real) pra
        // validar o round trip de socket + a montagem do endereço do peer.
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let rx_port = rx.local_addr().unwrap().port();
        rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

        let tx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let packet = encode_announce(7777, "Ginásio");
        tx.send_to(&packet, (Ipv4Addr::LOCALHOST, rx_port)).unwrap();

        let mut buf = [0u8; 128];
        let (n, src) = rx.recv_from(&mut buf).unwrap();
        let (addr, name) = peer_from_datagram(&buf[..n], src).expect("devia decodificar");
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(addr.port(), 7777, "porta vem do payload, não do remetente");
        assert_eq!(name, "Ginásio");
    }

    /// Caminho real: um `Advertiser` em broadcast e um `Browser` na porta fixa
    /// se enxergam. Ignorado por padrão — usa a `DISCOVERY_PORT` real e depende
    /// de o ambiente permitir broadcast (não roda junto com outros testes).
    #[test]
    #[ignore = "usa a DISCOVERY_PORT real + broadcast; rode com --ignored"]
    fn advertiser_e_browser_se_enxergam() {
        let browser = Browser::start().unwrap();
        let _adv = Advertiser::start(7777, "Teste".into()).unwrap();
        for _ in 0..30 {
            if browser
                .peers()
                .iter()
                .any(|p| p.name == "Teste" && p.addr.port() == 7777)
            {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("o browser não enxergou o anúncio do advertiser");
    }
}
