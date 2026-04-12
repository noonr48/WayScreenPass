//! Remote Desktop Core
//!
//! Core functionality for monitor detection and PipeWire integration.

mod monitor;
mod encoder;
pub mod protocol;

#[cfg(feature = "portal-capture")]
mod pipewire;

pub use monitor::{MonitorInfo, enumerate_monitors, find_monitor, get_primary_monitor};
#[cfg(feature = "portal-capture")]
pub use pipewire::{VideoStream, VideoFrame};
pub use encoder::{H264Encoder, EncodedFrame};
pub use protocol::*;

use thiserror::Error;

/// Core error types
#[derive(Error, Debug)]
pub enum CoreError {
    #[error("PipeWire error: {0}")]
    PipeWire(String),

    #[error("No monitors found")]
    NoMonitors,

    #[error("Monitor not found: {0}")]
    MonitorNotFound(String),

    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
