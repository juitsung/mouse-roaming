#![windows_subsystem = "windows"]

mod gui;
mod os_icons;
mod tray;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use mouseroaming_core::engine::EngineOptions;
use mouseroaming_core::{AppConfig, AppRole, MouseRoamingEngine};
use mouseroaming_os::create_native_backend;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliRole {
    Server,
    Client,
}

#[derive(Debug, Parser)]
#[command(
    name = "mouseroaming",
    version,
    about = "MouseRoaming — 跨 Windows / macOS 區域網路滑鼠鍵盤與剪貼簿共享系統"
)]
struct Args {
    /// Run without opening the GUI window (CLI/daemon mode)
    #[arg(long)]
    headless: bool,

    /// Override role (server or client)
    #[arg(long, value_enum)]
    role: Option<CliRole>,

    /// Override Server listen port
    #[arg(long)]
    port: Option<u16>,

    /// Override Client target address (e.g. 192.168.1.10:24850)
    #[arg(long)]
    connect: Option<String>,

    /// Optional shared pairing token
    #[arg(long)]
    token: Option<String>,

    /// Custom configuration directory
    #[arg(long)]
    config_dir: Option<PathBuf>,

    /// Disable mDNS LAN discovery
    #[arg(long)]
    no_mdns: bool,
}

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    unsafe {
        // If launched from an existing terminal, attach to parent console so CLI flags work
        extern "system" {
            fn AttachConsole(dwProcessId: u32) -> i32;
        }
        AttachConsole(u32::MAX);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let config_dir = args
        .config_dir
        .unwrap_or_else(AppConfig::default_config_dir);
    let mut config = AppConfig::load_or_default(&config_dir);

    if let Some(role) = args.role {
        config.role = match role {
            CliRole::Server => AppRole::Server,
            CliRole::Client => AppRole::Client,
        };
    }
    if let Some(port) = args.port {
        config.listen_port = port;
    }
    if let Some(addr) = args.connect {
        config.connect_address = addr;
        config.role = AppRole::Client;
    }
    if let Some(token) = args.token {
        config.pairing_token = token;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let backend = create_native_backend()?;
    let options = EngineOptions {
        config_dir: config_dir.clone(),
        enable_mdns: !args.no_mdns,
        ..EngineOptions::default()
    };

    if args.headless {
        config.auto_start_service = true;
        let handle = MouseRoamingEngine::spawn(rt.handle(), backend, config, options)?;
        info!("MouseRoaming running in headless mode. Press Ctrl+C to exit.");
        rt.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutting down MouseRoaming...");
            handle.shutdown();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        return Ok(());
    }

    let handle = MouseRoamingEngine::spawn(rt.handle(), backend, config, options)?;

    let app_tray = match tray::AppTray::try_new() {
        Ok(t) => Some(t),
        Err(e) => {
            warn!("System tray unavailable: {e}");
            None
        }
    };

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("MouseRoaming — 跨 Windows / macOS 滑鼠共享")
        .with_inner_size([820.0, 640.0])
        .with_min_inner_size([680.0, 520.0]);

    if let Ok(icon_data) = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png")) {
        viewport = viewport.with_icon(icon_data);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let engine_for_exit = handle.clone();
    eframe::run_native(
        "MouseRoaming",
        native_options,
        Box::new(move |cc| Ok(Box::new(gui::MouseRoamingGui::new(cc, handle, app_tray)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    engine_for_exit.shutdown();
    Ok(())
}
