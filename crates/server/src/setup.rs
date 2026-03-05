//! Setup Wizard
//!
//! Interactive first-time setup for remote desktop server

use anyhow::{Result, anyhow};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{PermissionsExt, MetadataExt};
use std::path::Path;
use std::process::Command;
use tracing::info;

/// Setup wizard for first-time configuration
pub struct SetupWizard;

impl SetupWizard {
    /// Run the setup wizard
    pub async fn run(reset: bool, authorize: bool) -> Result<()> {
        println!();
        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║       Remote Desktop Wayland - Setup Wizard                ║");
        println!("╚════════════════════════════════════════════════════════════╝");
        println!();

        if reset {
            println!("🔄 Reset mode: Will reset authorization tokens...");
            Self::clear_portal_token()?;
            println!();
        }

        let mut all_passed = true;

        // 1. Check uinput permissions
        if !Self::check_uinput()? {
            all_passed = false;
        }

        // 2. Check Tailscale
        if !Self::check_tailscale()? {
            all_passed = false;
        }

        // 3. Check portal
        if !Self::check_portal()? {
            all_passed = false;
        }

        // 4. Check wl-clipboard (for clipboard sync)
        if !Self::check_wl_clipboard()? {
            all_passed = false;
        }

        // 5. Check x264 encoder
        if !Self::check_x264()? {
            all_passed = false;
        }

        // 6. Pre-authorize screen capture if requested
        if authorize {
            println!();
            if !Self::authorize_screen_capture().await? {
                all_passed = false;
            }
        }

        // 7. Ask about systemd service
        Self::ask_install_systemd()?;

        println!();
        if all_passed {
            println!("✅ Setup complete! All checks passed.");
            println!();
            println!("You can now start the server with:");
            println!("  remote-desktop-server start");
        } else {
            println!("⚠️  Setup complete with warnings.");
            println!("   Some features may not work until the issues above are resolved.");
        }
        println!();

        Ok(())
    }

    /// Clear the portal restore token
    fn clear_portal_token() -> Result<()> {
        let token_path = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("remote-desktop")
            .join("restore_token.txt");

        if token_path.exists() {
            fs::remove_file(&token_path)?;
            println!("   ✅ Cleared portal authorization token");
        }
        Ok(())
    }

    /// Check uinput permissions for input injection
    fn check_uinput() -> Result<bool> {
        println!("📋 Checking uinput permissions...");

        let uinput_path = Path::new("/dev/uinput");

        if !uinput_path.exists() {
            println!("   ❌ /dev/uinput does not exist");
            println!("   💡 Load the uinput module: sudo modprobe uinput");
            return Ok(false);
        }

        // Check if we can read/write
        let metadata = fs::metadata(uinput_path)?;
        let mode = metadata.permissions().mode();

        // Check if current user has read/write access
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        let file_uid = metadata.uid();
        let file_gid = metadata.gid();

        let has_access = (file_uid == uid && (mode & 0o600) == 0o600) ||  // Owner
                        (file_gid == gid && (mode & 0o060) == 0o060) ||  // Group
                        (mode & 0o006) == 0o006;  // Others

        if has_access {
            println!("   ✅ /dev/uinput is accessible");
            return Ok(true);
        }

        println!("   ⚠️  /dev/uinput is not accessible by current user");
        println!("   💡 Add yourself to the 'input' group:");
        println!("      sudo usermod -aG input $USER");
        println!("   💡 Then log out and log back in for changes to take effect");
        println!();

        Ok(false)
    }

    /// Check Tailscale installation and status
    fn check_tailscale() -> Result<bool> {
        println!("📋 Checking Tailscale...");

        // Check if tailscale command exists
        let tailscale_exists = Command::new("which")
            .arg("tailscale")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !tailscale_exists {
            println!("   ⚠️  Tailscale is not installed");
            println!("   💡 Install from: https://tailscale.com/download");
            return Ok(false);
        }

        // Check if tailscaled is running
        let status = Command::new("tailscale")
            .arg("status")
            .output();

        match status {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);

                    // Check for 100.x.y.z IP (Tailscale CGNAT range)
                    if stdout.contains("100.") {
                        println!("   ✅ Tailscale is connected");

                        // Extract and show the Tailscale IP
                        for line in stdout.lines() {
                            if line.contains("100.") {
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if !parts.is_empty() {
                                    println!("   📍 IP: {}", parts[0]);
                                }
                                break;
                            }
                        }
                        return Ok(true);
                    } else {
                        println!("   ⚠️  Tailscale is installed but not connected");
                        println!("   💡 Run: tailscale up");
                        return Ok(false);
                    }
                } else {
                    println!("   ⚠️  Tailscale daemon is not running");
                    println!("   💡 Start with: sudo systemctl start tailscaled");
                    return Ok(false);
                }
            }
            Err(_) => {
                println!("   ⚠️  Could not check Tailscale status");
                return Ok(false);
            }
        }
    }

    /// Check xdg-desktop-portal for screen capture
    fn check_portal() -> Result<bool> {
        println!("📋 Checking xdg-desktop-portal...");

        // Check for portal backends
        let portals = ["xdg-desktop-portal-kde", "xdg-desktop-portal-gnome", "xdg-desktop-portal-wlr"];

        let mut found_portal = false;
        for portal in &portals {
            let output = Command::new("pgrep")
                .arg("-f")
                .arg(portal)
                .output();

            if let Ok(output) = output {
                if output.status.success() && !output.stdout.is_empty() {
                    println!("   ✅ {} is running", portal);
                    found_portal = true;
                    break;
                }
            }
        }

        if !found_portal {
            // Check if any xdg-desktop-portal is running
            let output = Command::new("pgrep")
                .arg("-f")
                .arg("xdg-desktop-portal")
                .output();

            if let Ok(output) = output {
                if output.status.success() && !output.stdout.is_empty() {
                    println!("   ✅ xdg-desktop-portal is running");
                    found_portal = true;
                }
            }
        }

        if !found_portal {
            println!("   ⚠️  No xdg-desktop-portal backend detected");
            println!("   💡 For KDE Plasma: Install xdg-desktop-portal-kde");
            println!("   💡 For GNOME: Install xdg-desktop-portal-gnome");
            println!("   💡 For wlroots: Install xdg-desktop-portal-wlr");
            return Ok(false);
        }

        Ok(true)
    }

    /// Check wl-clipboard for clipboard synchronization
    fn check_wl_clipboard() -> Result<bool> {
        println!("📋 Checking wl-clipboard...");

        let wl_paste = Command::new("which")
            .arg("wl-paste")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let wl_copy = Command::new("which")
            .arg("wl-copy")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if wl_paste && wl_copy {
            println!("   ✅ wl-clipboard is installed");
            return Ok(true);
        }

        println!("   ⚠️  wl-clipboard is not installed (clipboard sync will not work)");
        println!("   💡 Install with: sudo pacman -S wl-clipboard");
        println!("      or: sudo apt install wl-clipboard");

        Ok(false)
    }

    /// Check x264 encoder for H.264 encoding
    fn check_x264() -> Result<bool> {
        println!("📋 Checking x264 encoder...");

        let x264 = Command::new("which")
            .arg("x264")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if x264 {
            // Get version
            let version = Command::new("x264")
                .arg("--version")
                .output();

            if let Ok(output) = version {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = stdout.lines().next() {
                    println!("   ✅ {}",
                        first_line.trim().trim_start_matches("x264 "));
                    return Ok(true);
                }
            }

            println!("   ✅ x264 is installed");
            return Ok(true);
        }

        println!("   ❌ x264 is not installed (required for H.264 encoding)");
        println!("   💡 Install with: sudo pacman -S x264");
        println!("      or: sudo apt install x264");

        Ok(false)
    }

    /// Pre-authorize screen capture through the portal
    async fn authorize_screen_capture() -> Result<bool> {
        use std::path::PathBuf;
        println!("📋 Pre-authorizing screen capture...");
        println!();
        println!("   A portal dialog will appear - please select your screen and approve.");
        println!("   This will save the authorization for future connections.");
        println!();

        // Create the session manager and request a session
        let manager = remote_desktop_portal::SessionManager::new().await
            .map_err(|e| anyhow!("Failed to create session manager: {}", e))?;

        match manager.create_persistent_session().await {
            Ok(session) => {
                println!();
                println!("   ✅ Screen capture authorized!");
                println!("   📍 PipeWire node ID: {}", session.pipewire_node_id());
                println!();
                println!("   Future connections will not require approval.");
                Ok(true)
            }
            Err(e) => {
                println!();
                println!("   ⚠️  Failed to authorize screen capture: {}", e);
                println!("   💡 You can still use the server, but each connection will prompt for approval.");
                Ok(false)
            }
        }
    }

    /// Ask to install systemd user service
    fn ask_install_systemd() -> Result<()> {
        println!();
        println!("📋 Systemd Service (Optional)");
        println!("   Would you like to install a systemd user service?");
        println!("   This allows the server to start automatically on login.");
        println!();

        print!("   Install systemd service? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes" {
            Self::install_systemd_service()?;
        } else {
            println!("   Skipping systemd service installation");
        }

        Ok(())
    }

    /// Install systemd user service
    fn install_systemd_service() -> Result<()> {
        println!("   Installing systemd service...");

        let service_dir = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not find config directory"))?
            .join("systemd")
            .join("user");

        fs::create_dir_all(&service_dir)?;

        let service_path = service_dir.join("remote-desktop-server.service");

        let service_content = r#"[Unit]
Description=Remote Desktop Server for Wayland
After=graphical-session.target
Wants=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/remote-desktop-server start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#;

        fs::write(&service_path, service_content)?;

        println!("   ✅ Service file written to {:?}", service_path);

        // Run systemctl daemon-reload
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        println!();
        println!("   To enable the service:");
        println!("      systemctl --user enable remote-desktop-server.service");
        println!();
        println!("   To start now:");
        println!("      systemctl --user start remote-desktop-server.service");
        println!();
        println!("   To view logs:");
        println!("      journalctl --user -u remote-desktop-server -f");

        Ok(())
    }
}

// Need PathBuf for clear_portal_token
use std::path::PathBuf;
