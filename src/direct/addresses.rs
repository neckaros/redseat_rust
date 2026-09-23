//! Candidate addresses for direct HTTPS: LAN interface addresses, global IPv6 addresses and
//! the public IPv4 when the port is reachable on it (UPnP-IGD mapping or manual forwarding).

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use igd_next::{
    aio::{
        tokio::{search_gateway, Tokio},
        Gateway,
    },
    AddPortError, PortMappingProtocol, SearchOptions,
};

use super::cloud::AddressReport;

/// The cloud accepts at most 16 addresses per list.
const MAX_ADDRESSES: usize = 16;
/// UPnP lease, renewed on every address check (well before it expires).
const UPNP_LEASE_SECONDS: u32 = 3600;
const UPNP_DESCRIPTION: &str = "RedSeat";

/// Virtual interfaces that other devices can't reach (containers, VMs, WSL).
const IGNORED_INTERFACE_PREFIXES: &[&str] = &[
    "docker",
    "br-",
    "veth",
    "virbr",
    "cni",
    "flannel",
    "podman",
    "vEthernet (WSL",
    "vEthernet (Default Switch",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// Private or unique-local: reachable from the same network.
    Lan,
    PublicV4,
    PublicV6,
}

/// Classifies an address the way the cloud validates it; `None` for addresses no browser
/// could use (loopback, link-local, multicast, unspecified, IPv4-mapped…).
pub fn classify(ip: IpAddr) -> Option<AddressKind> {
    match ip {
        IpAddr::V4(ip) => classify_v4(ip),
        IpAddr::V6(ip) => classify_v6(ip),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Option<AddressKind> {
    let [a, b, ..] = ip.octets();
    if a == 0 || ip.is_loopback() || ip.is_link_local() || ip.is_multicast() || a >= 240 {
        return None;
    }
    // 100.64.0.0/10 is carrier-grade NAT, but also VPN overlays (Tailscale): LAN-only.
    if ip.is_private() || (a == 100 && (64..128).contains(&b)) {
        return Some(AddressKind::Lan);
    }
    Some(AddressKind::PublicV4)
}

fn classify_v6(ip: Ipv6Addr) -> Option<AddressKind> {
    let segments = ip.segments();
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return None;
    }
    // Link-local (fe80::/10), IPv4-mapped (::ffff:0:0/96) and IPv4-compatible (::/96).
    if (segments[0] & 0xffc0) == 0xfe80 || segments[..5].iter().all(|s| *s == 0) {
        return None;
    }
    if (segments[0] & 0xfe00) == 0xfc00 {
        return Some(AddressKind::Lan);
    }
    if (segments[0] & 0xe000) == 0x2000 {
        return Some(AddressKind::PublicV6);
    }
    None
}

#[derive(Debug, Clone)]
struct LocalV4 {
    ip: Ipv4Addr,
    netmask: Ipv4Addr,
}

#[derive(Debug, Default)]
struct LocalAddresses {
    lan: Vec<IpAddr>,
    lan_v4: Vec<LocalV4>,
    public_v4: Option<Ipv4Addr>,
    public_v6: Vec<Ipv6Addr>,
}

fn local_addresses() -> LocalAddresses {
    let mut addresses = LocalAddresses::default();
    let interfaces = match if_addrs::get_if_addrs() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            crate::tools::log::log_error(
                crate::tools::log::LogServiceType::Register,
                format!("Direct HTTPS: unable to list network interfaces: {error}"),
            );
            return addresses;
        }
    };
    for interface in interfaces {
        if interface.oper_status == if_addrs::IfOperStatus::Down
            || IGNORED_INTERFACE_PREFIXES
                .iter()
                .any(|prefix| interface.name.starts_with(prefix))
        {
            continue;
        }
        let ip = interface.ip();
        match (classify(ip), &interface.addr) {
            (Some(AddressKind::Lan), if_addrs::IfAddr::V4(v4)) => {
                addresses.lan.push(ip);
                addresses.lan_v4.push(LocalV4 {
                    ip: v4.ip,
                    netmask: v4.netmask,
                });
            }
            (Some(AddressKind::Lan), _) => addresses.lan.push(ip),
            (Some(AddressKind::PublicV4), if_addrs::IfAddr::V4(v4)) => {
                addresses.public_v4.get_or_insert(v4.ip);
            }
            (Some(AddressKind::PublicV6), if_addrs::IfAddr::V6(v6)) => {
                addresses.public_v6.push(v6.ip)
            }
            _ => {}
        }
    }
    addresses
}

pub struct DiscoveryOptions {
    /// Port clients connect to (the router's external port and the reported port).
    pub port: u16,
    /// Port the server listens on locally.
    pub local_port: u16,
    /// LAN addresses to report instead of the discovered ones (e.g. the host's in Docker).
    pub lan_override: Option<Vec<IpAddr>>,
    /// The user forwarded `port` on the router: report the public IPv4 without UPnP.
    pub port_forwarded: bool,
    /// The server listens on IPv6; without it IPv6 candidates are left out.
    pub ipv6: bool,
}

/// Discovers candidate addresses, opening the port with UPnP-IGD when a gateway offers it.
pub async fn discover(options: &DiscoveryOptions) -> AddressReport {
    let local = local_addresses();

    let mut lan: Vec<IpAddr> = match &options.lan_override {
        Some(lan) => lan
            .iter()
            .copied()
            .filter(|ip| classify(*ip).is_some())
            .collect(),
        None => local.lan.clone(),
    };
    if !options.ipv6 {
        lan.retain(IpAddr::is_ipv4);
    }
    dedup(&mut lan);

    // With overridden LAN addresses (container), map to the host: its address and the
    // exposed port. Otherwise map to this interface's address and the listening port.
    let upnp_target = match &options.lan_override {
        Some(_) => lan.iter().find_map(|ip| match ip {
            IpAddr::V4(ip) => Some(UpnpTarget::Fixed(SocketAddrV4::new(*ip, options.port))),
            IpAddr::V6(_) => None,
        }),
        None => Some(UpnpTarget::GatewaySubnet(local.lan_v4.clone())),
    };

    let mut ipv4 = local.public_v4;
    if let (None, Some(target)) = (ipv4, upnp_target) {
        ipv4 = match upnp_map_port(options, target).await {
            Ok(external) => Some(external),
            Err(reason) => {
                crate::tools::log::log_info(
                    crate::tools::log::LogServiceType::Register,
                    format!("Direct HTTPS: no UPnP port mapping ({reason})"),
                );
                None
            }
        };
    }
    if ipv4.is_none() && options.port_forwarded {
        ipv4 = crate::server::get_ipv4()
            .await
            .ok()
            .and_then(|ip| ip.trim().parse::<Ipv4Addr>().ok())
            .filter(|ip| classify_v4(*ip) == Some(AddressKind::PublicV4));
    }

    let mut ipv6 = if options.ipv6 {
        local.public_v6
    } else {
        vec![]
    };
    dedup(&mut ipv6);

    AddressReport {
        lan: lan
            .into_iter()
            .take(MAX_ADDRESSES)
            .map(|ip| ip.to_string())
            .collect(),
        ipv4: ipv4.map(|ip| ip.to_string()),
        ipv6: ipv6
            .into_iter()
            .take(MAX_ADDRESSES)
            .map(|ip| ip.to_string())
            .collect(),
        port: options.port,
    }
}

/// Removes duplicates, keeping the first occurrence (and the configured order).
fn dedup<T: Eq + std::hash::Hash + Clone>(values: &mut Vec<T>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

enum UpnpTarget {
    /// The local address on the gateway's subnet, with the listening port.
    GatewaySubnet(Vec<LocalV4>),
    Fixed(SocketAddrV4),
}

const SSDP_MULTICAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
const SSDP_PORT: u16 = 1900;
const SSDP_SEARCH: &str = "M-SEARCH * HTTP/1.1\r\n\
    HOST: 239.255.255.250:1900\r\n\
    ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\
    MAN: \"ssdp:discover\"\r\n\
    MX: 2\r\n\r\n";
const SSDP_WAIT: Duration = Duration::from_secs(3);

/// Routers answering an SSDP search sent from `local`'s interface, on `local`'s subnet.
///
/// `IP_MULTICAST_IF` pins the multicast to that interface: binding alone isn't enough, since
/// the routing table (which a full-tunnel VPN points into the tunnel) still picks the egress.
async fn ssdp_gateways(local: &LocalV4) -> std::io::Result<Vec<Ipv4Addr>> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.bind(&SocketAddr::V4(SocketAddrV4::new(local.ip, 0)).into())?;
    socket.set_multicast_if_v4(&local.ip)?;
    socket.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(socket.into())?;
    socket
        .send_to(SSDP_SEARCH.as_bytes(), SSDP_MULTICAST)
        .await?;

    let deadline = tokio::time::Instant::now() + SSDP_WAIT;
    let mut gateways = vec![];
    let mut buffer = [0u8; 2048];
    loop {
        match tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await {
            Ok(Ok((_, SocketAddr::V4(from)))) => {
                if same_subnet(local, *from.ip()) && !gateways.contains(from.ip()) {
                    gateways.push(*from.ip());
                }
            }
            Ok(Ok(_)) => {}
            // Windows reports ICMP "port unreachable" from earlier sends as a receive error.
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => return Ok(gateways),
        }
    }
}

/// The IGD at `gateway_ip`, reached with a unicast search from `bind_ip` (unicast follows the
/// LAN route, so a VPN can't divert it).
async fn gateway_at(bind_ip: Ipv4Addr, gateway_ip: Ipv4Addr) -> Result<Gateway<Tokio>, String> {
    search_gateway(SearchOptions {
        bind_addr: SocketAddr::V4(SocketAddrV4::new(bind_ip, 0)),
        broadcast_address: SocketAddr::V4(SocketAddrV4::new(gateway_ip, SSDP_PORT)),
        timeout: Some(SSDP_WAIT),
        single_search_timeout: Some(SSDP_WAIT),
    })
    .await
    .map_err(|e| format!("{gateway_ip}: {e}"))
}

/// Maps `port` on the IGD gateway to this host and returns the gateway's public IPv4.
///
/// Every router answering on any LAN interface is tried until one maps the port and has a
/// public address: on a host with several networks, the first to answer may be an internal
/// router.
async fn upnp_map_port(options: &DiscoveryOptions, target: UpnpTarget) -> Result<Ipv4Addr, String> {
    // (address to map to, gateway)
    let mut candidates: Vec<(SocketAddrV4, Gateway<Tokio>)> = vec![];
    let mut errors: Vec<String> = vec![];
    match target {
        UpnpTarget::GatewaySubnet(lan_v4) => {
            let found = futures::future::join_all(
                lan_v4
                    .iter()
                    .map(|local| async move { (local.ip, ssdp_gateways(local).await) }),
            )
            .await;
            for (local_ip, gateways) in found {
                match gateways {
                    Ok(gateways) => {
                        for gateway_ip in gateways {
                            match gateway_at(local_ip, gateway_ip).await {
                                Ok(gateway) => candidates.push((
                                    SocketAddrV4::new(local_ip, options.local_port),
                                    gateway,
                                )),
                                Err(error) => errors.push(error),
                            }
                        }
                    }
                    Err(error) => errors.push(format!("search from {local_ip}: {error}")),
                }
            }
        }
        // A container's addresses aren't the host's: search from the default interface.
        UpnpTarget::Fixed(local) => match search_gateway(SearchOptions {
            timeout: Some(SSDP_WAIT),
            single_search_timeout: Some(SSDP_WAIT),
            ..Default::default()
        })
        .await
        {
            Ok(gateway) => candidates.push((local, gateway)),
            Err(error) => errors.push(error.to_string()),
        },
    }
    if candidates.is_empty() && errors.is_empty() {
        return Err("no gateway answered".to_string());
    }
    for (local, gateway) in candidates {
        match map_on(&gateway, options.port, local).await {
            Ok(external) => return Ok(external),
            Err(error) => errors.push(format!("{}: {error}", gateway.addr)),
        }
    }
    Err(errors.join("; "))
}

async fn map_on(
    gateway: &Gateway<Tokio>,
    port: u16,
    local: SocketAddrV4,
) -> Result<Ipv4Addr, String> {
    // Leased mappings only: they expire on their own once RedSeat stops renewing them. A
    // permanent one (routers answering OnlyPermanentLeasesSupported) would keep forwarding
    // the port after RedSeat stops or moves; forward it manually there (`portForwarded`).
    gateway
        .add_port(
            PortMappingProtocol::TCP,
            port,
            SocketAddr::V4(local),
            UPNP_LEASE_SECONDS,
            UPNP_DESCRIPTION,
        )
        .await
        .map_err(|e| match e {
            AddPortError::OnlyPermanentLeasesSupported => {
                "the router only allows permanent mappings; forward the port manually and set portForwarded".to_string()
            }
            e => format!("mapping refused: {e}"),
        })?;

    match gateway.get_external_ip().await {
        Ok(IpAddr::V4(external)) if classify_v4(external) == Some(AddressKind::PublicV4) => {
            Ok(external)
        }
        Ok(external) => Err(format!("gateway has no public IPv4 ({external})")),
        Err(e) => Err(format!("unable to read external IP: {e}")),
    }
}

fn same_subnet(local: &LocalV4, other: Ipv4Addr) -> bool {
    let mask = u32::from(local.netmask);
    u32::from(local.ip) & mask == u32::from(other) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(ip: &str) -> Option<AddressKind> {
        classify(ip.parse().unwrap())
    }

    #[test]
    fn classifies_like_the_cloud() {
        assert_eq!(kind("192.168.1.10"), Some(AddressKind::Lan));
        assert_eq!(kind("10.0.0.2"), Some(AddressKind::Lan));
        assert_eq!(kind("172.20.0.2"), Some(AddressKind::Lan));
        assert_eq!(kind("100.100.1.1"), Some(AddressKind::Lan));
        assert_eq!(kind("fd00::10"), Some(AddressKind::Lan));
        assert_eq!(kind("82.64.1.2"), Some(AddressKind::PublicV4));
        assert_eq!(kind("2a01:e0a::10"), Some(AddressKind::PublicV6));

        for rejected in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.1.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
            "::ffff:192.168.1.10",
            "::192.168.1.10",
        ] {
            assert_eq!(kind(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn dedup_keeps_the_configured_order() {
        let parse = |ip: &str| ip.parse::<IpAddr>().unwrap();
        let mut ips = vec![
            parse("192.168.1.10"),
            parse("100.64.1.2"),
            parse("192.168.1.10"),
        ];
        dedup(&mut ips);
        assert_eq!(ips, vec![parse("192.168.1.10"), parse("100.64.1.2")]);
    }

    /// Sends a real SSDP search from each LAN interface (`cargo test -- --ignored`).
    #[tokio::test]
    #[ignore]
    async fn ssdp_search_runs_on_every_interface() {
        for local in local_addresses().lan_v4 {
            let gateways = ssdp_gateways(&local).await;
            println!("{} -> {:?}", local.ip, gateways);
        }
    }

    #[test]
    fn subnet_match() {
        let local = LocalV4 {
            ip: "192.168.1.10".parse().unwrap(),
            netmask: "255.255.255.0".parse().unwrap(),
        };
        assert!(same_subnet(&local, "192.168.1.1".parse().unwrap()));
        assert!(!same_subnet(&local, "192.168.2.1".parse().unwrap()));
    }
}
