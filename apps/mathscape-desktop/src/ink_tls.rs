use axum_server::tls_rustls::RustlsConfig;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, net::Ipv4Addr, path::Path};
use time::{Duration, OffsetDateTime};

pub(crate) fn addresses() -> Vec<String> {
    let mut values = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => {
                Some(ip.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values.push(Ipv4Addr::LOCALHOST.to_string());
    values.push("localhost".into());
    values
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
