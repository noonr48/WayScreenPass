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
                // Resolve the monitor name once using the initial connection
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

                // Drop the initial connection; the reconnect loop will establish its own
                drop(conn);

                println!("Connecting to monitor: {}", monitor_name);

                let mut reconnect_delay = std::time::Duration::from_secs(1);
                let max_delay = std::time::Duration::from_secs(30);
                let mut player: Option<connection::VideoPlayer> = None;

                loop {
                    // Connect (or reconnect)
                    let mut conn = match connection::ClientConnection::connect(&addr).await {
                        Ok(c) => {
                            reconnect_delay = std::time::Duration::from_secs(1); // reset backoff
                            c
                        }
                        Err(e) => {
                            error!("Connection failed: {}. Retrying in {:?}...", e, reconnect_delay);
                            tokio::time::sleep(reconnect_delay).await;
                            reconnect_delay = (reconnect_delay * 2).min(max_delay);
                            continue;
                        }
                    };

                    // Select monitor (re-do handshake on each reconnection)
                    let stream_info = match conn.select_monitor(&monitor_name).await {
                        Ok(info) => info,
                        Err(e) => {
                            error!("Failed to select monitor: {}. Reconnecting...", e);
                            tokio::time::sleep(reconnect_delay).await;
                            reconnect_delay = (reconnect_delay * 2).min(max_delay);
                            continue;
                        }
                    };

                    info!("Stream started: {}x{}@{}fps", stream_info.width, stream_info.height, stream_info.fps);

                    // Create video player once; reuse across reconnections
                    if player.is_none() {
                        match connection::VideoPlayer::new(
                            stream_info.width,
                            stream_info.height,
                            stream_info.fps,
                        ) {
                            Ok(p) => player = Some(p),
                            Err(e) => {
                                error!("Failed to create video player: {}", e);
                                return Err(e);
                            }
                        }
                    }

                    let p = player.as_mut().unwrap();

                    // Streaming loop
                    let mut disconnected = false;
                    let start = std::time::Instant::now();
                    let mut last_stats = start;

                    loop {
                        match conn.receive_message().await {
                            Ok(Some(connection::ReceivedMessage::VideoFrame(frame))) => {
                                // Process frame: decode, display, and get input events
                                let input_messages = p.process_frame(&frame)?;

                                // Send input events back to server
                                for msg in input_messages {
                                    if let Err(e) = conn.send_message(msg).await {
                                        warn!("Failed to send input: {}", e);
                                        disconnected = true;
                                        break;
                                    }
                                }

                                if disconnected {
                                    break;
                                }

                                // Check if window was closed
                                if !p.is_running() {
                                    info!("Window closed, exiting");
                                    return Ok(());
                                }

                                // Check if we should exit (test mode)
                                if test_duration > 0 {
                                    let elapsed = start.elapsed().as_secs();
                                    if elapsed >= test_duration {
                                        info!("Test duration reached, exiting");
                                        return Ok(());
                                    }
                                }
                            }
                            Ok(Some(connection::ReceivedMessage::ClipboardEvent(event))) => {
                                // Handle clipboard event from server
                                if let Err(e) = p.handle_clipboard(&event) {
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
                                warn!("Connection error: {}. Will reconnect...", e);
                                break;
                            }
                        }

                        // Print stats every 5 seconds
                        if last_stats.elapsed() >= std::time::Duration::from_secs(5) {
                            info!("Streaming stats: {:.1} fps, {} frames received",
                                p.fps(), p.frame_count());
                            last_stats = std::time::Instant::now();
                        }
                    }

                    // If we get here, we disconnected — loop back and reconnect
                    warn!("Disconnected. Reconnecting in {:?}...", reconnect_delay);
                    tokio::time::sleep(reconnect_delay).await;
                    reconnect_delay = (reconnect_delay * 2).min(max_delay);
                }
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
