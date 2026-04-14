//! Status Command
//!
//! Display current server status

use crate::virtual_display::read_session_metadata;
use anyhow::{Result, anyhow};
use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::info;

/// Server status information
#[derive(Debug)]
pub struct ServerStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub port: u16,
}

/// Check server status
pub fn get_status() -> Result<ServerStatus> {
    // Check PID file
    let pid_path = get_pid_file_path()?;

    let mut status = ServerStatus {
        running: false,
        pid: None,
        port: 3389, // Default port
    };

    // Try to read PID file
    if pid_path.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                status.pid = Some(pid);

                // Check if process is running
                if is_process_running(pid) {
                    status.running = true;
                } else {
                    // Clean up stale PID file
                    let _ = fs::remove_file(&pid_path);
                    status.pid = None;
                }
            }
        }
    }

    Ok(status)
}

/// Display server status
pub fn display_status() -> Result<()> {
    println!();
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║       Remote Desktop Server Status                         ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    let status = get_status()?;
    let headless_session = read_session_metadata()?;

    if status.running {
        println!("Server Status: 🟢 RUNNING");
        if let Some(pid) = status.pid {
            println!("  PID: {}", pid);
        }
        println!("  Port: {}", status.port);
        println!();

        // Show connection info
        println!("Connection Info:");
        println!("  Address: 0.0.0.0:{}", status.port);

        // Try to get Tailscale IP
        if let Ok(tailscale_ip) = get_tailscale_ip() {
            println!("  Tailscale: {}:{}", tailscale_ip, status.port);
        }
    } else {
        println!("Server Status: 🔴 NOT RUNNING");
        println!();
        println!("To start the server:");
        println!("  remote-desktop-server start");
    }

    println!();

    if let Some(ref session) = headless_session {
        println!("Headless Session:");
        println!("  🟢 {} on {}", session.compositor, session.output_name);
        println!("  Socket: {}", session.wayland_display);
        println!("  Size: {}x{}@{}Hz", session.width, session.height, session.refresh_rate);
        println!("  Launch apps with:");
        println!("    remote-desktop-server launch <command> [args...]");
        println!();
    }

    println!("Input Injection:");
    if headless_session.is_some() {
        println!("  🟢 Wayland virtual keyboard + pointer");
        println!("     Scoped to the dedicated headless session socket");
    } else {
        println!("  ℹ️  Input backend becomes available once the headless session starts");
    }

    println!();

    Ok(())
}

/// Get the PID file path
fn get_pid_file_path() -> Result<std::path::PathBuf> {
    // Use XDG_RUNTIME_DIR if available, otherwise fallback to /tmp
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());

    Ok(Path::new(&runtime_dir).join("remote-desktop-server.pid"))
}

/// Write PID file (called when server starts)
pub fn write_pid_file() -> Result<()> {
    let pid_path = get_pid_file_path()?;
    let pid = std::process::id();
    fs::write(&pid_path, pid.to_string())?;
    info!("Wrote PID file: {:?}", pid_path);
    Ok(())
}

/// Remove PID file (called when server stops)
pub fn remove_pid_file() -> Result<()> {
    let pid_path = get_pid_file_path()?;
    if pid_path.exists() {
        fs::remove_file(&pid_path)?;
        info!("Removed PID file: {:?}", pid_path);
    }
    Ok(())
}

/// Check if a process is running
fn is_process_running(pid: u32) -> bool {
    // Check /proc/<pid>/stat
    let proc_path = format!("/proc/{}/stat", pid);
    Path::new(&proc_path).exists()
}

/// Get Tailscale IP address
fn get_tailscale_ip() -> Result<String> {
    let output = Command::new("tailscale")
        .arg("status")
        .output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("100.") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    return Ok(parts[0].to_string());
                }
            }
        }
    }

    Err(anyhow!("Tailscale IP not found"))
}
