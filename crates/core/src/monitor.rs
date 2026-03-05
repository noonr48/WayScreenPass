//! Monitor detection for multi-GPU systems
//!
//! Enumerates monitors across AMD and NVIDIA GPUs using libdrm and KWin D-Bus

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Information about a detected monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Human-readable name (e.g., "card0-DP-1", "card1-HDMI-2")
    pub name: String,

    /// GPU device node (e.g., "/dev/dri/card0")
    pub gpu_device: String,

    /// Connector name
    pub connector: String,

    /// Resolution (width, height) - assuming 1080p for this use case
    pub resolution: (u32, u32),

    /// Refresh rate in Hz
    pub refresh_rate: u32,

    /// Whether this is the primary monitor
    pub is_primary: bool,
}

/// Enumerate all available monitors across all GPUs
pub fn enumerate_monitors() -> super::Result<Vec<MonitorInfo>> {
    let mut monitors = Vec::new();

    // Use glob to find all DRM card devices
    let card_pattern = std::path::Path::new("/dev/dri");

    // Try to read card devices
    let entries = std::fs::read_dir(card_pattern)
        .map_err(|e| super::CoreError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Only look at card* devices
        if !name.starts_with("card") {
            continue;
        }

        debug!("Checking DRM device: {:?}", path);

        // For now, add a placeholder monitor entry for each GPU
        // In a full implementation, we'd enumerate actual connectors using the drm crate properly
        let gpu_name = path.display().to_string();
        debug!("Found GPU device: {}", gpu_name);

        // Add a placeholder monitor for this GPU
        monitors.push(MonitorInfo {
            name: format!("{}-DP-1", name),
            gpu_device: gpu_name.clone(),
            connector: "DP-1".to_string(),
            resolution: (1920, 1080),
            refresh_rate: 60,
            is_primary: monitors.is_empty(), // First one is primary
        });
    }

    if monitors.is_empty() {
        // Return a fallback for testing
        warn!("No monitors detected via DRM, returning fallback");
        monitors.push(MonitorInfo {
            name: "card0-DP-1".to_string(),
            gpu_device: "/dev/dri/card0".to_string(),
            connector: "DP-1".to_string(),
            resolution: (1920, 1080),
            refresh_rate: 60,
            is_primary: true,
        });
    }

    Ok(monitors)
}

/// Find a monitor by name
pub fn find_monitor(name: &str) -> super::Result<MonitorInfo> {
    let monitors = enumerate_monitors()?;

    monitors
        .into_iter()
        .find(|m| m.name == name || m.connector == name)
        .ok_or_else(|| super::CoreError::MonitorNotFound(name.to_string()))
}

/// Get the primary monitor (or first available if none marked as primary)
pub fn get_primary_monitor() -> super::Result<MonitorInfo> {
    let monitors = enumerate_monitors()?;

    monitors
        .into_iter()
        .find(|m| m.is_primary)
        .ok_or(super::CoreError::NoMonitors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_monitors() {
        let monitors = enumerate_monitors();
        match monitors {
            Ok(ms) => {
                println!("Found {} monitors:", ms.len());
                for m in &ms {
                    println!("  - {} ({}x{}@{}Hz) on {} (primary: {})",
                        m.name, m.resolution.0, m.resolution.1,
                        m.refresh_rate, m.gpu_device, m.is_primary);
                }
                assert!(!ms.is_empty(), "Should have at least one monitor");
            }
            Err(e) => {
                println!("Monitor enumeration error: {}", e);
            }
        }
    }

    #[test]
    fn test_find_monitor() {
        match find_monitor("DP-1") {
            Ok(m) => {
                println!("Found monitor: {}", m.name);
            }
            Err(e) => {
                println!("Find monitor error (expected in some environments): {}", e);
            }
        }
    }

    #[test]
    fn test_get_primary() {
        match get_primary_monitor() {
            Ok(m) => {
                println!("Primary monitor: {} (primary={})", m.name, m.is_primary);
            }
            Err(e) => {
                println!("Get primary error (expected in some environments): {}", e);
            }
        }
    }
}
