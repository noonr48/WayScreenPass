# Development Guidelines & Conventions

**Updated**: 2026-04-06

---

## Crate Organization

```
WayScreenPass/
├── crates/
│   ├── core/               # Shared library - NO binary output
│   │   └── src/
│   │       ├── lib.rs       # Public API exports
│   │       ├── monitor.rs   # sysfs DRM monitor detection
│   │       ├── pipewire.rs  # PipeWire screen capture
│   │       ├── encoder.rs   # Persistent x264 encoder + NAL parser
│   │       └── protocol.rs  # Binary protocol (13 message types)
│   │
│   ├── portal/             # Headless input + clipboard helpers - NO binary output
│   │   └── src/
│   │       ├── lib.rs       # Exports, PortalError type
│   │       ├── session.rs   # Optional ashpd portal session support
│   │       ├── input.rs     # Headless Wayland virtual keyboard/pointer
│   │       └── clipboard.rs # wl-paste --watch, wl-copy
│   │
│   ├── server/             # Server binary
│   │   └── src/
│   │       ├── main.rs      # CLI, signal handling
│   │       ├── connection.rs # Client handler, video streaming
│   │       ├── setup.rs     # Setup wizard
│   │       ├── status.rs    # PID file, status display
│   │       └── virtual_display.rs # Headless compositor
│   │
│   ├── client/             # Client binary
│   │   └── src/
│   │       ├── main.rs      # CLI, auto-reconnect loop
│   │       ├── connection.rs # Protocol handler, VideoPlayer
│   │       ├── decoder.rs   # ffmpeg H.264 decoder
│   │       ├── display.rs   # SDL2 window, YUV rendering
│   │       ├── input.rs     # scancode→evdev mapping
│   │       └── clipboard.rs # SDL2 clipboard handler
│   │
│   └── tray/               # System tray binary
│       └── src/
│           ├── main.rs      # Tray app, event loop
│           ├── tray.rs      # ksni implementation
│           ├── config.rs    # hosts.toml management
│           └── connection.rs # Spawn client connections
│
├── config/                  # Example configs
├── docs/                    # Protocol spec, deep dives
├── Cargo.toml               # Workspace config
├── CODING.md                # THIS FILE
├── README.md                # User-facing docs
├── IMPLEMENTATION_SUMMARY.md # Technical summary
├── PROGRESS.md              # Progress tracker
└── TEST_RESULTS.md          # Test results
```

## Rules

### File Placement

| File Type | Location |
|-----------|----------|
| Library code | `crates/<crate>/src/` |
| Binary entry | `crates/<bin>/src/main.rs` |
| Tests | Same file, `#[cfg(test)] mod tests` |
| Docs | Root `*.md` or `docs/` |
| Config examples | `config/` |
| Service files | `systemd/` |

### Prohibited

- Never put library code in root `src/`
- Never mix binary and library in one crate
- Never duplicate workspace dependencies in member Cargo.toml
- Never put implementation logic in `lib.rs` (use module files)

## Code Style

### Module Structure

```rust
//! Module documentation

use std::...;          // 1. External imports
use crate::...;        // 2. Internal imports

pub struct Thing { }   // 3. Public types
impl Thing { }         // 4. Public impls

struct Internal { }    // 5. Private types
impl Internal { }      // 6. Private impls

#[cfg(test)]           // 7. Tests (bottom of file)
mod tests { }
```

### Naming

| Category | Convention | Example |
|----------|------------|---------|
| Modules | `snake_case` | `mod video_stream;` |
| Types | `PascalCase` | `struct VideoFrame;` |
| Functions | `snake_case` | `fn enumerate_monitors()` |
| Constants | `SCREAMING_SNAKE_CASE` | `const MAX_FRAME_SIZE: usize` |

### Error Handling

- Library crates (`core`, `portal`): use `thiserror` custom error types
- Binary crates (`server`, `client`, `tray`): use `anyhow::Result`
- Always log errors with `tracing::{error, warn, debug}`

### Async

- All async code uses Tokio
- Streaming uses `mpsc::unbounded_channel`
- Connection handling uses `tokio::select!`

### Dependencies

```toml
# In workspace root Cargo.toml:
[workspace.dependencies]
tokio = { version = "1.40", features = ["full"] }

# In crate Cargo.toml:
[dependencies]
tokio = { workspace = true }    # Always use workspace inheritance
```

## Testing

```bash
# Full suite (needs WAYLAND_DISPLAY for clipboard/display)
export WAYLAND_DISPLAY=wayland-0
cargo test --workspace

# Core only (no display needed)
cargo test -p remote-desktop-core

# With output
cargo test --workspace -- --nocapture
```

Target: zero warnings, zero ignored tests.

## Git Workflow

### Commit Messages

```
<type>(<scope>): <subject>

Types: feat, fix, docs, refactor, test, chore
Example: fix(encoder): replace per-frame x264 with persistent pipe
```

### Pre-Commit Checklist

- [ ] `cargo build --release` — zero warnings
- [ ] `cargo test --workspace` — all pass
- [ ] Public types have `///` documentation
- [ ] No `unsafe` code without justification

---

**Last updated**: 2026-04-06
