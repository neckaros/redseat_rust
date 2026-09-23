//! TLS for direct HTTPS: an SNI-gated certificate resolver that can be swapped at runtime,
//! plus local key and CSR generation (the private key never leaves the server).

use std::sync::{Arc, RwLock};

use rcgen::{Certificate, CertificateParams, DistinguishedName, PKCS_ECDSA_P256_SHA256};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

use crate::error::{RsError, RsResult};

/// A certificate with the DNS names it answers for (taken from the leaf's subjectAltName).
#[derive(Debug, Clone)]
pub struct SniCertificate {
    names: Vec<String>,
    key: Arc<CertifiedKey>,
}

impl SniCertificate {
    /// Loads a PEM chain (leaf first) and the PEM private key that matches the leaf.
    pub fn from_pem(chain_pem: &str, key_pem: &str) -> RsResult<Self> {
        let chain = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RsError::Error(format!("Invalid certificate chain: {e}")))?;
        let leaf = chain
            .first()
            .ok_or_else(|| RsError::Error("Empty certificate chain".to_string()))?;
        let names = dns_names(leaf)?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
            .map_err(|e| RsError::Error(format!("Invalid private key: {e}")))?;
        let signer = rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|e| RsError::Error(format!("Unsupported private key: {e}")))?;
        let certified = CertifiedKey::new(chain, signer);
        certified
            .keys_match()
            .map_err(|e| RsError::Error(format!("Certificate and key don't match: {e}")))?;
        Ok(Self {
            names,
            key: Arc::new(certified),
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    fn matches(&self, server_name: &str) -> bool {
        self.names
            .iter()
            .any(|name| name_matches(name, server_name))
    }
}

fn dns_names(leaf: &CertificateDer<'_>) -> RsResult<Vec<String>> {
    let (_, cert) = parse_x509_certificate(leaf.as_ref())
        .map_err(|e| RsError::Error(format!("Unable to parse certificate: {e}")))?;
    let names = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some(dns.to_ascii_lowercase()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if names.is_empty() {
        return Err(RsError::Error(
            "Certificate has no DNS subjectAltName".to_string(),
        ));
    }
    Ok(names)
}

/// `*.example.com` matches exactly one extra label (`a.example.com`, not `a.b.example.com`).
fn name_matches(pattern: &str, server_name: &str) -> bool {
    let server_name = server_name.trim_end_matches('.').to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(suffix) => server_name
            .strip_suffix(suffix)
            .and_then(|prefix| prefix.strip_suffix('.'))
            .is_some_and(|label| !label.is_empty() && !label.contains('.')),
        None => server_name == pattern,
    }
}

#[derive(Debug, Default)]
struct Certificates {
    direct: Option<SniCertificate>,
    /// The direct certificate replaced by a label rotation, still served for the old label.
    previous_direct: Option<SniCertificate>,
    /// The `<id>-srv.redseat.cloud` certificate, kept until every client uses direct HTTPS.
    legacy: Option<SniCertificate>,
}

/// Serves a certificate only when the client's SNI matches one of its names. Handshakes
/// without SNI (or with a bare IP, which clients never send as SNI) get no certificate, so
/// scanners probing an IP can't link it to the server's label.
#[derive(Debug, Default)]
pub struct SniResolver {
    certificates: RwLock<Certificates>,
}

impl SniResolver {
    pub fn set_legacy(&self, certificate: Option<SniCertificate>) {
        if let Ok(mut certificates) = self.certificates.write() {
            certificates.legacy = certificate;
        }
    }

    /// Hot-swaps the direct certificate. When its names change (label rotation), the previous
    /// one keeps answering for the old label.
    pub fn set_direct(&self, certificate: SniCertificate) {
        if let Ok(mut certificates) = self.certificates.write() {
            let previous = certificates.direct.replace(certificate);
            let renamed = previous.as_ref().is_some_and(|previous| {
                Some(previous.names()) != certificates.direct.as_ref().map(|c| c.names())
            });
            if renamed {
                certificates.previous_direct = previous;
            }
        }
    }

    pub fn has_certificate(&self) -> bool {
        self.certificates
            .read()
            .map(|c| c.direct.is_some() || c.legacy.is_some())
            .unwrap_or(false)
    }

    fn find(&self, server_name: &str) -> Option<Arc<CertifiedKey>> {
        let certificates = self.certificates.read().ok()?;
        let found = [
            &certificates.direct,
            &certificates.previous_direct,
            &certificates.legacy,
        ]
        .into_iter()
        .flatten()
        .find(|certificate| certificate.matches(server_name))
        .map(|certificate| certificate.key.clone());
        found
    }
}

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.find(client_hello.server_name()?)
    }
}

/// Rustls server config using `resolver`, advertising HTTP/2 and HTTP/1.1.
pub fn server_config(resolver: Arc<SniResolver>) -> rustls::ServerConfig {
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}

pub struct CertificateRequest {
    pub csr_pem: String,
    pub key_pem: String,
}

/// A fresh ECDSA P-256 key and a CSR for exactly `name`: one DNS subjectAltName, empty subject.
pub fn create_csr(name: &str) -> RsResult<CertificateRequest> {
    let mut params = CertificateParams::new(vec![name.to_string()]);
    params.alg = &PKCS_ECDSA_P256_SHA256;
    params.distinguished_name = DistinguishedName::new();
    let certificate = Certificate::from_params(params)
        .map_err(|e| RsError::Error(format!("Unable to create key pair: {e}")))?;
    let csr_pem = certificate
        .serialize_request_pem()
        .map_err(|e| RsError::Error(format!("Unable to create CSR: {e}")))?;
    Ok(CertificateRequest {
        csr_pem,
        key_pem: certificate.serialize_private_key_pem(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use x509_parser::prelude::FromDer;

    const NAME: &str = "*.abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud";

    fn self_signed(names: Vec<String>) -> (String, String) {
        let mut params = CertificateParams::new(names);
        params.alg = &PKCS_ECDSA_P256_SHA256;
        let certificate = Certificate::from_params(params).unwrap();
        (
            certificate.serialize_pem().unwrap(),
            certificate.serialize_private_key_pem(),
        )
    }

    #[test]
    fn wildcard_matches_a_single_label() {
        assert!(name_matches(
            NAME,
            "192-168-1-10.abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud"
        ));
        assert!(name_matches(
            NAME,
            "2A01-E0A--10.ABCDEFGHIJKLMNOPQRSTUVWXYZ234567.servers.redseat.cloud."
        ));
        assert!(!name_matches(
            NAME,
            "abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud"
        ));
        assert!(!name_matches(
            NAME,
            "a.b.abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud"
        ));
        assert!(!name_matches(
            NAME,
            "1-2-3-4.otherlabel.servers.redseat.cloud"
        ));
        assert!(name_matches("srv.redseat.cloud", "SRV.redseat.cloud"));
    }

    #[test]
    fn csr_requests_only_the_wildcard_name() {
        let request = create_csr(NAME).unwrap();
        let (_, pem) = x509_parser::pem::parse_x509_pem(request.csr_pem.as_bytes()).unwrap();
        let (_, csr) =
            x509_parser::certification_request::X509CertificationRequest::from_der(&pem.contents)
                .unwrap();
        assert_eq!(csr.certification_request_info.subject.iter().count(), 0);
        let names: Vec<String> = csr
            .requested_extensions()
            .into_iter()
            .flatten()
            .filter_map(|extension| match extension {
                x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) => Some(
                    san.general_names
                        .iter()
                        .map(|name| format!("{name}"))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(names, vec![format!("DNSName({NAME})")]);
    }

    #[test]
    fn resolver_requires_matching_sni() {
        let (chain, key) = self_signed(vec![NAME.to_string()]);
        let resolver = SniResolver::default();
        resolver.set_direct(SniCertificate::from_pem(&chain, &key).unwrap());
        assert!(resolver
            .find("82-64-1-2.abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud")
            .is_some());
        assert!(resolver.find("82.64.1.2").is_none());
        assert!(resolver.find("unrelated.example.com").is_none());
    }

    #[test]
    fn rotation_keeps_serving_the_old_label() {
        let old_name = "*.oldlabel.servers.redseat.cloud";
        let (old_chain, old_key) = self_signed(vec![old_name.to_string()]);
        let (new_chain, new_key) = self_signed(vec![NAME.to_string()]);
        let resolver = SniResolver::default();
        resolver.set_direct(SniCertificate::from_pem(&old_chain, &old_key).unwrap());
        resolver.set_direct(SniCertificate::from_pem(&new_chain, &new_key).unwrap());
        assert!(resolver
            .find("1-2-3-4.oldlabel.servers.redseat.cloud")
            .is_some());
        assert!(resolver
            .find("1-2-3-4.abcdefghijklmnopqrstuvwxyz234567.servers.redseat.cloud")
            .is_some());
    }

    #[test]
    fn mismatched_key_is_rejected() {
        let (chain, _) = self_signed(vec![NAME.to_string()]);
        let (_, other_key) = self_signed(vec![NAME.to_string()]);
        assert!(SniCertificate::from_pem(&chain, &other_key).is_err());
    }
}
