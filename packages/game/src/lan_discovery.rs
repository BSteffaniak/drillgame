#![allow(
    dead_code,
    reason = "LAN discovery is wired incrementally into online UX"
)]

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::multiplayer::QuinnHostConnectionDescriptor;

pub const SERVICE_TYPE: &str = "_drillgame._udp.local.";
pub const PROTOCOL_VERSION: &str = "1";
const DESCRIPTOR_READ_LIMIT_BYTES: usize = 128 * 1024;
const DEFAULT_DESCRIPTOR_FETCH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanGameAdvertisement {
    pub instance_name: String,
    pub host_name: String,
    pub game_addr: SocketAddr,
    pub descriptor_addr: SocketAddr,
    pub session_id: String,
}

impl LanGameAdvertisement {
    #[must_use]
    pub fn from_descriptor(descriptor: &QuinnHostConnectionDescriptor) -> Self {
        let machine_name = local_machine_name();
        let game_addr = descriptor.host_addr;
        let descriptor_addr = SocketAddr::new(game_addr.ip(), 0);
        Self {
            instance_name: format!("Drillgame on {machine_name}"),
            host_name: format!("{}.local.", dns_label(&machine_name)),
            game_addr,
            descriptor_addr,
            session_id: format!("drillgame-{}", game_addr.port()),
        }
    }

    #[must_use]
    pub const fn with_descriptor_addr(mut self, descriptor_addr: SocketAddr) -> Self {
        self.descriptor_addr = descriptor_addr;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanDiscoveredGame {
    pub instance_name: String,
    pub host_name: String,
    pub game_addr: SocketAddr,
    pub descriptor_addr: SocketAddr,
    pub session_id: String,
}

pub struct LanDiscoveryPublisher {
    daemon: ServiceDaemon,
    fullname: String,
}

impl LanDiscoveryPublisher {
    /// Publish this LAN game through mDNS until the returned publisher is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the mDNS daemon cannot be started or service info cannot be registered.
    pub fn publish(advertisement: &LanGameAdvertisement) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        let properties = HashMap::from([
            ("protocol".to_owned(), PROTOCOL_VERSION.to_owned()),
            ("session_id".to_owned(), advertisement.session_id.clone()),
            ("game_addr".to_owned(), advertisement.game_addr.to_string()),
            (
                "descriptor_addr".to_owned(),
                advertisement.descriptor_addr.to_string(),
            ),
        ]);
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &advertisement.instance_name,
            &advertisement.host_name,
            advertisement.game_addr.ip().to_string(),
            advertisement.game_addr.port(),
            Some(properties),
        )?;
        let fullname = service.get_fullname().to_owned();
        daemon.register(service)?;
        Ok(Self { daemon, fullname })
    }
}

impl Drop for LanDiscoveryPublisher {
    fn drop(&mut self) {
        let _ignored = self.daemon.unregister(&self.fullname);
    }
}

#[derive(Debug)]
pub struct LanDescriptorServer {
    local_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LanDescriptorServer {
    /// Serve the descriptor JSON over a tiny local TCP endpoint until dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot bind or descriptor serialization fails.
    pub fn serve(
        bind_addr: SocketAddr,
        descriptor: &QuinnHostConnectionDescriptor,
    ) -> Result<Self, std::io::Error> {
        let json = serde_json::to_vec(descriptor).map_err(std::io::Error::other)?;
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::Builder::new()
            .name("drillgame_lan_descriptor".to_owned())
            .spawn(move || {
                while !thread_shutdown.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _peer)) => {
                            let _ignored = stream.write_all(&json);
                            let _ignored = stream.flush();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25));
                        }
                        Err(_error) => break,
                    }
                }
            })?;
        Ok(Self {
            local_addr,
            shutdown,
            handle: Some(handle),
        })
    }

    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for LanDescriptorServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ignored = TcpStream::connect_timeout(&self.local_addr, Duration::from_millis(50));
        if let Some(handle) = self.handle.take() {
            let _ignored = handle.join();
        }
    }
}

pub struct LanDiscoveryBrowser {
    daemon: ServiceDaemon,
}

impl LanDiscoveryBrowser {
    /// Create an mDNS browser for Drillgame LAN sessions.
    ///
    /// # Errors
    ///
    /// Returns an error if the mDNS daemon cannot be started.
    pub fn new() -> Result<Self, mdns_sd::Error> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
        })
    }

    /// Browse for LAN sessions for a bounded amount of time.
    ///
    /// # Errors
    ///
    /// Returns an error if mDNS browsing fails.
    pub fn browse_for(&self, duration: Duration) -> Result<Vec<LanDiscoveredGame>, mdns_sd::Error> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        let deadline = std::time::Instant::now() + duration;
        let mut games = Vec::new();
        while std::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if let Ok(ServiceEvent::ServiceResolved(info)) =
                receiver.recv_timeout(remaining.min(Duration::from_millis(100)))
                && let Some(game) = discovered_game_from_service(&info)
                && !games.iter().any(|existing| existing == &game)
            {
                games.push(game);
            }
        }
        Ok(games)
    }
}

/// Fetch a descriptor from a LAN descriptor endpoint.
///
/// # Errors
///
/// Returns an error if the endpoint cannot be reached, read, or parsed.
pub fn fetch_descriptor(
    descriptor_addr: SocketAddr,
) -> Result<QuinnHostConnectionDescriptor, std::io::Error> {
    let stream = TcpStream::connect_timeout(&descriptor_addr, DEFAULT_DESCRIPTOR_FETCH_TIMEOUT)?;
    stream.set_read_timeout(Some(DEFAULT_DESCRIPTOR_FETCH_TIMEOUT))?;
    let mut json = Vec::new();
    stream
        .take(u64::try_from(DESCRIPTOR_READ_LIMIT_BYTES).expect("descriptor limit fits u64"))
        .read_to_end(&mut json)?;
    serde_json::from_slice(&json).map_err(std::io::Error::other)
}

#[must_use]
pub const fn patch_descriptor_addr_for_lan(
    mut descriptor: QuinnHostConnectionDescriptor,
    discovered_game_addr: SocketAddr,
) -> QuinnHostConnectionDescriptor {
    descriptor.host_addr = patch_non_lan_addr(descriptor.host_addr, discovered_game_addr.ip());
    descriptor
}

fn resolved_service_addr(info: &ResolvedService) -> Option<SocketAddr> {
    info.get_addresses()
        .iter()
        .find(|address| address.is_ipv4())
        .map(mdns_sd::ScopedIp::to_ip_addr)
        .map(|address| SocketAddr::new(address, info.get_port()))
}

const fn patch_non_lan_addr(addr: SocketAddr, fallback_ip: IpAddr) -> SocketAddr {
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        SocketAddr::new(fallback_ip, addr.port())
    } else {
        addr
    }
}

#[must_use]
pub fn likely_lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).ok()?;
    socket
        .connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 80))
        .ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn discovered_game_from_service(info: &ResolvedService) -> Option<LanDiscoveredGame> {
    let properties = info.get_properties();
    let descriptor_addr = properties.get("descriptor_addr")?.val_str().parse().ok()?;
    let game_addr = properties
        .get("game_addr")
        .and_then(|property| property.val_str().parse().ok())
        .or_else(|| resolved_service_addr(info))?;
    let descriptor_addr = patch_non_lan_addr(descriptor_addr, game_addr.ip());
    let game_addr = patch_non_lan_addr(game_addr, descriptor_addr.ip());
    Some(LanDiscoveredGame {
        instance_name: info.get_fullname().split('.').next()?.to_owned(),
        host_name: info.get_hostname().to_owned(),
        game_addr,
        descriptor_addr,
        session_id: properties
            .get("session_id")
            .map_or_else(String::new, |property| property.val_str().to_owned()),
    })
}

#[must_use]
pub fn local_machine_name() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Unknown Miner".to_owned())
}

fn dns_label(name: &str) -> String {
    let label: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    label.trim_matches('-').to_owned()
}

#[must_use]
pub const fn localhost_descriptor_bind_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_label_sanitizes_hostname_for_mdns() {
        assert_eq!(dns_label("Braden's MacBook Pro"), "braden-s-macbook-pro");
    }

    #[test]
    fn advertisement_uses_descriptor_game_address() {
        let descriptor = QuinnHostConnectionDescriptor {
            host_addr: "192.168.1.20:4243".parse().expect("address parses"),
            server_name: "localhost".to_owned(),
            certificate_der: vec![1, 2, 3],
        };
        let advertisement = LanGameAdvertisement::from_descriptor(&descriptor)
            .with_descriptor_addr("192.168.1.20:4244".parse().expect("address parses"));

        assert_eq!(advertisement.game_addr, descriptor.host_addr);
        assert_eq!(advertisement.descriptor_addr.port(), 4244);
        assert!(advertisement.instance_name.starts_with("Drillgame on "));
    }

    #[test]
    fn descriptor_server_round_trips_descriptor() {
        let descriptor = QuinnHostConnectionDescriptor {
            host_addr: "127.0.0.1:4243".parse().expect("address parses"),
            server_name: "localhost".to_owned(),
            certificate_der: vec![1, 2, 3],
        };
        let server =
            LanDescriptorServer::serve("127.0.0.1:0".parse().expect("address parses"), &descriptor)
                .expect("server starts");

        let fetched = fetch_descriptor(server.local_addr()).expect("descriptor fetches");

        assert_eq!(fetched, descriptor);
    }
}
