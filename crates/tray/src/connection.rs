//! Connection management

use anyhow::Result;
use std::process::Command;
use tracing::{error, info, warn};

use crate::config::HostConfig;

/// Spawns a new connection to a remote host
pub fn connect_to_host(host: &HostConfig, client_binary: &str) -> Result<()> {
    info!("Connecting to {} ({})", host.name, host.address());

    let addr = host.address();

    let result = Command::new(client_binary)
        .arg("connect")
        .arg(&addr)
        .spawn();

    match result {
        Ok(child) => {
            info!("Spawned connection process (PID: {:?})", child.id());
            Ok(())
        }
        Err(e) => {
            error!("Failed to spawn connection: {}", e);
            warn!("Make sure '{}' is in PATH or specify full path", client_binary);
            Err(e.into())
        }
    }
}

/// Show a notification using notify-send
pub fn show_notification(title: &str, message: &str) -> Result<()> {
    Command::new("notify-send")
        .arg("-i")
        .arg("computer")
        .arg(title)
        .arg(message)
        .spawn()?;
    Ok(())
}
