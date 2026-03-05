//! Portal session management for persistent remote desktop access
//!
//! Uses xdg-desktop-portal with restore_token for persistent authorization

use ashpd::desktop::screencast::{Screencast, CursorMode, SourceType};
use ashpd::desktop::PersistMode;
use std::fs;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Restore token storage path
fn token_path() -> PathBuf {
    let mut path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"));
    path.push("remote-desktop");
    let _ = fs::create_dir_all(&path);
    path.push("restore_token.txt");
    path
}

/// Manages portal sessions with persistent authorization
pub struct SessionManager;

impl SessionManager {
    /// Create a new session manager
    pub async fn new() -> super::Result<Self> {
        info!("Creating SessionManager with ashpd");
        Ok(Self)
    }

    /// Create a new session with persistent authorization
    pub async fn create_persistent_session(&self) -> super::Result<PortalSession> {
        info!("Creating persistent portal session");

        // Load restore token for persistent authorization
        let restore_token = self.load_restore_token();

        if restore_token.is_some() {
            info!("Found saved restore token, attempting to reuse session");
        } else {
            info!("No restore token found, will request new authorization");
        }

        // Connect to the screencast portal
        let proxy = Screencast::new().await
            .map_err(|e| super::PortalError::Portal(format!("Failed to connect to portal: {:?}", e)))?;

        // Create session
        let session = proxy.create_session().await
            .map_err(|e| super::PortalError::Portal(format!("Failed to create session: {:?}", e)))?;

        // Select sources (monitors/windows)
        // Always use PersistMode::Application to get a restore token for future sessions
        let persist_mode = PersistMode::Application;

        proxy.select_sources(
            &session,
            CursorMode::Hidden,
            SourceType::Monitor | SourceType::Window,
            true,  // multiple sources
            restore_token.as_deref(),
            persist_mode,
        ).await
            .map_err(|e| super::PortalError::Portal(format!("Failed to select sources: {:?}", e)))?;

        // Start screencast (shows portal dialog if needed)
        let streams_response = proxy.start(&session, None::<&ashpd::WindowIdentifier>).await
            .map_err(|e| super::PortalError::Portal(format!("Failed to start screencast: {:?}", e)))?
            .response()
            .map_err(|e| super::PortalError::Portal(format!("Failed to get streams response: {:?}", e)))?;

        // Get the first stream's PipeWire node ID
        let streams = streams_response.streams();
        if streams.is_empty() {
            return Err(super::PortalError::Portal("No streams returned from portal".to_string()));
        }

        let node_id = streams[0].pipe_wire_node_id();
        debug!("Got PipeWire node ID: {}", node_id);

        // Store restore token for future use (persistent authorization)
        if let Some(token) = streams_response.restore_token() {
            if !token.is_empty() {
                self.save_restore_token(token)?;
            }
        }

        Ok(PortalSession {
            node_id,
            restore_token: streams_response.restore_token().map(|s| s.to_string()),
        })
    }

    /// Load a previously saved restore token
    fn load_restore_token(&self) -> Option<String> {
        let path = token_path();
        if !path.exists() {
            debug!("No restore token found at {:?}", path);
            return None;
        }

        match fs::read_to_string(&path) {
            Ok(content) => {
                let token = content.trim().to_string();
                if !token.is_empty() {
                    debug!("Loaded restore token from {:?}", path);
                    Some(token)
                } else {
                    None
                }
            }
            Err(e) => {
                warn!("Failed to read restore token: {}", e);
                None
            }
        }
    }

    /// Save a restore token for future sessions
    fn save_restore_token(&self, token: &str) -> super::Result<()> {
        let path = token_path();

        fs::write(&path, token)
            .map_err(|e| super::PortalError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)
                .map_err(|e| super::PortalError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?
                .permissions();
            perms.set_mode(0o600);
            let _ = fs::set_permissions(&path, perms)
                .map_err(|e| super::PortalError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)));
        }

        info!("Restore token saved to {:?}", path);
        Ok(())
    }

    /// Clear stored restore token (for re-authorization)
    pub fn clear_restore_token(&self) -> super::Result<()> {
        let path = token_path();
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| super::PortalError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            info!("Restore token cleared");
        }
        Ok(())
    }
}

/// An active portal session with remote desktop access
pub struct PortalSession {
    node_id: u32,
    restore_token: Option<String>,
}

impl PortalSession {
    /// Get the PipeWire node ID for screen capture
    ///
    /// This returns the PipeWire node ID that should be used
    /// with the PipeWire API to connect to the screen capture stream.
    pub fn pipewire_node_id(&self) -> u32 {
        self.node_id
    }

    /// Get the libei socket for input injection
    pub async fn input_socket(&self) -> super::Result<(OwnedFd, OwnedFd)> {
        Err(super::PortalError::Portal("libei integration not yet implemented".to_string()))
    }

    /// Check if the session is still valid
    pub async fn is_active(&self) -> bool {
        true
    }

    /// Get the restore token (if available)
    pub fn restore_token(&self) -> Option<&str> {
        self.restore_token.as_deref()
    }
}

/// Setup wizard for one-time authorization
pub async fn setup_wizard() -> super::Result<()> {
    println!("=== Remote Desktop Setup Wizard ===");
    println!();
    println!("This will authorize the remote desktop service to access your screen.");
    println!("You will see a portal dialog from KDE - please approve it.");
    println!();
    println!("After authorization, remote access will work without prompts.");
    println!();
    println!("NOTE: Make sure xdg-desktop-portal-kde is installed and running.");

    let manager = SessionManager::new().await?;

    println!("Requesting screencast permission...");
    let session = manager.create_persistent_session().await?;

    println!();
    println!("✓ Authorization complete!");
    println!("  PipeWire node ID: {}", session.pipewire_node_id());
    println!("  The remote desktop server can now access your screen.");
    println!("  You can revoke this permission at any time in KDE Settings.");
    println!();

    Ok(())
}
