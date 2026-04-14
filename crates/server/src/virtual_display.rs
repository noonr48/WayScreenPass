//! Headless Wayland session management.
//!
//! The first implementation target is a dedicated sway session running on a
//! headless wlroots backend. That gives the server a real Wayland desktop on
//! the same machine while avoiding portal prompts for screen capture.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

static COMPOSITOR_RUNNING: AtomicBool = AtomicBool::new(false);
const SESSION_METADATA_FILE: &str = "remote-desktop-headless-session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadlessSessionInfo {
    pub compositor: String,
    pub runtime_dir: String,
    pub wayland_display: String,
    pub sway_socket: String,
    pub output_name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
}

impl HeadlessSessionInfo {
    pub fn apply_to_command(&self, cmd: &mut Command) {
        cmd.env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("WAYLAND_DISPLAY", &self.wayland_display)
            .env("SWAYSOCK", &self.sway_socket)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env("GDK_BACKEND", "wayland")
            .env("QT_QPA_PLATFORM", "wayland")
            .env_remove("DISPLAY");
    }
}

/// Headless session configuration and process ownership.
pub struct VirtualDisplay {
    width: u32,
    height: u32,
    refresh_rate: u32,
    compositor: Option<Child>,
    session: Option<HeadlessSessionInfo>,
}

impl VirtualDisplay {
    pub fn new(width: u32, height: u32, refresh_rate: u32) -> Self {
        Self {
            width,
            height,
            refresh_rate,
            compositor: None,
            session: None,
        }
    }

    /// Start the dedicated headless sway session and persist its metadata so
    /// other commands can launch applications into it.
    pub fn start(&mut self) -> Result<HeadlessSessionInfo> {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .context("XDG_RUNTIME_DIR is required to run the headless Wayland session")?;
        let runtime_path = Path::new(&runtime_dir);

        let sway_path = which::which("sway")
            .context("Headless mode requires sway to be installed")?;
        let before_wayland = list_wayland_sockets(runtime_path)?;
        let config_path = self.write_sway_config()?;

        info!(
            "Starting headless sway session ({}x{}@{}Hz)",
            self.width, self.height, self.refresh_rate
        );

        let mut cmd = Command::new(&sway_path);
        cmd.env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env_remove("DISPLAY")
            .arg("--unsupported-gpu")
            .arg("--config")
            .arg(&config_path);

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to start sway via {:?}", sway_path))?;

        let sway_socket = format!(
            "{}/sway-ipc.{}.{}.sock",
            runtime_dir,
            unsafe { libc::geteuid() },
            child.id()
        );

        self.compositor = Some(child);
        wait_for_path(
            Path::new(&sway_socket),
            self.compositor.as_mut(),
            Duration::from_secs(5),
            "sway IPC socket",
        )?;

        let wayland_display = wait_for_wayland_socket(
            runtime_path,
            &before_wayland,
            self.compositor.as_mut(),
            Duration::from_secs(5),
        )?;

        let output_name = self.configure_sway_headless_output(&sway_socket)?;

        let session = HeadlessSessionInfo {
            compositor: "sway".to_string(),
            runtime_dir,
            wayland_display,
            sway_socket,
            output_name,
            width: self.width,
            height: self.height,
            refresh_rate: self.refresh_rate,
        };

        write_session_metadata(&session)?;
        self.session = Some(session.clone());
        COMPOSITOR_RUNNING.store(true, Ordering::SeqCst);

        info!(
            "Headless sway session ready on {} ({})",
            session.wayland_display, session.output_name
        );

        Ok(session)
    }

    fn write_sway_config(&self) -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("remote-desktop");
        fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("sway_headless.conf");
        let config = r#"
focus_follows_mouse no
default_border pixel 1
exec_always dbus-update-activation-environment --systemd WAYLAND_DISPLAY SWAYSOCK XDG_CURRENT_DESKTOP=sway
"#;

        fs::write(&config_path, config)?;
        Ok(config_path)
    }

    fn configure_sway_headless_output(&mut self, sway_socket: &str) -> Result<String> {
        run_swaymsg(sway_socket, &["create_output"])
            .context("Failed to create headless sway output")?;

        let output_name = wait_for_headless_output(
            sway_socket,
            self.compositor.as_mut(),
            Duration::from_secs(5),
        )?;

        let mode = format!("{}x{}@{}Hz", self.width, self.height, self.refresh_rate);
        run_swaymsg(sway_socket, &["output", &output_name, "resolution", &mode]).with_context(
            || format!("Failed to set sway output {} to {}", output_name, mode),
        )?;

        run_swaymsg(sway_socket, &["workspace", "1", "output", &output_name]).with_context(
            || format!("Failed to bind workspace 1 to {}", output_name),
        )?;

        Ok(output_name)
    }

    #[allow(dead_code)]
    pub fn session(&self) -> Option<&HeadlessSessionInfo> {
        self.session.as_ref()
    }

    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.compositor {
            match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    warn!("Headless compositor exited with status: {}", status);
                    false
                }
                Err(e) => {
                    warn!("Error checking compositor status: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        let _ = remove_session_metadata();

        if let Some(ref mut child) = self.compositor {
            info!("Stopping headless sway session...");
            let _ = child.kill();
            let _ = child.wait();
        }

        COMPOSITOR_RUNNING.store(false, Ordering::SeqCst);
    }
}

pub fn check_compositor_available() -> Result<String> {
    which::which("sway").context("Install sway for the dedicated headless session runtime")?;
    which::which("swaymsg").context("Install swaymsg alongside sway for runtime control")?;
    which::which("grim").context("Install grim for headless session capture")?;
    Ok("sway".to_string())
}

#[allow(dead_code)]
pub fn list_available_compositors() -> Vec<&'static str> {
    let mut available = Vec::new();

    if which::which("sway").is_ok() && which::which("grim").is_ok() {
        available.push("sway (recommended)");
    }

    available
}

pub fn session_metadata_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    Ok(Path::new(&runtime_dir).join(SESSION_METADATA_FILE))
}

pub fn read_session_metadata() -> Result<Option<HeadlessSessionInfo>> {
    let path = session_metadata_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(&path)
        .with_context(|| format!("Failed to read headless session metadata from {:?}", path))?;
    let session = serde_json::from_slice(&data)
        .with_context(|| format!("Failed to parse headless session metadata in {:?}", path))?;

    Ok(Some(session))
}

pub fn write_session_metadata(session: &HeadlessSessionInfo) -> Result<()> {
    let path = session_metadata_path()?;
    let data = serde_json::to_vec_pretty(session)?;
    fs::write(&path, data)
        .with_context(|| format!("Failed to write headless session metadata to {:?}", path))?;
    Ok(())
}

pub fn remove_session_metadata() -> Result<()> {
    let path = session_metadata_path()?;
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove headless session metadata at {:?}", path))?;
    }
    Ok(())
}

pub fn launch_in_headless_session(command: &[String]) -> Result<()> {
    if command.is_empty() {
        return Err(anyhow!("No command provided to launch"));
    }

    let session = read_session_metadata()?
        .ok_or_else(|| anyhow!("No active headless session found. Start the server first."))?;

    let mut child = Command::new(&command[0]);
    child.args(&command[1..]);
    session.apply_to_command(&mut child);

    let child = child
        .spawn()
        .with_context(|| format!("Failed to launch {:?} in the headless session", command))?;

    info!(
        "Launched {:?} in headless session on {} (pid={})",
        command,
        session.output_name,
        child.id()
    );

    Ok(())
}

fn list_wayland_sockets(runtime_dir: &Path) -> Result<BTreeSet<String>> {
    let mut sockets = BTreeSet::new();
    for entry in fs::read_dir(runtime_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("wayland-") {
            sockets.insert(name);
        }
    }
    Ok(sockets)
}

fn wait_for_wayland_socket(
    runtime_dir: &Path,
    before: &BTreeSet<String>,
    compositor: Option<&mut Child>,
    timeout: Duration,
) -> Result<String> {
    let start = Instant::now();
    let mut compositor = compositor;

    loop {
        let current = list_wayland_sockets(runtime_dir)?;
        if let Some(socket) = current.iter().find(|socket| !before.contains(socket.as_str())) {
            return Ok(socket.clone());
        }

        if let Some(child) = compositor.as_mut() {
            if let Some(status) = child.try_wait()? {
                return Err(anyhow!(
                    "Headless compositor exited before creating a Wayland socket: {}",
                    status
                ));
            }
        }

        if start.elapsed() >= timeout {
            return Err(anyhow!("Timed out waiting for the headless Wayland socket"));
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_path(
    path: &Path,
    compositor: Option<&mut Child>,
    timeout: Duration,
    description: &str,
) -> Result<()> {
    let start = Instant::now();
    let mut compositor = compositor;

    loop {
        if path.exists() {
            return Ok(());
        }

        if let Some(child) = compositor.as_mut() {
            if let Some(status) = child.try_wait()? {
                return Err(anyhow!(
                    "Headless compositor exited before {} became available: {}",
                    description,
                    status
                ));
            }
        }

        if start.elapsed() >= timeout {
            return Err(anyhow!("Timed out waiting for {}", description));
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_headless_output(
    sway_socket: &str,
    compositor: Option<&mut Child>,
    timeout: Duration,
) -> Result<String> {
    let start = Instant::now();
    let mut compositor = compositor;

    loop {
        if let Ok(outputs) = get_sway_outputs(sway_socket) {
            if let Some(output) = outputs
                .into_iter()
                .find(|output| output.name.starts_with("HEADLESS-"))
            {
                return Ok(output.name);
            }
        }

        if let Some(child) = compositor.as_mut() {
            if let Some(status) = child.try_wait()? {
                return Err(anyhow!(
                    "Headless compositor exited before exposing a headless output: {}",
                    status
                ));
            }
        }

        if start.elapsed() >= timeout {
            return Err(anyhow!("Timed out waiting for sway to expose HEADLESS-* output"));
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_swaymsg(sway_socket: &str, args: &[&str]) -> Result<()> {
    let swaymsg = which::which("swaymsg")
        .context("swaymsg is required to control the headless sway session")?;

    debug!("Running swaymsg {:?}", args);
    let output = Command::new(swaymsg)
        .args(["--socket", sway_socket])
        .args(args)
        .output()
        .with_context(|| format!("Failed to run swaymsg {:?}", args))?;

    if output.status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "swaymsg {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[derive(Debug, Deserialize)]
struct SwayOutput {
    name: String,
}

fn get_sway_outputs(sway_socket: &str) -> Result<Vec<SwayOutput>> {
    let swaymsg = which::which("swaymsg")
        .context("swaymsg is required to query headless sway outputs")?;
    let output = Command::new(swaymsg)
        .args(["--socket", sway_socket, "-r", "-t", "get_outputs"])
        .output()
        .context("Failed to query sway outputs")?;

    if !output.status.success() {
        return Err(anyhow!(
            "swaymsg get_outputs failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let outputs = serde_json::from_slice(&output.stdout)
        .context("Failed to parse sway output list")?;
    Ok(outputs)
}
