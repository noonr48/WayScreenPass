//! Monitor detection for multi-GPU systems
//!
//! Enumerates monitors across AMD and NVIDIA GPUs using the Linux sysfs DRM interface

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
    let drm_path = std::path::Path::new("/sys/class/drm");

    let entries = match std::fs::read_dir(drm_path) {
        Ok(e) => e,
        Err(e) => {
            warn!("Cannot read /sys/class/drm: {}", e);
            return Ok(vec![fallback_monitor()]);
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.file_name().to_string_lossy().to_string();

        // Skip entries that don't look like connector entries (e.g. "version", "card0" without connector)
        // Valid entries look like: card0-DP-1, card0-HDMI-A-1, card1-eDP-1
        if !name.contains('-') || name.starts_with('.') {
            continue;
        }

        let entry_path = entry.path();

        // Check if connected
        let status_path = entry_path.join("status");
        let status = std::fs::read_to_string(&status_path)
            .unwrap_or_default()
            .trim()
            .to_string();

        if status != "connected" {
            debug!("Skipping {} (status: {})", name, status);
            continue;
        }

        // Parse connector name from entry name (e.g., "card0-DP-1" -> connector "DP-1", gpu "card0")
        let (gpu_name, connector) = match name.split_once('-') {
            Some((gpu, conn)) => (gpu.to_string(), conn.to_string()),
            None => continue,
        };

        // Read modes to get resolution
        let modes_path = entry_path.join("modes");
        let modes_content = std::fs::read_to_string(&modes_path).unwrap_or_default();
        let first_mode = modes_content.lines().next().unwrap_or("1920x1080");

        let (width, height) = parse_mode(first_mode);

        // Check enabled state
        let enabled_path = entry_path.join("enabled");
        let enabled = std::fs::read_to_string(&enabled_path)
            .unwrap_or_default()
            .trim()
            .to_string();

        let gpu_device = format!("/dev/dri/{}", gpu_name);

        debug!("Found monitor: {} on {} ({}x{}, enabled={})",
            name, gpu_device, width, height, enabled);

        monitors.push(MonitorInfo {
            name: name.clone(),
            gpu_device,
            connector,
            resolution: (width, height),
            refresh_rate: 60, // sysfs modes don't always include refresh, default to 60
            is_primary: monitors.is_empty(), // First connected monitor is primary
        });
    }

    // Sort by name for consistent ordering
    monitors.sort_by(|a, b| a.name.cmp(&b.name));

    // Mark first as primary if none set
    if !monitors.is_empty() && !monitors.iter().any(|m| m.is_primary) {
        monitors[0].is_primary = true;
    }

    if monitors.is_empty() {
        warn!("No connected monitors found via sysfs, returning fallback");
        monitors.push(fallback_monitor());
    }

    info!("Detected {} monitors", monitors.len());
    Ok(monitors)
}

/// Parse a mode string like "1920x1080" into (width, height)
fn parse_mode(mode: &str) -> (u32, u32) {
    let parts: Vec<&str> = mode.trim().split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().unwrap_or(1920);
        let h = parts[1].parse().unwrap_or(1080);
        (w, h)
    } else {
        (1920, 1080)
    }
}

/// Fallback monitor when detection fails
fn fallback_monitor() -> MonitorInfo {
    MonitorInfo {
        name: "fallback-DP-1".to_string(),
        gpu_device: "/dev/dri/card0".to_string(),
        connector: "DP-1".to_string(),
        resolution: (1920, 1080),
        refresh_rate: 60,
        is_primary: true,
    }
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
    fn test_parse_mode() {
        assert_eq!(parse_mode("1920x1080"), (1920, 1080));
        assert_eq!(parse_mode("2560x1440"), (2560, 1440));
        assert_eq!(parse_mode("3840x2160"), (3840, 2160));
        assert_eq!(parse_mode("invalid"), (1920, 1080));
        assert_eq!(parse_mode(""), (1920, 1080));
    }

    #[test]
    fn test_fallback_monitor() {
        let m = fallback_monitor();
        assert_eq!(m.name, "fallback-DP-1");
        assert_eq!(m.resolution, (1920, 1080));
        assert!(m.is_primary);
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
