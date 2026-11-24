use crate::NetError;
use rcgen::{CertificateParams, DnType, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex, Once};

static CRYPTO_INIT: Once = Once::new();

pub fn ensure_crypto_provider() {
    CRYPTO_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[derive(Debug, Clone)]
pub struct NodeIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

impl NodeIdentity {
    pub fn short_fingerprint(&self) -> String {
        format_short_fingerprint(&self.fingerprint)
    }

    pub fn cert_chain(&self) -> Vec<CertificateDer<'static>> {
        vec![CertificateDer::from(self.cert_der.clone())]
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key_der.clone().into())
    }
}

pub fn cert_fingerprint_hex(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    hex::encode_upper(digest)
}

pub fn normalize_fingerprint(fp: &str) -> String {
    fp.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase()
}

pub fn format_short_fingerprint(fp: &str) -> String {
    let norm = normalize_fingerprint(fp);
    let prefix: String = norm.chars().take(12).collect();
    prefix
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

pub fn generate_ephemeral_identity(hostname: &str) -> Result<NodeIdentity, NetError> {
    let mut params = CertificateParams::new(vec![
        "localhost".to_string(),
        hostname.to_string(),
        "mouseroaming.local".to_string(),
    ])?;
    params
        .distinguished_name
        .push(DnType::CommonName, format!("MouseRoaming ({hostname})"));

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();
    let fingerprint = cert_fingerprint_hex(&cert_der);

    Ok(NodeIdentity {
        cert_der,
        key_der,
        fingerprint,
    })
}

pub fn load_or_generate_identity(dir: &Path, hostname: &str) -> Result<NodeIdentity, NetError> {
    fs::create_dir_all(dir)?;
    let cert_path = dir.join("node_cert.pem");
    let key_path = dir.join("node_key.pem");

    if cert_path.exists() && key_path.exists() {
        let cert_pem = fs::read(&cert_path)?;
        let key_pem = fs::read(&key_path)?;
        let mut cert_cursor = Cursor::new(cert_pem);
        let certs: Vec<_> = rustls_pemfile::certs(&mut cert_cursor)
            .collect::<Result<Vec<_>, _>>()?;
        let mut key_cursor = Cursor::new(key_pem);
        let key = rustls_pemfile::private_key(&mut key_cursor)?;

        if let (Some(cert), Some(key)) = (certs.into_iter().next(), key) {
            let cert_der = cert.as_ref().to_vec();
            let key_der = key.secret_der().to_vec();
            let fingerprint = cert_fingerprint_hex(&cert_der);
            return Ok(NodeIdentity {
                cert_der,
                key_der,
                fingerprint,
            });
        }
    }

    let mut params = CertificateParams::new(vec![
        "localhost".to_string(),
        hostname.to_string(),
        "mouseroaming.local".to_string(),
    ])?;
    params
        .distinguished_name
        .push(DnType::CommonName, format!("MouseRoaming ({hostname})"));

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    fs::write(&cert_path, cert.pem())?;
    fs::write(&key_path, key_pair.serialize_pem())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    }

    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();
    let fingerprint = cert_fingerprint_hex(&cert_der);

    Ok(NodeIdentity {
        cert_der,
        key_der,
        fingerprint,
    })
}

#[derive(Debug, Clone)]
pub enum TrustPolicy {
    /// Trust On First Use: accepts any unseen certificate and records its fingerprint.
    /// If `pinned` contains a fingerprint, requires exact match.
    TrustOnFirstUse {
        expected_fingerprint: Option<String>,
    },
    /// Strict mode: only accepts certificates whose SHA-256 fingerprint is in the trusted set.
    StrictTrustedSet(Arc<Mutex<HashSet<String>>>),
}

#[derive(Debug)]
struct TofuCertVerifier {
    policy: TrustPolicy,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for TofuCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let actual_fp = cert_fingerprint_hex(end_entity.as_ref());
        if let Ok(mut guard) = self.observed_fingerprint.lock() {
            *guard = Some(actual_fp.clone());
        }

        match &self.policy {
            TrustPolicy::TrustOnFirstUse {
                expected_fingerprint,
            } => {
                if let Some(expected) = expected_fingerprint {
                    let norm_expected = normalize_fingerprint(expected);
                    if norm_expected != actual_fp {
                        return Err(rustls::Error::General(format!(
                            "TLS fingerprint mismatch: expected {norm_expected}, got {actual_fp}"
                        )));
                    }
                }
                Ok(ServerCertVerified::assertion())
            }
            TrustPolicy::StrictTrustedSet(set) => {
                let guard = set
                    .lock()
                    .map_err(|_| rustls::Error::General("trust set poisoned".into()))?;
                if guard.contains(&actual_fp) {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General(format!(
                        "untrusted peer TLS fingerprint: {actual_fp}"
                    )))
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn build_server_config(identity: &NodeIdentity) -> Result<Arc<rustls::ServerConfig>, NetError> {
    ensure_crypto_provider();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(identity.cert_chain(), identity.private_key())?;
    Ok(Arc::new(config))
}

pub fn build_client_config(
    policy: TrustPolicy,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
) -> Result<Arc<rustls::ClientConfig>, NetError> {
    ensure_crypto_provider();
    let verifier = Arc::new(TofuCertVerifier {
        policy,
        observed_fingerprint,
    });
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reloads_identity_with_same_fingerprint() {
        let dir = std::env::temp_dir().join(format!(
            "mouseroaming_tls_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let id1 = load_or_generate_identity(&dir, "test-host").expect("generate");
        let id2 = load_or_generate_identity(&dir, "test-host").expect("reload");

        assert_eq!(id1.fingerprint, id2.fingerprint);
        assert_eq!(id1.short_fingerprint().len(), 14); // XXXX-XXXX-XXXX

        let _ = fs::remove_dir_all(&dir);
    }
}
