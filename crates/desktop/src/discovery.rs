//! Descoberta de parceiros de link na LAN (Fase Link, L2b + L2c).
//!
//! Roda **dois mecanismos em paralelo**, alimentando a mesma lista:
//!   - **mDNS** (padrão, tipo Bonjour) via `mdns-sd`: registra/descobre o
//!     serviço `_auroragba-link._tcp.local.`;
//!   - **UDP broadcast** (rede de segurança, só `std`): anúncio em
//!     `255.255.255.255:7778`, pra quando o multicast do mDNS estiver bloqueado.
//!
//! Não dá pra detectar de forma confiável que o multicast caiu (o mDNS só fica
//! em silêncio), então em vez de um "fallback" condicional os dois rodam sempre
//! e a lista é a UNIÃO (deduplicada por endereço). Cada lado é best-effort: se
//! um falha ao subir, o outro ainda acha o parceiro.
//!
//! Modelo: quem hospeda ([`Advertiser`]) anuncia "host de AuroraGBA na porta TCP
//! X, nome Y"; quem está ocioso ([`Browser`]) escuta e monta a lista (o IP vem
//! da rede, a porta TCP do anúncio). A UI só clica num da lista pra conectar.
//!
//! Escopo: LAN (nem broadcast nem mDNS cruzam roteadores). Mesmo aparelho usa
//! `127.0.0.1`. Se um mecanismo não puder subir (ex.: a porta UDP já tomada por
//! outra instância na mesma máquina), ele só fica indisponível e o resto segue.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

/// Porta UDP fixa onde os hosts anunciam presença (o link TCP usa a 7777).
pub const DISCOVERY_PORT: u16 = 7778;

const MAGIC: &[u8; 4] = b"AGBD";
const VERSION: u8 = 1;
/// Intervalo entre reanúncios do host (UDP).
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(1);
/// Tempo sem reanúncio até um host UDP sumir da lista (alguns ciclos).
const PEER_TTL: Duration = Duration::from_secs(4);
/// Limite do nome no payload UDP (mantém o pacote pequeno e previsível).
const MAX_NAME: usize = 64;

/// Tipo de serviço mDNS do link (RFC 6763). O ponto final é exigido.
const MDNS_SERVICE: &str = "_auroragba-link._tcp.local.";

/// Hosts vistos por UDP: endereço → (nome, instante do último anúncio).
type UdpPeers = Arc<Mutex<HashMap<SocketAddr, (String, Instant)>>>;
/// Hosts vistos por mDNS: fullname → (nome de instância, endereços).
type MdnsPeers = Arc<Mutex<HashMap<String, (String, Vec<SocketAddr>)>>>;

/// Um host de link visto na LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Nome amigável anunciado pelo host (só rótulo; pode repetir).
    pub name: String,
    /// IP do host + porta TCP do link.
    pub addr: SocketAddr,
}

// ===== UDP broadcast =====================================================

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

// ===== mDNS ==============================================================

/// Sanitiza o nome amigável num hostname mDNS válido ("<algo>.local.").
fn mdns_hostname(name: &str) -> String {
    let host: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let host = host.trim_matches('-');
    let host = if host.is_empty() { "auroragba" } else { host };
    format!("{host}.local.")
}

/// Extrai o nome de instância do fullname mDNS ("Nome._auroragba-link…" → "Nome").
fn instance_name(fullname: &str) -> String {
    fullname.split('.').next().unwrap_or(fullname).to_string()
}

/// Registra o serviço mDNS do host. Best-effort: erro = mDNS indisponível.
fn start_mdns_advertise(tcp_port: u16, name: &str) -> mdns_sd::Result<(ServiceDaemon, String)> {
    let daemon = ServiceDaemon::new()?;
    // `()` + enable_addr_auto: a lib preenche os IPs do host sozinha.
    let info = ServiceInfo::new(
        MDNS_SERVICE,
        name,
        &mdns_hostname(name),
        (),
        tcp_port,
        HashMap::<String, String>::new(),
    )?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info)?;
    Ok((daemon, fullname))
}

/// Inicia a busca mDNS, alimentando `peers` (keyed por fullname pra add/remove
/// por evento). Best-effort: erro = mDNS indisponível.
fn start_mdns_browse(peers: MdnsPeers) -> mdns_sd::Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(MDNS_SERVICE)?;
    thread::spawn(move || {
        // Sai quando o daemon é desligado (no Drop) e o canal fecha.
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let name = instance_name(info.get_fullname());
                    let port = info.get_port();
                    // Só IPv4 não-loopback: IPv6 link-local precisa de escopo
                    // e loopback é o próprio host (pra isso, 127.0.0.1 manual).
                    let addrs: Vec<SocketAddr> = info
                        .get_addresses()
                        .iter()
                        .filter_map(|scoped| match scoped.to_ip_addr() {
                            IpAddr::V4(v4) if !v4.is_loopback() => {
                                Some(SocketAddr::new(IpAddr::V4(v4), port))
                            }
                            _ => None,
                        })
                        .collect();
                    if !addrs.is_empty() {
                        peers
                            .lock()
                            .unwrap()
                            .insert(info.get_fullname().to_string(), (name, addrs));
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    peers.lock().unwrap().remove(&fullname);
                }
                _ => {}
            }
        }
    });
    Ok(daemon)
}

// ===== Advertiser / Browser =============================================

/// Anuncia, na LAN, que estamos hospedando um link — por mDNS e UDP broadcast.
/// Roda até ser dropado (ex.: quando a conexão fecha ou é cancelada).
pub struct Advertiser {
    udp_stop: Arc<AtomicBool>,
    /// Daemon mDNS + fullname registrado (pra desregistrar no Drop). `None` se
    /// o mDNS não pôde subir.
    mdns: Option<(ServiceDaemon, String)>,
}

impl Advertiser {
    /// Começa a anunciar um host na porta TCP `tcp_port` com o rótulo `name`.
    /// Falha só se nem o UDP conseguir subir; o mDNS é best-effort.
    pub fn start(tcp_port: u16, name: String) -> io::Result<Advertiser> {
        // UDP: porta efêmera só pra enviar; o broadcast vai pra DISCOVERY_PORT.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        let udp_stop = Arc::new(AtomicBool::new(false));
        let thread_stop = udp_stop.clone();
        let packet = encode_announce(tcp_port, &name);
        thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let _ = socket.send_to(&packet, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
                sleep_interruptible(&thread_stop, ANNOUNCE_INTERVAL);
            }
        });

        let mdns = start_mdns_advertise(tcp_port, &name)
            .map_err(|e| log::info!("anúncio mDNS indisponível: {e}"))
            .ok();

        Ok(Advertiser { udp_stop, mdns })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.udp_stop.store(true, Ordering::Relaxed);
        if let Some((daemon, fullname)) = &self.mdns {
            let _ = daemon.unregister(fullname);
            let _ = daemon.shutdown();
        }
    }
}

/// Escuta anúncios de hosts na LAN (mDNS + UDP) e mantém a lista do que foi
/// visto. Roda até ser dropado.
pub struct Browser {
    /// Hosts vistos por UDP (expira por TTL).
    udp_peers: UdpPeers,
    /// Hosts vistos por mDNS (add/remove por evento).
    mdns_peers: MdnsPeers,
    udp_stop: Arc<AtomicBool>,
    /// Daemon mDNS (desligado no Drop pra encerrar a thread de browse). `None`
    /// se o mDNS não pôde subir.
    mdns: Option<ServiceDaemon>,
}

impl Browser {
    /// Sobe a escuta. Erro só se o UDP não puder abrir a porta; o mDNS é
    /// best-effort. Com os dois off, `peers()` fica sempre vazio (mas o caminho
    /// manual segue valendo).
    pub fn start() -> io::Result<Browser> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))?;
        // Timeout no recv pra a thread checar o `stop` mesmo sem tráfego.
        socket.set_read_timeout(Some(Duration::from_millis(500)))?;
        let udp_peers = Arc::new(Mutex::new(HashMap::new()));
        let udp_stop = Arc::new(AtomicBool::new(false));
        let thread_peers = udp_peers.clone();
        let thread_stop = udp_stop.clone();
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

        let mdns_peers = Arc::new(Mutex::new(HashMap::new()));
        let mdns = start_mdns_browse(mdns_peers.clone())
            .map_err(|e| log::info!("busca mDNS indisponível: {e}"))
            .ok();

        Ok(Browser { udp_peers, mdns_peers, udp_stop, mdns })
    }

    /// Lista atual de hosts vistos — UNIÃO de mDNS e UDP, deduplicada por
    /// endereço. Expira os hosts UDP silenciosos há mais de `PEER_TTL`; os mDNS
    /// saem por evento. Ordenada (nome, endereço) pra a UI não ficar pulando.
    pub fn peers(&self) -> Vec<Peer> {
        let now = Instant::now();
        let mut by_addr: HashMap<SocketAddr, String> = HashMap::new();

        {
            let mut udp = self.udp_peers.lock().unwrap();
            udp.retain(|_, (_, seen)| now.duration_since(*seen) < PEER_TTL);
            for (addr, (name, _)) in udp.iter() {
                by_addr.entry(*addr).or_insert_with(|| name.clone());
            }
        }
        {
            let mdns = self.mdns_peers.lock().unwrap();
            for (name, addrs) in mdns.values() {
                for addr in addrs {
                    by_addr.entry(*addr).or_insert_with(|| name.clone());
                }
            }
        }

        let mut peers: Vec<Peer> =
            by_addr.into_iter().map(|(addr, name)| Peer { name, addr }).collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name).then(a.addr.cmp(&b.addr)));
        peers
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.udp_stop.store(true, Ordering::Relaxed);
        if let Some(daemon) = &self.mdns {
            let _ = daemon.shutdown();
        }
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

    #[test]
    fn mdns_hostname_sanitiza() {
        assert_eq!(mdns_hostname("Sala do Ash"), "Sala-do-Ash.local.");
        assert_eq!(mdns_hostname("!!!"), "auroragba.local.");
        assert_eq!(instance_name("Ash._auroragba-link._tcp.local."), "Ash");
    }

    /// Caminho real do UDP: `Advertiser` em broadcast e `Browser` na porta fixa
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

    /// Caminho real do mDNS, isolado do UDP: registrar + buscar resolve o host
    /// e popula o mapa. Ignorado — usa multicast real e depende de o ambiente
    /// ter uma interface IPv4 não-loopback.
    #[test]
    #[ignore = "usa mDNS real (multicast); rode com --ignored"]
    fn mdns_resolve_popula_a_lista() {
        let peers: MdnsPeers = Arc::new(Mutex::new(HashMap::new()));
        let _browse = start_mdns_browse(peers.clone()).unwrap();
        let _adv = start_mdns_advertise(7777, "MdnsTeste").unwrap();
        for _ in 0..100 {
            let found = peers
                .lock()
                .unwrap()
                .values()
                .any(|(name, addrs)| name == "MdnsTeste" && !addrs.is_empty());
            if found {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("mDNS não resolveu o host anunciado");
    }
}
