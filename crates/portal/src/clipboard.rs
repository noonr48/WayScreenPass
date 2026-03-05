//! Wayland Clipboard Handler
//!
//! Provides clipboard access via wl-copy/wl-paste commands (wl-clipboard package)

use anyhow::{Result, anyhow};
use std::process::Command;
use tracing::{debug, warn};
use xxhash_rust::xxh3::xxh3_64;

/// Wayland clipboard handler using wl-clipboard tools
pub struct ClipboardHandler {
    last_content_hash: Option<u64>,
    last_mime_type: Option<String>,
}

impl ClipboardHandler {
    /// Create a new clipboard handler
    pub fn new() -> Result<Self> {
        // Check if wl-paste and wl-copy are available
        Self::check_wl_clipboard()?;

        debug!("ClipboardHandler initialized");
        Ok(Self {
            last_content_hash: None,
            last_mime_type: None,
        })
    }

    /// Check if wl-clipboard tools are available
    fn check_wl_clipboard() -> Result<()> {
        let wl_paste = Command::new("which")
            .arg("wl-paste")
            .output()
            .map(|o| o.status.success());

        let wl_copy = Command::new("which")
            .arg("wl-copy")
            .output()
            .map(|o| o.status.success());

        if wl_paste.unwrap_or(false) && wl_copy.unwrap_or(false) {
            Ok(())
        } else {
            Err(anyhow!("wl-clipboard (wl-paste/wl-copy) not found. Please install wl-clipboard package."))
        }
    }

    /// Get current clipboard content for a specific MIME type
    pub fn get_content(&self, mime_type: &str) -> Result<Option<Vec<u8>>> {
        let output = Command::new("wl-paste")
            .arg("--type")
            .arg(mime_type)
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    Ok(Some(output.stdout))
                } else {
                    // Type not available in clipboard
                    debug!("MIME type {} not available in clipboard", mime_type);
                    Ok(None)
                }
            }
            Err(e) => {
                warn!("Failed to get clipboard content: {}", e);
                Ok(None)
            }
        }
    }

    /// Set clipboard content with a specific MIME type
    pub fn set_content(&self, mime_type: &str, data: &[u8]) -> Result<()> {
        let output = Command::new("wl-copy")
            .arg("--type")
            .arg(mime_type)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match output {
            Ok(mut child) => {
                // Write data to stdin
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(data)?;
                }

                let status = child.wait()?;
                if status.success() {
                    debug!("Set clipboard content: {} bytes, type={}", data.len(), mime_type);
                    Ok(())
                } else {
                    Err(anyhow!("wl-copy failed with status: {}", status))
                }
            }
            Err(e) => {
                Err(anyhow!("Failed to run wl-copy: {}", e))
            }
        }
    }

    /// Check if clipboard content has changed since last check
    ///
    /// Returns true if the clipboard content hash is different from the last recorded hash
    pub fn has_changed(&mut self) -> bool {
        // Get the first available MIME type and check for changes
        for mime_type in ["text/plain", "text/html", "image/png"] {
            if let Ok(Some(data)) = self.get_content(mime_type) {
                let hash = xxh3_64(&data);

                if self.last_content_hash != Some(hash) || self.last_mime_type.as_deref() != Some(mime_type) {
                    self.last_content_hash = Some(hash);
                    self.last_mime_type = Some(mime_type.to_string());
                    return true;
                }

                // Content matches our last recorded state
                return false;
            }
        }

        // No clipboard content available
        false
    }

    /// Get the current content hash (for preventing echo)
    pub fn current_hash(&self) -> Option<u64> {
        self.last_content_hash
    }

    /// Set the expected content hash (after setting clipboard from remote)
    pub fn set_expected_hash(&mut self, hash: u64, mime_type: &str) {
        self.last_content_hash = Some(hash);
        self.last_mime_type = Some(mime_type.to_string());
    }

    /// Get list of available MIME types in current clipboard
    pub fn available_types(&self) -> Vec<String> {
        let output = Command::new("wl-paste")
            .arg("--list-types")
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                } else {
                    Vec::new()
                }
            }
            Err(_) => Vec::new(),
        }
    }

    /// Get clipboard content with automatic MIME type detection
    ///
    /// Tries supported types in order of preference
    pub fn get_best_content(&self) -> Result<Option<(String, Vec<u8>)>> {
        // Priority order for clipboard content
        for mime_type in ["text/plain", "text/html", "image/png"] {
            if let Some(data) = self.get_content(mime_type)? {
                if !data.is_empty() {
                    return Ok(Some((mime_type.to_string(), data)));
                }
            }
        }

        Ok(None)
    }
}

impl Default for ClipboardHandler {
    fn default() -> Self {
        Self::new().expect("Failed to create ClipboardHandler")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires wl-clipboard and Wayland session
    fn test_clipboard_handler() {
        let handler = ClipboardHandler::new();
        assert!(handler.is_ok());
    }

    #[test]
    #[ignore] // Requires wl-clipboard and Wayland session
    fn test_clipboard_roundtrip() {
        let handler = ClipboardHandler::new().unwrap();

        // Set some text
        handler.set_content("text/plain", b"Hello, World!").unwrap();

        // Get it back
        let content = handler.get_content("text/plain").unwrap();
        assert!(content.is_some());
        assert_eq!(content.unwrap(), b"Hello, World!");
    }
}
