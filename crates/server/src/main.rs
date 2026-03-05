//! Remote Desktop Server
//!
//! Main server binary for the Wayland remote desktop solution

mod connection;
mod setup;
mod status;
mod virtual_display;

use clap::{Parser, Subcommand};
use tracing::{info, error, warn};
use tracing_subscriber::EnvFilter;
use connection::ServerState;
use setup::SetupWizard;
use status::display_status;
use virtual_display::VirtualDisplay;
use std::env;

#[derive(Parser)]
#[command(name = "remote-desktop-server")]
#[command(about = "Remote desktop server for Wayland", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server
    Start {
        /// Port to listen on (default: 3389)
        #[arg(short, long)]
        port: Option<u16>,

        /// Use virtual display (headless mode, no portal approval needed)
        #[arg(name = "virtual", long)]
        virtual_display: bool,

        /// Virtual display width (default: 1920)
        #[arg(long, default_value = "1920")]
        width: u32,

        /// Virtual display height (default: 1080)
        #[arg(long, default_value = "1080")]
        height: u32,

        /// Virtual display refresh rate (default: 60)
        #[arg(long, default_value = "60")]
        refresh: u32,
    },

    /// Run the setup wizard
    Setup {
        /// Reset existing authorization
        #[arg(long)]
        reset: bool,

        /// Pre-authorize screen capture (avoids prompts on connect)
        #[arg(long)]
        authorize: bool,
    },

    /// List available monitors
    ListMonitors,

    /// Show status
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("remote_desktop=info".parse().unwrap())
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start { port, virtual_display, width, height, refresh } => {
            let port = port.unwrap_or(3389);
            let is_virtual = virtual_display;
            info!("Starting remote desktop server on port {} (virtual={})", port, is_virtual);

            // Start virtual display if requested
            let mut virtual_display_guard = if is_virtual {
                info!("Starting virtual display mode...");

                // Try to start a headless compositor (optional - will use test patterns if unavailable)
                match virtual_display::check_compositor_available() {
                    Ok(compositor) => {
                        info!("Found {} for virtual display", compositor);

                        let mut vd = VirtualDisplay::new(width, height, refresh);
                        match vd.start() {
                            Ok(wayland_socket) => {
                                info!("Virtual display started on {}", wayland_socket);
                                // Set WAYLAND_DISPLAY for capture
                                unsafe { env::set_var("WAYLAND_DISPLAY", &wayland_socket); }
                            }
                            Err(e) => {
                                warn!("Failed to start virtual display compositor: {}", e);
                                warn!("Using test pattern generator instead");
                            }
                        }
                        Some(vd)
                    }
                    Err(e) => {
                        warn!("No headless compositor found: {}", e);
                        warn!("Using test pattern generator for virtual display");
                        None
                    }
                }
            } else {
                None
            };

            // Write PID file
            if let Err(e) = status::write_pid_file() {
                error!("Failed to write PID file: {}", e);
            }

            let state = ServerState::new();

            // Set virtual display mode on state
            if is_virtual {
                state.set_virtual_mode(true);
            }

            // Initialize input handler asynchronously
            let state_clone = state.clone();
            tokio::spawn(async move {
                state_clone.init_input_handler(remote_desktop_portal::InputBackend::Uinput).await;
            });

            let result = connection::start_server(&format!("0.0.0.0:{}", port), state).await;

            // Clean up
            let _ = status::remove_pid_file();
            drop(virtual_display_guard);

            result
        }

        Commands::Setup { reset, authorize } => {
            info!("Running setup wizard (reset={}, authorize={})", reset, authorize);
            SetupWizard::run(reset, authorize).await
        }

        Commands::ListMonitors => {
            info!("Listing available monitors");
            match remote_desktop_core::enumerate_monitors() {
                Ok(monitors) => {
                    println!("Available monitors:");
                    for monitor in &monitors {
                        println!("  - {} ({}x{}@{}Hz){}",
                            monitor.name,
                            monitor.resolution.0,
                            monitor.resolution.1,
                            monitor.refresh_rate,
                            if monitor.is_primary { " [PRIMARY]" } else { "" }
                        );
                    }
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to list monitors: {}", e);
                    Err(e.into())
                }
            }
        }

        Commands::Status => {
            info!("Checking status");
            display_status()
        }
    }
}
