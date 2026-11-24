use crate::NetError;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use mouseroaming_proto::{OsKind, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub const MDNS_SERVICE_TYPE: &str = "_mouseroaming._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub full_name: String,
    pub peer_id: String,
    pub hostname: String,
    pub os: OsKind,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub fingerprint: String,
}

impl DiscoveredPeer {
    pub fn preferred_addr(&self) -> Option<String> {
        let ip = self
            .addresses
            .iter()
            .find(|ip| ip.is_ipv4())
            .or_else(|| self.addresses.first())?;
        Some(match ip {
            IpAddr::V4(v4) => format!("{v4}:{}", self.port),
            IpAddr::V6(v6) => format!("[{v6}]:{}", self.port),
        })
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Found(DiscoveredPeer),
    Removed(String),
}

pub struct MdnsService {
    daemon: ServiceDaemon,
    registered_fullname: Option<String>,
}

impl MdnsService {
    pub fn new() -> Result<Self, NetError> {
        let daemon = ServiceDaemon::new().map_err(|e| NetError::Mdns(e.to_string()))?;
        Ok(Self {
            daemon,
            registered_fullname: None,
        })
    }

    pub fn advertise(
        &mut self,
        peer_id: &str,
        hostname: &str,
        os: OsKind,
        port: u16,
        fingerprint: &str,
    ) -> Result<(), NetError> {
        if let Some(old) = self.registered_fullname.take() {
            let _ = self.daemon.unregister(&old);
        }

        let safe_host: String = hostname
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let instance_name = format!(
            "MouseRoaming-{}-{}",
            safe_host,
            &peer_id[..peer_id.len().min(6)]
        );
        let host_fqdn = format!("{safe_host}.local.");

        let mut props = HashMap::new();
        props.insert("peer_id".to_string(), peer_id.to_string());
        props.insert("hostname".to_string(), hostname.to_string());
        props.insert("os".to_string(), os.as_tag().to_string());
        props.insert("fp".to_string(), fingerprint.to_string());
        props.insert("ver".to_string(), PROTOCOL_VERSION.to_string());

        let info = ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            &instance_name,
            &host_fqdn,
            "",
            port,
            props,
        )
        .map_err(|e| NetError::Mdns(e.to_string()))?
        .enable_addr_auto();

        let fullname = info.get_fullname().to_string();
        self.daemon
            .register(info)
            .map_err(|e| NetError::Mdns(e.to_string()))?;
        self.registered_fullname = Some(fullname);
        Ok(())
    }

    pub fn stop_advertising(&mut self) {
        if let Some(old) = self.registered_fullname.take() {
            let _ = self.daemon.unregister(&old);
        }
    }

    pub fn start_browse(
        &self,
        local_peer_id: String,
    ) -> Result<mpsc::UnboundedReceiver<DiscoveryEvent>, NetError> {
        let receiver = self
            .daemon
            .browse(MDNS_SERVICE_TYPE)
            .map_err(|e| NetError::Mdns(e.to_string()))?;
        let (tx, rx) = mpsc::unbounded_channel();

        std::thread::Builder::new()
            .name("mouseroaming-mdns-browse".into())
            .spawn(move || {
                while let Ok(event) = receiver.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let peer_id = info
                                .get_property_val_str("peer_id")
                                .unwrap_or_default()
                                .to_string();
                            if peer_id == local_peer_id {
                                continue;
                            }
                            let hostname = info
                                .get_property_val_str("hostname")
                                .unwrap_or_else(|| info.get_hostname())
                                .trim_end_matches(".local.")
                                .to_string();
                            let os = OsKind::from_tag(
                                info.get_property_val_str("os").unwrap_or("unknown"),
                            );
                            let fingerprint = info
                                .get_property_val_str("fp")
                                .unwrap_or_default()
                                .to_string();
                            let addresses: Vec<IpAddr> =
                                info.get_addresses().iter().copied().collect();
                            let peer = DiscoveredPeer {
                                full_name: info.get_fullname().to_string(),
                                peer_id,
                                hostname,
                                os,
                                addresses,
                                port: info.get_port(),
                                fingerprint,
                            };
                            debug!("mDNS discovered peer: {:?}", peer);
                            if tx.send(DiscoveryEvent::Found(peer)).is_err() {
                                break;
                            }
                        }
                        ServiceEvent::ServiceRemoved(_ty, fullname) => {
                            if tx.send(DiscoveryEvent::Removed(fullname)).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| NetError::Mdns(e.to_string()))?;

        Ok(rx)
    }

    pub fn shutdown(self) {
        if let Err(e) = self.daemon.shutdown() {
            warn!("mDNS daemon shutdown warning: {e}");
        }
    }
}
