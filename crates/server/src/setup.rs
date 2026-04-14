//! Setup Wizard
//!
//! Interactive first-time setup for the headless remote desktop server.

use anyhow::{Result, anyhow};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::Command;

/// Setup wizard for first-time configuration
pub struct SetupWizard;

impl SetupWizard {
    /// Run the setup wizard
    pub async fn run() -> Result<()> {
        println!();
        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║      Headless Wayland Remote Desktop - Setup Wizard        ║");
        println!("╚════════════════════════════════════════════════════════════╝");
        println!();

        let mut all_passed = true;

        // 1. Check Tailscale
        if !Self::check_tailscale()? {
            all_passed = false;
        }

        // 2. Check dedicated headless-session runtime
        if !Self::check_headless_runtime()? {
            all_passed = false;
        }

        // 3. Check wl-clipboard (for clipboard sync)
        if !Self::check_wl_clipboard()? {
            all_passed = false;
        }

        // 4. Check x264 encoder
        if !Self::check_x264()? {
            all_passed = false;
        }

        // 5. Check linger for unattended boot/start
        if !Self::check_user_linger()? {
            all_passed = false;
        }

        // 6. Ask about systemd service
        Self::ask_install_systemd()?;

        println!();
        if all_passed {
            println!("✅ Setup complete! All checks passed.");
            println!();
            println!("You can now start the headless session server with:");
            println!("  remote-desktop-server start");
        } else {
            println!("⚠️  Setup complete with warnings.");
            println!("   Some features may not work until the issues above are resolved.");
        }
        println!();

        Ok(())
    }

    /// Check runtime dependencies for the dedicated headless session path
    fn check_headless_runtime() -> Result<bool> {
        println!("📋 Checking headless session runtime...");

        let checks = [
            ("sway", "sway"),
            ("swaymsg", "swaymsg"),
            ("grim", "grim"),
        ];

        let mut missing = Vec::new();
        for (binary, package_hint) in checks {
            let exists = Command::new("which")
                .arg(binary)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if exists {
                println!("   ✅ {} is installed", binary);
            } else {
                println!("   ❌ {} is missing", binary);
                missing.push(package_hint);
            }
        }

        if missing.is_empty() {
            println!("   ✅ Headless sway/grim runtime is ready");
            return Ok(true);
        }

        println!("   💡 Install the missing tools to use 'remote-desktop-server start'");
        println!("      Missing: {}", missing.join(", "));
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

    /// Ask to install systemd user service
    fn ask_install_systemd() -> Result<()> {
        println!();
        println!("📋 Systemd Service (Optional)");
        println!("   Would you like to install a systemd user service?");
        println!("   This allows the headless server to start automatically.");
        println!("   For cold boot / remote-only use, also enable linger for your user.");
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

        let service_path = service_dir.join("remote-desktop.service");
        let exec_path = env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "%h/.cargo/bin/remote-desktop-server".to_string());

        let service_content = format!(r#"[Unit]
Description=WayScreenPass Headless Remote Desktop Server
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
ExecStart={} start
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#, exec_path);

        fs::write(&service_path, service_content)?;

        println!("   ✅ Service file written to {:?}", service_path);

        // Run systemctl daemon-reload
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        println!();
        println!("   To enable the service:");
        println!("      systemctl --user enable remote-desktop.service");
        println!();
        println!("   To start now:");
        println!("      systemctl --user start remote-desktop.service");
        println!();
        println!("   To view logs:");
        println!("      journalctl --user -u remote-desktop -f");
        println!();
        println!("   For unattended boot without local login:");
        println!(
            "      sudo loginctl enable-linger {}",
            env::var("USER").unwrap_or_else(|_| "$USER".to_string())
        );

        Ok(())
    }

    fn check_user_linger() -> Result<bool> {
        println!("📋 Checking unattended boot support (linger)...");

        let user = env::var("USER").unwrap_or_else(|_| "current user".to_string());
        let output = Command::new("loginctl")
            .args(["show-user", &user, "-p", "Linger"])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("Linger=yes") {
                    println!("   ✅ loginctl linger is enabled for {}", user);
                    Ok(true)
                } else {
                    println!("   ⚠️  linger is disabled for {}", user);
                    println!("   💡 Enable it for unattended boot/loginless startup:");
                    println!("      sudo loginctl enable-linger {}", user);
                    Ok(false)
                }
            }
            Ok(output) => {
                println!("   ⚠️  Could not query linger state: {}", String::from_utf8_lossy(&output.stderr).trim());
                println!("   💡 Verify manually with: loginctl show-user {} -p Linger", user);
                Ok(false)
            }
            Err(e) => {
                println!("   ⚠️  Failed to run loginctl: {}", e);
                println!("   💡 Verify manually with: loginctl show-user {} -p Linger", user);
                Ok(false)
            }
        }
    }
}
