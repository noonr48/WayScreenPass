//! Client Clipboard Handler
//!
//! Provides clipboard access via SDL2 clipboard API

use anyhow::{Result, anyhow};
use sdl2::clipboard::ClipboardUtil;
use tracing::{debug, warn};
use xxhash_rust::xxh3::xxh3_64;

/// SDL2-based clipboard handler
pub struct ClipboardHandler {
    clipboard: ClipboardUtil,
    last_content_hash: Option<u64>,
}

impl ClipboardHandler {
    /// Create a new clipboard handler from a video subsystem
    pub fn new(video_subsystem: &sdl2::VideoSubsystem) -> Result<Self> {
        let clipboard = video_subsystem.clipboard();

        debug!("ClipboardHandler initialized");
        Ok(Self {
            clipboard,
            last_content_hash: None,
        })
    }

    /// Get clipboard text content
    pub fn get_text(&self) -> Result<Option<String>> {
        // Check if clipboard has text
        if !self.clipboard.has_clipboard_text() {
            return Ok(None);
        }

        match self.clipboard.clipboard_text() {
            Ok(text) => Ok(Some(text)),
            Err(e) => Err(anyhow!("Failed to get clipboard text: {}", e)),
        }
    }

    /// Set clipboard text content
    pub fn set_text(&mut self, text: &str) -> Result<()> {
        self.clipboard.set_clipboard_text(text)
            .map_err(|e| anyhow!("Failed to set clipboard text: {}", e))?;

        // Update hash to prevent echo
        self.last_content_hash = Some(xxh3_64(text.as_bytes()));
        debug!("Set clipboard text: {} chars", text.len());

        Ok(())
    }

    /// Check if clipboard content has changed since last check
    ///
    /// Returns true if the clipboard content hash is different from the last recorded hash
    pub fn has_changed(&mut self) -> bool {
        match self.get_text() {
            Ok(Some(text)) => {
                let hash = xxh3_64(text.as_bytes());

                if self.last_content_hash != Some(hash) {
                    self.last_content_hash = Some(hash);
                    return true;
                }

                false
            }
            Ok(None) => {
                // No text in clipboard
                if self.last_content_hash.is_some() {
                    // Previously had content, now empty - that's a change
                    self.last_content_hash = None;
                    return true;
                }
                false
            }
            Err(e) => {
                warn!("Error checking clipboard: {}", e);
                false
            }
        }
    }

    /// Get the current content hash (for preventing echo)
    #[allow(dead_code)]
    pub fn current_hash(&self) -> Option<u64> {
        self.last_content_hash
    }

    /// Set the expected content hash (after setting clipboard from remote)
    #[allow(dead_code)]
    pub fn set_expected_hash(&mut self, hash: u64) {
        self.last_content_hash = Some(hash);
    }

    /// Get clipboard content as bytes with MIME type
    ///
    /// For SDL2, we only support text/plain
    pub fn get_content(&mut self) -> Result<Option<(String, Vec<u8>)>> {
        if let Some(text) = self.get_text()? {
            return Ok(Some(("text/plain".to_string(), text.into_bytes())));
        }
        Ok(None)
    }

    /// Set clipboard content from remote
    ///
    /// Currently only text/plain and text/html are supported
    pub fn set_content(&mut self, mime_type: &str, data: &[u8]) -> Result<()> {
        match mime_type {
            "text/plain" | "text/html" => {
                let text = String::from_utf8_lossy(data);
                self.set_text(&text)?;
            }
            "image/png" => {
                // SDL2 doesn't support image clipboard directly
                // For now, just log and skip
                debug!("Received image clipboard, not supported by SDL2 clipboard");
            }
            _ => {
                warn!("Unsupported clipboard MIME type: {}", mime_type);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    
    fn test_clipboard_handler() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let handler = ClipboardHandler::new(&video);
        assert!(handler.is_ok());
    }
}
