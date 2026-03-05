//! Remote Desktop System Tray Application
//!
//! A system tray app for managing remote desktop connections.

mod config;
mod connection;
mod tray;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{debug, error, info, warn};

use config::{HostConfig, HostsConfig};
use connection::{connect_to_host, show_notification};
use tray::{RemoteDesktopTray, TrayMessage};

#[derive(Parser)]
#[command(name = "remote-desktop-tray")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to remote-desktop client binary
    #[arg(long, default_value = "remote-desktop")]
    client: String,
}

#[derive(Subcommand)]
enum Commands {
    Add {
        name: String,
        host: String,
        #[arg(short, long, default_value = "3389")]
        port: u16,
    },
    Remove { name: String },
    List,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    match args.command {
        Some(Commands::Add { name, host, port }) => {
            let mut cfg = HostsConfig::load()?;
            cfg.add_host(HostConfig::new(name.clone(), host, Some(port)));
            cfg.save()?;
            println!("Added host: {}", name);
        }
        Some(Commands::Remove { name }) => {
            let mut cfg = HostsConfig::load()?;
            if cfg.remove_host(&name) {
                cfg.save()?;
                println!("Removed host: {}", name);
            } else {
                println!("Host not found: {}", name);
            }
        }
        Some(Commands::List) => {
            let cfg = HostsConfig::load()?;
            if cfg.hosts.is_empty() {
                println!("No saved hosts.");
            } else {
                println!("Saved hosts:");
                for h in &cfg.hosts {
                    println!("  - {} ({})", h.name, h.address());
                }
            }
        }
        None => run_tray(&args.client)?,
    }

    Ok(())
}

fn run_tray(client_binary: &str) -> Result<()> {
    info!("Starting Remote Desktop Tray");

    let cfg = HostsConfig::load()?;
    info!("Loaded {} saved hosts", cfg.hosts.len());

    let (tx, rx) = std::sync::mpsc::channel::<TrayMessage>();
    let hosts = Arc::new(Mutex::new(cfg.hosts));

    let tray = RemoteDesktopTray::new(hosts.clone(), tx, client_binary.to_string());

    // Spawn tray in separate thread (it blocks)
    thread::spawn(move || {
        debug!("Tray service starting");
        ksni::TrayService::new(tray).spawn();
    });

    thread::sleep(std::time::Duration::from_millis(500));
    info!("Tray icon created. Click to access menu.");

    // Event loop
    loop {
        match rx.recv() {
            Ok(TrayMessage::Connect { index }) => {
                let hosts_list = hosts.lock().unwrap().clone();
                if let Some(host) = hosts_list.get(index) {
                    info!("Connecting to: {} ({})", host.name, host.address());
                    match connect_to_host(host, client_binary) {
                        Ok(()) => { show_notification("Remote Desktop", &format!("Connecting to {}...", host.name)).ok(); }
                        Err(e) => {
                            error!("Connection failed: {}", e);
                            show_notification("Remote Desktop Error", &format!("Failed: {}", e)).ok();
                        }
                    }
                } else {
                    warn!("Invalid host index: {}", index);
                }
            }
            Ok(TrayMessage::AddHost) => {
                match show_add_dialog() {
                    Ok(Some((name, hostname, port))) => {
                        hosts.lock().unwrap().push(HostConfig::new(name.clone(), hostname.clone(), port));
                        let mut cfg = HostsConfig::load()?;
                        cfg.add_host(HostConfig::new(name.clone(), hostname, port));
                        cfg.save()?;
                        show_notification("Remote Desktop", &format!("Added host: {}", name)).ok();
                    }
                    Ok(None) => debug!("Add host cancelled"),
                    Err(e) => error!("Dialog error: {}", e),
                }
            }
            Ok(TrayMessage::RemoveHost { index }) => {
                let hosts_list = hosts.lock().unwrap().clone();
                if let Some(host) = hosts_list.get(index) {
                    let name = host.name.clone();
                    hosts.lock().unwrap().remove(index);
                    let mut cfg = HostsConfig::load()?;
                    cfg.remove_host(&name);
                    cfg.save()?;
                    show_notification("Remote Desktop", &format!("Removed host: {}", name)).ok();
                }
            }
            Ok(TrayMessage::Exit) => {
                info!("Exiting");
                break;
            }
            Err(_) => break,
        }
    }

    Ok(())
}

fn show_add_dialog() -> Result<Option<(String, String, Option<u16>)>> {
    use std::process::Command;

    let out = Command::new("zenity")
        .args(["--entry", "--text=Enter display name:", "--title=Add Remote Host"])
        .output()?;
    if !out.status.success() { return Ok(None); }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() { return Ok(None); }

    let out = Command::new("zenity")
        .args(["--entry", "--text=Enter hostname/IP:", "--title=Add Remote Host"])
        .output()?;
    if !out.status.success() { return Ok(None); }
    let hostname = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if hostname.is_empty() { return Ok(None); }

    let out = Command::new("zenity")
        .args(["--entry", "--text=Port (default 3389):", "--title=Add Remote Host", "--entry-text=3389"])
        .output()?;
    let port = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else { None };

    Ok(Some((name, hostname, port)))
}
