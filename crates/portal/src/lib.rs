//! Remote Desktop Portal Integration
//!
//! Handles xdg-desktop-portal session management and libei input injection.

mod session;
mod input;
pub mod clipboard;

pub use session::{SessionManager, PortalSession};
pub use input::{InputHandler, KeyEvent, PointerEvent, ButtonState, KeyState, InputBackend};
pub use clipboard::ClipboardHandler;

use thiserror::Error;

/// Portal error types
#[derive(Error, Debug)]
pub enum PortalError {
    #[error("Portal error: {0}")]
    Portal(String),

    #[error("Portal request denied")]
    Denied,

    #[error("Session not authorized")]
    NotAuthorized,

    #[error("Restore token not found")]
    RestoreTokenNotFound,

    #[error("libei error: {0}")]
    Libei(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, PortalError>;
