# Development Guidelines & Conventions

This document defines the coding standards, file structure rules, and development conventions for the remote-desktop-wayland project. **Follow these rules to maintain code consistency and prevent technical debt.**

---

## Table of Contents

1. [File Structure Conventions](#file-structure-conventions)
2. [Code Organization Rules](#code-organization-rules)
3. [Naming Conventions](#naming-conventions)
4. [Documentation Standards](#documentation-standards)
5. [Error Handling Patterns](#error-handling-patterns)
6. [Async Runtime Rules](#async-runtime-rules)
7. [Testing Conventions](#testing-conventions)
8. [Git Workflow](#git-workflow)

---

## File Structure Conventions

### Crate Organization

```
remote-desktop-wayland/
├── crates/
│   ├── core/               # Shared library - NO binary output
│   │   └── src/
│   │       ├── lib.rs      # Public API exports
│   │       ├── monitor.rs  # Monitor detection
│   │       ├── pipewire.rs # PipeWire integration
│   │       ├── encoder.rs  # Video encoding
│   │       └── protocol.rs # Protocol definitions
│   │
│   ├── portal/             # Portal integration - NO binary output
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── session.rs  # Portal session management
│   │       └── input.rs    # Input injection
│   │
│   ├── server/             # Server binary
│   │   └── src/
│   │       ├── main.rs     # Entry point + CLI
│   │       └── connection.rs # Connection handling
│   │
│   └── client/             # Client binary
│       └── src/
│           ├── main.rs     # Entry point + CLI
│           └── connection.rs # Connection handling
│
├── docs/                   # Documentation only
│   └── PROTOCOL.md
│
├── systemd/                # Service files only
├── config/                 # Example configs only
├── nftables-rules.conf
├── Cargo.toml
├── README.md
├── CODING.md               # THIS FILE
└── PROGRESS.md             # Implementation tracking
```

### File Placement Rules

| File Type | Location | Examples |
|-----------|----------|----------|
| Library code | `crates/<crate>/src/` | `monitor.rs`, `protocol.rs` |
| Binary entry | `crates/<bin>/src/main.rs` | Server/client mains |
| Tests | Alongside source (module `tests` mod) | `monitor.rs::tests` |
| Documentation | `docs/` or root | `PROTOCOL.md`, `README.md` |
| Config examples | `config/` | `*.conf.example` |
| Service files | `systemd/` | `*.service`, `*.socket` |

### Prohibited Locations

- ❌ **NEVER** put library code in `src/` at the root (use `crates/`)
- ❌ **NEVER** mix binary and library code in one crate (separate `core` from `server`/`client`)
- ❌ **NEVER** create arbitrary directories at root without approval
- ❌ **NEVER** put implementation details in `lib.rs` (use module files)

---

## Code Organization Rules

### Module Structure

Every Rust source file MUST follow this structure:

```rust
//! Crate/Module documentation (what, why, how)
//!
//! # Overview
//! Brief description of what this module does
//!
//! # Examples
//! ```rust
//! // Usage example
//! ```
//!
//! # Notes
//! Important implementation details or limitations

// 1. External imports (grouped by crate)
use std::{path::PathBuf, sync::Arc};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::module::SomeType;

// 2. Public types
/// Documentation for public type
pub struct PublicThing {
    /// Field documentation
    pub field: Type,
}

// 3. Public impl blocks
impl PublicThing {
    /// Public method documentation
    pub fn new() -> Self {
        Self { field: Default::default() }
    }
}

// 4. Private types
struct InternalState {
    data: Vec<u8>,
}

// 5. Private impl blocks
impl InternalState {
    fn process(&mut self) -> Result<()> {
        // ...
    }
}

// 6. Tests (at bottom of file)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        // ...
    }
}
```

### Re-export Rules

**Library crates only** (`core`, `portal`):

In `lib.rs`, re-export public API clearly:

```rust
//! Crate documentation

mod monitor;
mod encoder;

// Re-exports: Group by functionality
pub use monitor::{MonitorInfo, enumerate_monitors};
pub use encoder::{H264Encoder, EncodedFrame};
```

**Binary crates** (`server`, `client`):

```rust
mod connection;  // Private module, no re-exports

use connection::handle_client;  // Import locally only
```

---

## Naming Conventions

### Rust Naming Standards

| Category | Convention | Example |
|----------|------------|---------|
| Modules | `snake_case` | `mod video_stream;` |
| Types | `PascalCase` | `struct VideoFrame;` |
| Functions | `snake_case` | `fn enumerate_monitors()` |
| Constants | `SCREAMING_SNAKE_CASE` | `const MAX_FRAME_SIZE: usize` |
| Macros | `screaming_snake_case!` | `macro_rules! vec_of;` |

### Protocol-Related Names

- Message types: `ClientMessage`, `ServerMessage`
- Protocol enums: `FrameType`, `KeyState`, `PointerEventType`
- Codec functions: `encode_client_message()`, `parse_server_message()`

### File Naming

| Content | Extension | Example |
|---------|-----------|---------|
| Rust code | `.rs` | `monitor.rs` |
| Documentation | `.md` | `README.md` |
| Config | `.toml`, `.conf` | `config.toml` |
| Service files | `.service`, `.socket` | `remote-desktop.service` |

---

## Documentation Standards

### Module Documentation

Every module MUST have:
- `//!` doc comment at top
- Overview of purpose
- Usage examples (if non-trivial)
- Notes on limitations or stub status

### Function Documentation

Public functions MUST have:

```rust
/// Creates a new encoder instance
///
/// # Arguments
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
///
/// # Returns
/// Returns `Ok(H264Encoder)` if FFmpeg is available, otherwise
/// returns a stub encoder that produces empty frames.
///
/// # Errors
/// Returns an error if encoder initialization fails.
///
/// # Examples
/// ```rust
/// let encoder = H264Encoder::new(1920, 1080)?;
/// ```
pub fn new(width: u32, height: u32) -> Result<Self> {
    // ...
}
```

### Stub/Incomplete Code Markers

When implementing stubs, ALWAYS document why:

```rust
/// Encode a raw RGB frame
///
/// # Note
/// **STUB IMPLEMENTATION**: Returns empty encoded data.
///
/// FFmpeg 8.x removed avfft.h, breaking ffmpeg-sys-next build.
/// For production, consider:
/// - GPU encoding via VAAPI/VideoToolbox
/// - External x264 CLI via std::process
/// - Pure Rust encoder when available
///
/// # Returns
/// Always returns `Ok(EncodedFrame)` with empty data.
pub fn encode(&mut self, rgb_data: &[u8], stride: u32) -> Result<EncodedFrame> {
    warn!("Using stub encoder");
    Ok(EncodedFrame { data: vec![], ... })
}
```

---

## Error Handling Patterns

### Result Type Usage

```rust
// Standard function signature
pub fn do_work(&self) -> Result<Output, CoreError> {
    // ...
}

// Using anyhow in binaries (server/client)
use anyhow::Result;

pub fn run_server() -> Result<()> {
    let listener = TcpListener::bind(addr)?;
    // ...
    Ok(())
}
```

### Error Types

**Library crates** (`core`, `portal`): Define specific error types

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("PipeWire error: {0}")]
    PipeWire(String),

    #[error("No monitors found")]
    NoMonitors,
}
```

**Binary crates**: Use `anyhow::Result`

### Error Logging

```rust
use tracing::{error, warn, debug};

// Expected errors (user action, transient)
warn!("Failed to connect to portal: {:?}", e);

// Unexpected errors (bug)
error!("Critical failure: {:?}", e);

// Development debugging
debug!("Encoding frame {}: {} bytes", i, len);
```

---

## Async Runtime Rules

### Tokio Usage

**ALL async code MUST use Tokio:**

```toml
[dependencies]
tokio = { version = "1.40", features = ["full"] }
```

### Async Function Signatures

```rust
// Async methods return Future
pub async fn create_session(&self) -> Result<PortalSession> {
    // ...
}

// Use &mut self for stateful async operations
pub async fn send_key(&mut self, event: &KeyEvent) -> Result<()> {
    // ...
}
```

### Channel Patterns

For streaming video, use unbounded channels:

```rust
use tokio::sync::mpsc::unbounded_channel;

// Sender: Send frames to client
let (video_tx, mut video_rx) = unbounded_channel::<VideoFrame>();

// Receiver: In connection handler with tokio::select!
tokio::select! {
    Some(frame) = video_rx.recv() => {
        // Send frame to client
    }
    msg = socket.next() => {
        // Handle client message
    }
}
```

---

## Testing Conventions

### Test Placement

Tests go in the same file as the code:

```rust
// In monitor.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_monitors() {
        let monitors = enumerate_monitors().unwrap();
        assert!(!monitors.is_empty());
    }
}
```

### Test Naming

- Test functions: `test_<what>` (snake_case)
- Test modules: `mod tests` (snake_case)

### Integration Tests

For cross-module integration tests, create `tests/` directory in workspace root.

---

## Git Workflow

### Commit Messages

```
<type>: <subject>

<body>

<footer>
```

**Types:** `feat`, `fix`, `docs`, `refactor`, `test`, `chore`

**Examples:**

```
feat(server): add video streaming via channels

- Refactored handle_client to use tokio::select!
- Added unbounded channel for video frames
- Frames are now sent to client instead of dropped

Fixes #42
```

```
fix(client): resolve Cargo.toml dependency issue

The remote-desktop-core dependency was incorrectly nested
under crossterm, causing compilation failures.

Fixes #15
```

### Branch Naming

- `feature/<feature-name>` - New features
- `fix/<issue-name>` - Bug fixes
- `refactor/<area>` - Code refactoring
- `docs/<topic>` - Documentation updates

---

## Cargo.toml Conventions

### Workspace Dependencies

**NEVER** duplicate dependencies in workspace members:

```toml
# Cargo.toml (workspace root)
[workspace.dependencies]
tokio = { version = "1.40", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
tracing = "0.1"

# crates/*/Cargo.toml
[dependencies]
tokio = { workspace = true }  # ✅ Correct
serde = { workspace = true }  # ✅ Correct

# ❌ DON'T DO THIS:
# tokio = { version = "1.40" }  # Wrong! Use workspace inheritance
```

### Feature Flags

For optional features, define in workspace:

```toml
[workspace.dependencies]
ffmpeg-next = { package = "ffmpeg-next", version = "7.1", optional = true }

# Then in crate Cargo.toml:
[dependencies]
ffmpeg-next = { workspace = true, optional = true }

[features]
ffmpeg = ["ffmpeg-next"]
```

---

## Code Review Checklist

Before committing, verify:

- [ ] Code compiles with `cargo build --workspace`
- [ ] Tests pass with `cargo test --workspace`
- [ ] No `unused_import` or `dead_code` warnings (or explain why)
- [ ] Public types have `///` documentation
- [ ] Stub code is clearly marked with limitations
- [ ] Errors are properly typed and logged
- [ ] Async functions use `tokio::sync` primitives
- [ ] File follows module structure conventions
- [ ] README.md updated if public API changed

---

## Quick Reference

### Adding a New Module

1. Create file: `crates/<crate>/src/<module>.rs`
2. Add to `lib.rs` or `main.rs`: `mod <module>;`
3. Export public API: `pub use module::{Type, function};`
4. Add tests: `#[cfg(test)] mod tests { ... }`
5. Update this doc if introducing new patterns

### Updating Documentation

1. **README.md**: User-facing features, installation, quick start
2. **PROGRESS.md**: Implementation status, remaining work
3. **CODING.md**: This file (conventions only)
4. **docs/**: Deep technical dives (protocol specs, etc.)

---

## Version History

| Date | Changes |
|------|---------|
| 2025-02-10 | Initial version - established conventions for project structure |

---

**Remember**: Consistency prevents bugs. When in doubt, follow existing patterns in the codebase rather than introducing new ones.
