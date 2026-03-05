//! Virtual Display Module
//!
//! Creates a headless Wayland compositor with virtual output for remote desktop
//! without requiring portal permissions.

use anyhow::{Result, anyhow};
use std::process::{Command, Child};
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn, debug};

static COMPOSITOR_RUNNING: AtomicBool = AtomicBool::new(false);

/// Virtual display configuration
pub struct VirtualDisplay {
    width: u32,
    height: u32,
    refresh_rate: u32,
    compositor: Option<Child>,
    wayland_display: Option<String>,
}

impl VirtualDisplay {
    /// Create a new virtual display configuration
    pub fn new(width: u32, height: u32, refresh_rate: u32) -> Self {
        Self {
            width,
            height,
            refresh_rate,
            compositor: None,
            wayland_display: None,
        }
    }

    /// Start the virtual display (headless compositor)
    pub fn start(&mut self) -> Result<String> {
        info!("Starting virtual display ({}x{}@{}Hz)", self.width, self.height, self.refresh_rate);

        // Generate a unique WAYLAND_DISPLAY name
        let display_num = std::process::id() % 1000;
        let wayland_display = format!("wayland-{}", display_num);

        // Try different compositors in order of preference
        let result = self.try_start_cage(&wayland_display)
            .or_else(|_| self.try_start_weston(&wayland_display))
            .or_else(|_| self.try_start_sway(&wayland_display));

        match result {
            Ok(()) => {
                self.wayland_display = Some(wayland_display.clone());
                COMPOSITOR_RUNNING.store(true, Ordering::SeqCst);
                info!("Virtual display started on {}", wayland_display);
                Ok(wayland_display)
            }
            Err(e) => {
                Err(anyhow!("Failed to start any headless compositor: {}. Install cage, weston, or sway.", e))
            }
        }
    }

    /// Try to start cage (simple kiosk compositor)
    fn try_start_cage(&mut self, wayland_display: &str) -> Result<()> {
        // Check if cage is installed
        let cage_path = which::which("cage")
            .map_err(|_| anyhow!("cage not found"))?;

        info!("Found cage at {:?}", cage_path);

        // Start cage in headless mode with a dummy client
        let mut cmd = Command::new(&cage_path);
        cmd.env("WAYLAND_DISPLAY", wayland_display);

        // cage needs a client to run - we'll use a simple sleep or wayland-info
        // For headless operation, we run cage with -- backend flag if available
        // Recent versions of cage support headless via WLR_BACKENDS
        cmd.env("WLR_BACKENDS", "headless");
        cmd.env("WLR_LIBINPUT_NO_DEVICES", "1");

        // Set the resolution via cage arguments or environment
        cmd.arg("--");

        // Run a simple client that keeps the compositor alive
        // We'll use sleep infinity or a simple wayland client
        cmd.args(["sleep", "infinity"]);

        info!("Starting cage headless compositor...");
        let child = cmd.spawn()
            .map_err(|e| anyhow!("Failed to start cage: {}", e))?;

        self.compositor = Some(child);

        // Wait a moment for the compositor to start
        std::thread::sleep(std::time::Duration::from_millis(500));

        Ok(())
    }

    /// Try to start weston (reference Wayland compositor)
    fn try_start_weston(&mut self, wayland_display: &str) -> Result<()> {
        let weston_path = which::which("weston")
            .map_err(|_| anyhow!("weston not found"))?;

        info!("Found weston at {:?}", weston_path);

        // Create a weston.ini for headless operation
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("remote-desktop");

        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("weston.ini");

        let config_content = format!(r#"[core]
backend=headless-backend.so
shell=desktop-shell.so

[output]
name=virtual
mode={}x{}
"#, self.width, self.height);

        std::fs::write(&config_path, config_content)?;

        let mut cmd = Command::new(&weston_path);
        cmd.env("WAYLAND_DISPLAY", wayland_display);
        cmd.arg("--config");
        cmd.arg(&config_path);
        cmd.arg("--socket");
        cmd.arg(wayland_display);

        info!("Starting weston headless compositor...");
        let child = cmd.spawn()
            .map_err(|e| anyhow!("Failed to start weston: {}", e))?;

        self.compositor = Some(child);

        // Wait for weston to initialize
        std::thread::sleep(std::time::Duration::from_millis(1000));

        Ok(())
    }

    /// Try to start sway (i3-compatible Wayland compositor)
    fn try_start_sway(&mut self, wayland_display: &str) -> Result<()> {
        let sway_path = which::which("sway")
            .map_err(|_| anyhow!("sway not found"))?;

        info!("Found sway at {:?}", sway_path);

        // Create a minimal sway config
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("remote-desktop");

        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("sway_config");

        let config_content = r#"# Minimal sway config for virtual display
# Disable workspace auto_back_and_forth
workspace_auto_back_and_forth no

# No bars or decorations
default_border none
default_floating_border none

# Don't show window titles
font pango:monospace 0

# Exit handler
bindsym Mod4+Shift+e exec swaynag -t warning -m 'Exit sway?' -b 'Yes' 'swaymsg exit'
"#;

        std::fs::write(&config_path, config_content)?;

        let mut cmd = Command::new(&sway_path);
        cmd.env("WAYLAND_DISPLAY", wayland_display);
        cmd.env("WLR_BACKENDS", "headless");
        cmd.env("WLR_LIBINPUT_NO_DEVICES", "1");
        cmd.arg("--config");
        cmd.arg(&config_path);

        info!("Starting sway headless compositor...");
        let child = cmd.spawn()
            .map_err(|e| anyhow!("Failed to start sway: {}", e))?;

        self.compositor = Some(child);

        // Wait for sway to initialize and create virtual output
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // Create a virtual output in sway
        if let Ok(swaymsg) = which::which("swaymsg") {
            let output_cmd = Command::new(swaymsg)
                .env("WAYLAND_DISPLAY", wayland_display)
                .args(["output", &format!("create mode {}x{}@{}Hz",
                    self.width, self.height, self.refresh_rate)])
                .output();

            match output_cmd {
                Ok(o) if o.status.success() => {
                    info!("Created virtual output in sway");
                }
                _ => {
                    warn!("Could not create virtual output in sway, using default");
                }
            }
        }

        Ok(())
    }

    /// Get the WAYLAND_DISPLAY for this virtual display
    pub fn wayland_display(&self) -> Option<&str> {
        self.wayland_display.as_deref()
    }

    /// Check if the compositor is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.compositor {
            match child.try_wait() {
                Ok(None) => true, // Still running
                Ok(Some(status)) => {
                    warn!("Compositor exited with status: {}", status);
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

    /// Get the display dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.compositor {
            info!("Stopping virtual display compositor...");
            let _ = child.kill();
            let _ = child.wait();
        }
        COMPOSITOR_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Check if any supported headless compositor is available
pub fn check_compositor_available() -> Result<String> {
    if which::which("cage").is_ok() {
        return Ok("cage".to_string());
    }
    if which::which("weston").is_ok() {
        return Ok("weston".to_string());
    }
    if which::which("sway").is_ok() {
        return Ok("sway".to_string());
    }

    Err(anyhow!("No headless Wayland compositor found. Install one of: cage, weston, or sway"))
}

/// List available compositors
pub fn list_available_compositors() -> Vec<&'static str> {
    let mut available = Vec::new();

    if which::which("cage").is_ok() {
        available.push("cage (recommended)");
    }
    if which::which("weston").is_ok() {
        available.push("weston");
    }
    if which::which("sway").is_ok() {
        available.push("sway");
    }

    available
}
