use anyhow::Result;
use directories::ProjectDirs;
use mouseroaming_proto::{Edge, KeyMappingConfig, DEFAULT_PORT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppRole {
    /// Shares this machine's physical mouse & keyboard with connected clients.
    Server,
    /// Connects to a Server and receives mouse/keyboard input from it.
    Client,
}

impl AppRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Server => "Server (主控端 / 分享滑鼠)",
            Self::Client => "Client (受控端 / 接收滑鼠)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub peer_id: String,
    pub hostname: String,
    pub role: AppRole,
    pub listen_port: u16,
    pub connect_address: String,
    pub pairing_token: String,
    pub default_client_edge: Edge,
    pub dead_corner_px: i32,
    pub edge_switch_cooldown_ms: u64,
    pub clipboard_sync_enabled: bool,
    pub key_mapping: KeyMappingConfig,
    pub start_minimized: bool,
    pub auto_start_service: bool,
    pub trusted_fingerprints: HashMap<String, String>,
    pub edge_bindings: HashMap<Edge, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let host = hostname::get()
            .ok()
            .and_then(|s| s.into_string().ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "MouseRoaming-PC".to_string());
        Self {
            peer_id: Uuid::new_v4().to_string(),
            hostname: host,
            role: AppRole::Server,
            listen_port: DEFAULT_PORT,
            connect_address: format!("127.0.0.1:{DEFAULT_PORT}"),
            pairing_token: String::new(),
            default_client_edge: Edge::Right,
            dead_corner_px: 24,
            edge_switch_cooldown_ms: 150,
            clipboard_sync_enabled: true,
            key_mapping: KeyMappingConfig::default(),
            start_minimized: false,
            auto_start_service: true,
            trusted_fingerprints: HashMap::new(),
            edge_bindings: HashMap::new(),
        }
    }
}

impl AppConfig {
    pub fn default_config_dir() -> PathBuf {
        if let Some(proj) = ProjectDirs::from("io", "mouseroaming", "MouseRoaming") {
            proj.config_dir().to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".mouseroaming")
        }
    }

    pub fn load_or_default(dir: &Path) -> Self {
        let path = dir.join("config.toml");
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&content) {
                return cfg;
            }
        }
        let cfg = Self::default();
        let _ = cfg.save(dir);
        cfg
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let path = dir.join("config.toml");
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}
