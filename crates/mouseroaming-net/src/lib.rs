pub mod discovery;
pub mod tls;
pub mod transport;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("certificate generation error: {0}")]
    CertGen(#[from] rcgen::Error),
    #[error("protocol error: {0}")]
    Proto(#[from] mouseroaming_proto::ProtoError),
    #[error("mDNS error: {0}")]
    Mdns(String),
    #[error("peer certificate rejected: {0}")]
    UntrustedPeer(String),
    #[error("invalid server name or address: {0}")]
    InvalidAddress(String),
    #[error("connection closed")]
    Closed,
}

pub use discovery::{DiscoveredPeer, DiscoveryEvent, MdnsService, MDNS_SERVICE_TYPE};
pub use tls::{
    build_client_config, build_server_config, cert_fingerprint_hex, ensure_crypto_provider,
    format_short_fingerprint, generate_ephemeral_identity, load_or_generate_identity,
    normalize_fingerprint, NodeIdentity, TrustPolicy,
};
pub use transport::{
    connect_tls, MessageConnection, MessageReader, MessageWriter, TlsServerListener,
};
