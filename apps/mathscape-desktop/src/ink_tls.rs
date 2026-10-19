use axum_server::tls_rustls::RustlsConfig;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, net::Ipv4Addr, path::Path};
use time::{Duration, OffsetDateTime};

/// N3.0: how a desktop address is presented to the user. Ordering of the
/// variants is the display order: LAN addresses first, loopback last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressKind {
    /// RFC 1918 address on an ordinary interface; the only kind used by default.
    Lan,
    /// CGNAT 100.64/10 (Tailscale and similar) or a tunnel interface (utun, wg, tun, ppp...).
    Vpn,
    /// Docker, bridge, veth, VM host-only and similar virtual interfaces.
    Virtual,
    /// 169.254/16.
    LinkLocal,
    /// Anything else, including public addresses.
    Public,
    /// 127.0.0.1 and localhost, for testing on this computer.
    Loopback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostAddress {
    pub host: String,
    pub kind: AddressKind,
}

const VIRTUAL_PREFIXES: &[&str] = &[
    "docker",
    "br-",
    "veth",
    "virbr",
    "vmnet",
    "vboxnet",
    "cni",
    "flannel",
    "cali",
    "lxc",
    "lxd",
    "podman",
    "bridge",
    "vnic",
    "vethernet",
    "kube",
    "weave",
    "anpi",
    "ap1",
];
const VPN_PREFIXES: &[&str] = &[
    "utun",
    "tun",
    "tap",
    "wg",
    "tailscale",
    "zt",
    "ppp",
    "ipsec",
    "gif",
    "stf",
    "nordlynx",
    "proton",
    "mullvad",
    "wireguard",
    "openvpn",
];

/// Classify one IPv4 interface address. Interface-name patterns win over the
/// address range, so a Docker bridge on 172.17/16 is not offered as LAN.
pub(crate) fn classify(interface: &str, ip: Ipv4Addr) -> AddressKind {
    let name = interface.to_ascii_lowercase();
    if ip.is_loopback() {
        return AddressKind::Loopback;
    }
    if ip.is_link_local() {
        return AddressKind::LinkLocal;
    }
    let [a, b, ..] = ip.octets();
    if a == 100 && (64..128).contains(&b) {
        return AddressKind::Vpn;
    }
    if VPN_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) {
        return AddressKind::Vpn;
    }
    if VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return AddressKind::Virtual;
    }
    if ip.is_private() {
        return AddressKind::Lan;
    }
    AddressKind::Public
}

/// Classify, deduplicate and sort interface addresses: LAN first with the
/// default-route egress address leading, then the other kinds, and finally
/// 127.0.0.1 and localhost. Everything stays in the list (certificate SANs and
/// the Host allow-list use all of it); only the UI hides non-LAN entries.
pub(crate) fn order_addresses(
    interfaces: impl IntoIterator<Item = (String, Ipv4Addr)>,
    egress: Option<Ipv4Addr>,
) -> Vec<HostAddress> {
    let mut entries = interfaces
        .into_iter()
        .filter(|(_, ip)| !ip.is_loopback() && !ip.is_unspecified())
        .map(|(name, ip)| (classify(&name, ip), Some(ip) != egress, ip))
        .collect::<Vec<_>>();
    entries.sort();
    let mut seen = std::collections::HashSet::new();
    let mut values = entries
        .into_iter()
        .filter(|(_, _, ip)| seen.insert(*ip))
        .map(|(kind, _, ip)| HostAddress {
            host: ip.to_string(),
            kind,
        })
        .collect::<Vec<_>>();
    for host in [Ipv4Addr::LOCALHOST.to_string(), "localhost".into()] {
        values.push(HostAddress {
            host,
            kind: AddressKind::Loopback,
        });
    }
    values
}

/// IPv4 source address of the default route. A UDP `connect` only asks the
/// kernel for a route; no packet is sent. TEST-NET-1 is never contacted.
fn egress_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

pub(crate) fn host_addresses() -> Vec<HostAddress> {
    let interfaces = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            std::net::IpAddr::V4(ip) => Some((interface.name, ip)),
            _ => None,
        })
        .collect::<Vec<_>>();
    order_addresses(interfaces, egress_ipv4())
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
pub(crate) fn certificate(root: &Path) -> Result<(String, String), String> {
    let dir = root.join("ink-lan/trust");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let cert = dir.join("ca.crt");
    let key = dir.join("ca-key.pem");
    if cert.exists() != key.exists() {
        return Err("InkPad 信任资料不完整，请保留现有文件并检查 Workspace 恢复情况。".into());
    }
    if !cert.exists() {
        let keypair = KeyPair::generate().map_err(|e| e.to_string())?;
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        params
            .distinguished_name
            .push(DnType::CommonName, "Mathscape InkPad Local Trust");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
        let generated = params.self_signed(&keypair).map_err(|e| e.to_string())?;
        private_write(&key, keypair.serialize_pem().as_bytes())?;
        private_write(&cert, generated.pem().as_bytes())?;
    }
    let pem = fs::read_to_string(cert).map_err(|e| e.to_string())?;
    let key = fs::read_to_string(key).map_err(|e| e.to_string())?;
    let pair = KeyPair::from_pem(&key).map_err(|e| e.to_string())?;
    let _ = Issuer::from_ca_cert_pem(&pem, pair).map_err(|e| e.to_string())?;
    let fingerprint = format!("{:x}", Sha256::digest(pem.as_bytes()));
    Ok((pem, fingerprint))
}
pub(crate) async fn config(root: &Path, hosts: Vec<String>) -> Result<RustlsConfig, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (ca, _) = certificate(root)?;
    let key = KeyPair::from_pem(
        &fs::read_to_string(root.join("ink-lan/trust/ca-key.pem")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let issuer = Issuer::from_ca_cert_pem(&ca, key).map_err(|e| e.to_string())?;
    let leaf_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut params = CertificateParams::new(hosts).map_err(|e| e.to_string())?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Mathscape InkPad");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.not_before = OffsetDateTime::now_utc() - Duration::minutes(5);
    params.not_after = OffsetDateTime::now_utc() + Duration::days(365);
    let leaf = params
        .signed_by(&leaf_key, &issuer)
        .map_err(|e| e.to_string())?;
    RustlsConfig::from_pem(
        format!("{}{}", leaf.pem(), ca).into_bytes(),
        leaf_key.serialize_pem().into_bytes(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> Ipv4Addr {
        value.parse().unwrap()
    }

    #[test]
    fn classifies_lan_vpn_virtual_link_local_and_public_addresses() {
        assert_eq!(classify("en0", ip("192.168.1.20")), AddressKind::Lan);
        assert_eq!(classify("eth0", ip("10.0.0.5")), AddressKind::Lan);
        assert_eq!(classify("wlan0", ip("172.20.1.2")), AddressKind::Lan);
        assert_eq!(classify("en0", ip("100.64.0.1")), AddressKind::Vpn);
        assert_eq!(
            classify("tailscale0", ip("100.101.102.103")),
            AddressKind::Vpn
        );
        assert_eq!(classify("en0", ip("100.128.0.1")), AddressKind::Public);
        assert_eq!(classify("utun4", ip("10.8.0.2")), AddressKind::Vpn);
        assert_eq!(classify("wg0", ip("10.66.0.2")), AddressKind::Vpn);
        assert_eq!(classify("ppp0", ip("192.168.255.2")), AddressKind::Vpn);
        assert_eq!(classify("docker0", ip("172.17.0.1")), AddressKind::Virtual);
        assert_eq!(
            classify("br-1a2b3c", ip("172.18.0.1")),
            AddressKind::Virtual
        );
        assert_eq!(classify("veth12ab", ip("10.1.0.1")), AddressKind::Virtual);
        assert_eq!(
            classify("bridge100", ip("192.168.64.1")),
            AddressKind::Virtual
        );
        assert_eq!(
            classify("vmnet8", ip("192.168.150.1")),
            AddressKind::Virtual
        );
        assert_eq!(
            classify("vEthernet (WSL)", ip("172.29.0.1")),
            AddressKind::Virtual
        );
        assert_eq!(classify("en0", ip("169.254.10.1")), AddressKind::LinkLocal);
        assert_eq!(classify("en0", ip("203.0.113.9")), AddressKind::Public);
        assert_eq!(classify("lo0", ip("127.0.0.1")), AddressKind::Loopback);
    }

    #[test]
    fn orders_lan_first_with_the_egress_address_leading() {
        let ordered = order_addresses(
            [
                ("docker0".into(), ip("172.17.0.1")),
                ("utun3".into(), ip("100.90.1.2")),
                ("en1".into(), ip("10.0.0.9")),
                ("en0".into(), ip("192.168.1.20")),
                ("en0".into(), ip("192.168.1.20")),
                ("en5".into(), ip("169.254.3.3")),
                ("lo0".into(), ip("127.0.0.1")),
                ("en7".into(), ip("8.8.4.4")),
                ("en9".into(), ip("0.0.0.0")),
            ],
            Some(ip("192.168.1.20")),
        );
        let pairs = ordered
            .iter()
            .map(|address| (address.host.as_str(), address.kind))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                ("192.168.1.20", AddressKind::Lan),
                ("10.0.0.9", AddressKind::Lan),
                ("100.90.1.2", AddressKind::Vpn),
                ("172.17.0.1", AddressKind::Virtual),
                ("169.254.3.3", AddressKind::LinkLocal),
                ("8.8.4.4", AddressKind::Public),
                ("127.0.0.1", AddressKind::Loopback),
                ("localhost", AddressKind::Loopback),
            ]
        );
    }

    #[test]
    fn without_an_egress_route_lan_addresses_sort_numerically() {
        let ordered = order_addresses(
            [
                ("en0".into(), ip("192.168.1.20")),
                ("en1".into(), ip("10.0.0.9")),
            ],
            None,
        );
        assert_eq!(ordered[0].host, "10.0.0.9");
        assert_eq!(ordered[1].host, "192.168.1.20");
        assert!(
            order_addresses([], None)
                .iter()
                .all(|address| address.kind == AddressKind::Loopback)
        );
    }

    #[test]
    fn a_vpn_egress_does_not_outrank_lan_addresses() {
        let ordered = order_addresses(
            [
                ("utun4".into(), ip("10.8.0.2")),
                ("en0".into(), ip("192.168.1.20")),
            ],
            Some(ip("10.8.0.2")),
        );
        assert_eq!(ordered[0].host, "192.168.1.20");
        assert_eq!(ordered[1].kind, AddressKind::Vpn);
    }
}
