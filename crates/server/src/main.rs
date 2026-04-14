//! Remote Desktop Server
//!
//! Main server binary for the Wayland remote desktop solution

mod connection;
mod setup;
mod status;
mod virtual_display;

use clap::{Parser, Subcommand};
use tracing::{info, error};
use tracing_subscriber::EnvFilter;
use connection::ServerState;
use setup::SetupWizard;
use status::display_status;
use virtual_display::{VirtualDisplay, launch_in_headless_session, read_session_metadata};

#[derive(Parser)]
#[command(name = "remote-desktop-server")]
#[command(about = "Headless Wayland remote desktop server", long_about = None)]
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
    Setup,

    /// List available monitors
    ListMonitors,

    /// Show status
    Status,

    /// Launch an application inside the active headless session
    Launch {
        /// Command to run inside the headless session
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        command: Vec<String>,
    },
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
        Commands::Start { port, width, height, refresh } => {
            let port = port.unwrap_or(3389);
            info!("Starting headless remote desktop server on port {}", port);

            let state = ServerState::new();

            let compositor = virtual_display::check_compositor_available()?;
            info!("Starting dedicated headless session with {}", compositor);

            let mut vd = VirtualDisplay::new(width, height, refresh);
            let session = vd.start()?;
            state.set_headless_session(Some(session.clone())).await;
            let virtual_display_guard = Some(vd);

            let input_backend = remote_desktop_portal::InputBackend::HeadlessWayland {
                runtime_dir: session.runtime_dir.clone(),
                wayland_display: session.wayland_display.clone(),
            };

            // Write PID file
            if let Err(e) = status::write_pid_file() {
                error!("Failed to write PID file: {}", e);
            }

            // Initialize input handler asynchronously
            let state_clone = state.clone();
            tokio::spawn(async move {
                state_clone.init_input_handler(input_backend).await;
            });

            // Set up signal handling for clean shutdown
            let shutdown_signal = async {
                let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("Failed to set up SIGTERM handler");
                let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                    .expect("Failed to set up SIGINT handler");

                tokio::select! {
                    _ = sigterm.recv() => info!("Received SIGTERM"),
                    _ = sigint.recv() => info!("Received SIGINT"),
                }
            };

            let bind_addr = format!("0.0.0.0:{}", port);
            let result = tokio::select! {
                result = connection::start_server(&bind_addr, state) => {
                    result
                }
                _ = shutdown_signal => {
                    info!("Shutting down gracefully...");
                    Ok(())
                }
            };

            // Clean up
            let _ = status::remove_pid_file();
            drop(virtual_display_guard);

            result
        }

        Commands::Setup => {
            info!("Running setup wizard");
            SetupWizard::run().await
        }

        Commands::ListMonitors => {
            info!("Listing available monitors");
            let session = read_session_metadata()?
                .ok_or_else(|| anyhow::anyhow!("No active headless session found. Start the server first."))?;
            println!("Available monitors:");
            println!(
                "  - {} ({}x{}@{}Hz) [HEADLESS] [PRIMARY]",
                session.output_name, session.width, session.height, session.refresh_rate
            );
            Ok(())
        }

        Commands::Status => {
            info!("Checking status");
            display_status()
        }

        Commands::Launch { command } => {
            info!("Launching {:?} in headless session", command);
            launch_in_headless_session(&command)
        }
    }
}
