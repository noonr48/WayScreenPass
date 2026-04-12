//! Wayland Clipboard Handler
//!
//! Provides clipboard access via wl-copy/wl-paste commands (wl-clipboard package).
//! Uses `wl-paste --watch` for event-driven change detection.

use anyhow::{Result, anyhow};
use std::process::{Command, Stdio, Child};
use std::sync::mpsc;
use std::io::BufRead;
use tracing::{debug, info, warn};
use xxhash_rust::xxh3::xxh3_64;

/// Clipboard change event
#[derive(Debug, Clone)]
pub struct ClipboardChange {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ClipboardEnvironment {
    runtime_dir: String,
    wayland_display: String,
}

/// Wayland clipboard handler using wl-clipboard tools
pub struct ClipboardHandler {
    last_content_hash: Option<u64>,
    last_mime_type: Option<String>,
    watcher: Option<Child>,
    change_rx: Option<mpsc::Receiver<ClipboardChange>>,
    env: Option<ClipboardEnvironment>,
}

impl ClipboardHandler {
    /// Create a new clipboard handler
    pub fn new() -> Result<Self> {
        Self::with_wayland_session(None, None)
    }

    /// Create a clipboard handler bound to a specific Wayland session.
    pub fn new_for_wayland(runtime_dir: String, wayland_display: String) -> Result<Self> {
        Self::with_wayland_session(Some(runtime_dir), Some(wayland_display))
    }

    fn with_wayland_session(runtime_dir: Option<String>, wayland_display: Option<String>) -> Result<Self> {
        Self::check_wl_clipboard()?;

        let env = match (runtime_dir, wayland_display) {
            (Some(runtime_dir), Some(wayland_display)) => Some(ClipboardEnvironment {
                runtime_dir,
                wayland_display,
            }),
            _ => None,
        };

        let mut handler = Self {
            last_content_hash: None,
            last_mime_type: None,
            watcher: None,
            change_rx: None,
            env,
        };

        // Start the watcher automatically
        handler.start_watching();

        debug!("ClipboardHandler initialized with wl-paste --watch");
        Ok(handler)
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

    /// Start watching clipboard for changes using wl-paste --watch
    fn start_watching(&mut self) {
        // wl-paste --watch runs a command each time clipboard changes.
        // We use `echo CHANGED` so it prints a marker line on each change,
        // then we fetch the actual content when we see the marker.

        let mut command = Command::new("wl-paste");
        self.apply_wayland_env(&mut command);

        let mut child = match command
            .args(["--watch", "echo", "CHANGED"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn() {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to start wl-paste --watch: {}. Clipboard monitoring disabled.", e);
                return;
            }
        };

        // Take stdout from child (child still owns the process for kill)
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                warn!("wl-paste --watch has no stdout");
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        let env = self.env.clone();

        // Spawn a thread to read watcher output
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(text) if text.trim() == "CHANGED" => {
                        // Clipboard changed - fetch current content
                        // Try text/plain first, then text/html, then image/png
                        for mime in &["text/plain", "text/html", "image/png"] {
                            let mut cmd = Command::new("wl-paste");
                            apply_wayland_env_for_session(env.as_ref(), &mut cmd);
                            match cmd.arg("--no-newline")
                                .arg("--type")
                                .arg(mime)
                                .output() {
                                Ok(output) if output.status.success() && !output.stdout.is_empty() => {
                                    let _ = tx.send(ClipboardChange {
                                        mime_type: mime.to_string(),
                                        data: output.stdout,
                                    });
                                    break;
                                }
                                _ => continue,
                            }
                        }
                    }
                    Ok(_) => {} // ignore non-CHANGED lines
                    Err(_) => break, // pipe closed
                }
            }
            debug!("wl-paste --watch reader thread exiting");
        });

        self.watcher = Some(child);
        self.change_rx = Some(rx);

        info!("Clipboard watcher started (wl-paste --watch)");
    }

    /// Get current clipboard content for a specific MIME type
    pub fn get_content(&self, mime_type: &str) -> Result<Option<Vec<u8>>> {
        let mut command = Command::new("wl-paste");
        self.apply_wayland_env(&mut command);
        let output = command.arg("--no-newline")
            .arg("--type")
            .arg(mime_type)
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    Ok(Some(output.stdout))
                } else {
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
        let mut command = Command::new("wl-copy");
        self.apply_wayland_env(&mut command);
        let output = command.arg("--type")
            .arg(mime_type)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        match output {
            Ok(mut child) => {
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
    /// Uses the event-driven wl-paste --watch channel if available,
    /// falls back to polling otherwise.
    pub fn has_changed(&mut self) -> bool {
        if let Some(rx) = &self.change_rx {
            // Non-blocking check for clipboard changes
            match rx.try_recv() {
                Ok(change) => {
                    let hash = xxh3_64(&change.data);
                    if self.last_content_hash != Some(hash) {
                        self.last_content_hash = Some(hash);
                        self.last_mime_type = Some(change.mime_type);
                        return true;
                    }
                    false
                }
                Err(mpsc::TryRecvError::Empty) => false,
                Err(mpsc::TryRecvError::Disconnected) => {
                    warn!("Clipboard watcher disconnected");
                    false
                }
            }
        } else {
            // Fallback to old polling if watcher not available
            self.has_changed_poll()
        }
    }

    /// Fallback polling-based change detection
    fn has_changed_poll(&mut self) -> bool {
        for mime_type in ["text/plain", "text/html", "image/png"] {
            if let Ok(Some(data)) = self.get_content(mime_type) {
                let hash = xxh3_64(&data);

                if self.last_content_hash != Some(hash) || self.last_mime_type.as_deref() != Some(mime_type) {
                    self.last_content_hash = Some(hash);
                    self.last_mime_type = Some(mime_type.to_string());
                    return true;
                }

                return false;
            }
        }

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
        let mut command = Command::new("wl-paste");
        self.apply_wayland_env(&mut command);
        let output = command.arg("--list-types")
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
        for mime_type in ["text/plain", "text/html", "image/png"] {
            if let Some(data) = self.get_content(mime_type)? {
                if !data.is_empty() {
                    return Ok(Some((mime_type.to_string(), data)));
                }
            }
        }

        Ok(None)
    }

    fn apply_wayland_env(&self, command: &mut Command) {
        apply_wayland_env_for_session(self.env.as_ref(), command);
    }
}

impl Drop for ClipboardHandler {
    fn drop(&mut self) {
        if let Some(mut child) = self.watcher.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn apply_wayland_env_for_session(env: Option<&ClipboardEnvironment>, command: &mut Command) {
    if let Some(env) = env {
        command
            .env("XDG_RUNTIME_DIR", &env.runtime_dir)
            .env("WAYLAND_DISPLAY", &env.wayland_display)
            .env_remove("DISPLAY");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    
    fn test_clipboard_handler() {
        let handler = ClipboardHandler::new();
        assert!(handler.is_ok());
    }

    #[test]
    
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
