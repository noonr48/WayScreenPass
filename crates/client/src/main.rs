//! Remote Desktop Client
//!
//! CLI client for connecting to remote desktop servers

mod connection;
mod decoder;
mod display;
mod input;
mod clipboard;

use clap::{Parser, Subcommand};
use tracing::{info, error, warn};
use connection::{ClientConnection, VideoPlayer};

#[derive(Parser)]
#[command(name = "remote-desktop")]
#[command(about = "Remote desktop client for Wayland", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote desktop
    Connect {
        /// Hostname or IP address (via Tailscale)
        hostname: String,

        /// Port number (default: 3389)
        #[arg(short, long, default_value = "3389")]
        port: u16,

        /// List available monitors instead of connecting
        #[arg(long)]
        list: bool,

        /// Specific monitor to select
        #[arg(short, long)]
        monitor: Option<String>,

        /// Run in test mode (exit after streaming for N seconds)
        #[arg(long, default_value = "0")]
        test_duration: u64,
    },

    /// List monitors on remote host
    ListMonitors {
        /// Hostname or IP address
        hostname: String,

        /// Port number (default: 3389)
        #[arg(short, long, default_value = "3389")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("remote_desktop=info")
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Connect { hostname, port, list, monitor, test_duration } => {
            // Handle hostname that may already contain a port
            let addr = if hostname.contains(':') {
                hostname.clone()
            } else {
                format!("{}:{}", hostname, port)
            };
            info!("Connecting to {}", addr);

            // Connect to server
            let mut conn: connection::ClientConnection = connection::ClientConnection::connect(&addr).await?;

            if list {
                // List monitors
                let monitors: Vec<remote_desktop_core::protocol::MonitorInfo> = conn.get_monitors().await?;
                println!("Available monitors on {}:", hostname);
                for mon in &monitors {
                    println!("  - {} ({}x{}@{}Hz){}",
                        mon.name,
                        mon.width,
                        mon.height,
                        mon.refresh_rate,
                        if mon.is_primary { " [PRIMARY]" } else { "" }
                    );
                }
                Ok(())
            } else {
                // Select monitor
                let monitor_name = match monitor {
                    Some(ref m) => m.clone(),
                    None => {
                        // Auto-select primary monitor
                        let monitors = conn.get_monitors().await?;
                        let primary = monitors.iter()
                            .find(|m| m.is_primary)
                            .or_else(|| monitors.first())
                            .ok_or_else(|| anyhow::anyhow!("No monitors available"))?;
                        primary.name.clone()
                    }
                };

                println!("Connecting to monitor: {}", monitor_name);

                // Start streaming
                let stream_info = conn.select_monitor(&monitor_name).await?;
                println!("Stream started: {}x{}@{}fps",
                    stream_info.width, stream_info.height, stream_info.fps);

                // Create video player
                let player_result = connection::VideoPlayer::new(
                    stream_info.width,
                    stream_info.height,
                    stream_info.fps,
                );

                let mut player = match player_result {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to create video player: {}", e);
                        return Err(e);
                    }
                };

                // Initialize clipboard (requires SDL context which is created in display)
                // For now, we skip clipboard init since SDL context is owned by display
                // The clipboard will be initialized when needed

                // Receive and process frames
                let start = std::time::Instant::now();
                let mut last_stats = start;

                loop {
                    match conn.receive_message().await {
                        Ok(Some(connection::ReceivedMessage::VideoFrame(frame))) => {
                            // Process frame: decode, display, and get input events
                            let input_messages = player.process_frame(&frame)?;

                            // Send input events back to server
                            for msg in input_messages {
                                if let Err(e) = conn.send_message(msg).await {
                                    error!("Failed to send input message: {}", e);
                                }
                            }

                            // Check if window was closed
                            if !player.is_running() {
                                info!("Window closed, exiting");
                                break;
                            }

                            // Check if we should exit (test mode)
                            if test_duration > 0 {
                                let elapsed = start.elapsed().as_secs();
                                if elapsed >= test_duration {
                                    info!("Test duration reached, exiting");
                                    break;
                                }
                            }
                        }
                        Ok(Some(connection::ReceivedMessage::ClipboardEvent(event))) => {
                            // Handle clipboard event from server
                            if let Err(e) = player.handle_clipboard(&event) {
                                warn!("Failed to handle clipboard event: {}", e);
                            }
                        }
                        Ok(Some(connection::ReceivedMessage::StreamStats(_))) |
                        Ok(Some(connection::ReceivedMessage::Pong(_))) => {
                            // Stats and pongs are just logged, no action needed
                        }
                        Ok(None) => {
                            // No message available, continue
                        }
                        Err(e) => {
                            error!("Error receiving message: {}", e);
                            break;
                        }
                    }

                    // Print stats every 5 seconds
                    if last_stats.elapsed() >= std::time::Duration::from_secs(5) {
                        info!("Streaming stats: {:.1} fps, {} frames received",
                            player.fps(), player.frame_count());
                        last_stats = std::time::Instant::now();
                    }
                }

                info!("Connection closed");
                Ok(())
            }
        }

        Commands::ListMonitors { hostname, port } => {
            // Handle hostname that may already contain a port
            let addr = if hostname.contains(':') {
                hostname.clone()
            } else {
                format!("{}:{}", hostname, port)
            };
            info!("Listing monitors on {}", addr);

            let mut conn: connection::ClientConnection = connection::ClientConnection::connect(&addr).await?;
            let monitors: Vec<remote_desktop_core::protocol::MonitorInfo> = conn.get_monitors().await?;

            println!("Available monitors on {}:", hostname);
            for mon in &monitors {
                println!("  - {} ({}x{}@{}Hz){}",
                    mon.name,
                    mon.width,
                    mon.height,
                    mon.refresh_rate,
                    if mon.is_primary { " [PRIMARY]" } else { "" }
                );
            }

            Ok(())
        }
    }
}
