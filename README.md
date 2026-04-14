# WayScreenPass

Headless Wayland remote desktop over Tailscale.

A Rust-based remote desktop solution that creates a dedicated headless Sway session on Linux, captures it with `grim`, encodes as H.264 using a persistent `x264` process, and streams to a client over a custom TCP protocol. Keyboard and pointer input are injected back into the headless session through Wayland virtual input protocols.

## Architecture

```
CLIENT MACHINE                          REMOTE MACHINE
┌──────────────────────┐               ┌──────────────────────────────┐
│ remote-desktop       │               │ remote-desktop-server        │
│ SDL2 window          │               │ dedicated headless sway      │
│ ffmpeg H.264 decode  │◄─ TCP 3389 ──►│ grim capture (per-frame)     │
│ input capture        │  over         │ persistent x264 encoder      │
│ clipboard sync       │  Tailscale    │ wl-clipboard sync            │
└──────────────────────┘               │ zwp_virtual_keyboard_v1      │
                                       │ zwlr_virtual_pointer_v1      │
                                       └──────────────────────────────┘
```

## Current Status

| Component | Status | Details |
|-----------|--------|---------|
| Headless session | ✅ | Sway with headless wlroots backend |
| Video capture | ✅ | `grim` per-frame capture to PNG → RGB |
| Video encode | ✅ | Persistent `x264` process (Y4M over stdin/stdout) |
| Video decode | ✅ | `ffmpeg` via `ffmpeg-next` (H.264 → YUV420P) |
| Display | ✅ | SDL2 window with YUV texture rendering |
| Input (keyboard) | ✅ | Virtual keyboard via `zwp_virtual_keyboard_v1` |
| Input (pointer) | ✅ | Virtual pointer via `zwlr_virtual_pointer_v1` |
| Clipboard | ✅ | `wl-copy`/`wl-paste` with event-driven monitoring |
| Protocol | ✅ | Custom binary protocol (length-prefixed) |
| Reconnect | ✅ | Automatic reconnect with exponential backoff |
| Systemd | ✅ | User service for unattended startup |

## Server Commands

```bash
# Run setup wizard (installs dependencies, creates config)
remote-desktop-server setup

# Start headless session and server
remote-desktop-server start [options]

# Show server status
remote-desktop-server status

# List available monitors (shows headless output)
remote-desktop-server list-monitors

# Launch an application into the headless session
remote-desktop-server launch <command> [args...]
```

### Server Options

| Option | Default | Description |
|--------|---------|-------------|
| `--port` | 3389 | TCP port to bind (0.0.0.0) |
| `--width` | 1920 | Virtual display width |
| `--height` | 1080 | Virtual display height |
| `--refresh` | 60 | Virtual display refresh rate |

## Client Commands

```bash
# Connect to a remote host
remote-desktop connect <hostname> [options]

# List monitors on remote host
remote-desktop list-monitors <hostname> [options]
```

### Client Options

| Option | Default | Description |
|--------|---------|-------------|
| `--port` | 3389 | Server port |
| `--list` | false | List monitors instead of connecting |
| `--monitor` | auto | Specific monitor name (auto-selects primary) |
| `--test-duration` | 0 | Exit after N seconds (testing) |

## Quick Start

### 1. Build

```bash
cargo build --release
```

Binaries:
- `target/release/remote-desktop-server` (server)
- `target/release/remote-desktop` (client)

### 2. Remote Machine Setup

Install dependencies (Arch/CachyOS):

```bash
sudo pacman -S sway grim wl-clipboard x264 tailscale
```

Start the server:

```bash
./target/release/remote-desktop-server start
```

Or with custom resolution:

```bash
./target/release/remote-desktop-server start --width 1920 --height 1080 --refresh 60
```

### 3. Launch Applications

Run applications inside the headless session:

```bash
./target/release/remote-desktop-server launch foot
./target/release/remote-desktop-server launch alacritty
./target/release/remote-desktop-server launch firefox
```

### 4. Connect from Client

```bash
./target/release/remote-desktop connect <tailscale-hostname>
```

Or with explicit monitor selection:

```bash
./target/release/remote-desktop connect <tailscale-hostname> --monitor HEADLESS-1
```

## Protocol Details

### Message Flow

1. **Client → Server**: `Hello` (protocol version, capabilities)
2. **Server → Client**: `HelloAck` (acknowledgment)
3. **Server → Client**: `MonitorList` (available outputs)
4. **Client → Server**: `SelectMonitor` (chosen output)
5. **Server → Client**: `StreamStart` (stream parameters)
6. **Server → Client**: `VideoFrame` (H.264 access units, loop)
7. **Client → Server**: `KeyEvent`/`PointerEvent`/`ClipboardEvent` (input)

### Frame Encoding

- **Format**: H.264 Annex B (start-code delimited NAL units)
- **Parser**: Background thread extracts access units from x264 stdout
- **Keyframe**: IDR slices every 60 frames (configurable)
- **Color**: YUV420P (RGB24 → YUV420P conversion before encoding)

### Input Mapping

- **Keyboard**: SDL2 scancode (USB HID) → Linux evdev keycode
- **Pointer**: Normalized absolute coordinates (0.0–1.0) for motion
- **Scroll**: Delta in 1/120 units (high-resolution scroll)

### Clipboard

- **Types**: `text/plain`, `text/html`, `image/png`
- **Sync**: Event-driven via `wl-paste --watch` (500ms poll fallback)
- **Hash**: XXH3-64 for change detection and echo suppression

## Systemd Integration

### Unattended Startup

Create user service:

```bash
cat > ~/.config/systemd/user/remote-desktop.service <<EOF
[Unit]
Description=WayScreenPass Headless Remote Desktop
After=network.target

[Service]
Type=simple
ExecStart=/home/benbi/Desktop/waylandsec/WayScreenPass/target/release/remote-desktop-server start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

systemctl --user enable --now remote-desktop.service
sudo loginctl enable-linger "$USER"
```

Monitor logs:

```bash
journalctl --user -u remote-desktop -f
```

## Security Model

- **Network**: Tailscale-first (CGNAT 100.x.y.z or fd7a:115c:a1e0::/48)
- **Binding**: Server binds 0.0.0.0:3389 (network restriction expected)
- **Session**: Dedicated headless Wayland session (not live desktop)
- **Scope**: Input injection scoped to headless session socket

Optional firewall (nftables):

```bash
sudo nft -f nftables-rules.conf
```

## Workspace Structure

| Crate | Path | Purpose |
|-------|------|---------|
| `core` | `crates/core/` | Protocol, encoder, monitor helpers |
| `portal` | `crates/portal/` | Headless input backend, clipboard |
| `server` | `crates/server/` | Server CLI, connection handler |
| `client` | `crates/client/` | Client CLI, SDL2/ffmpeg |
| `tray` | `crates/tray/` | System tray launcher (future) |

## Key Implementation Details

### Headless Session

- **Compositor**: Sway with `WLR_BACKENDS=headless`
- **Output**: Created via `swaymsg create_output` (HEADLESS-*)
- **Metadata**: Persisted to `~/.remote-desktop-headless-session.json`
- **Launch**: `remote-desktop-server launch` sets env vars and spawns

### Video Pipeline

1. `grim -o HEADLESS-1 -` captures PNG
2. `image` crate decodes PNG → RGB24
3. RGB24 → YUV420P conversion
4. Y4M header + FRAME + YUV data to `x264` stdin
5. Background thread parses H.264 NAL units from stdout
6. Access units sent to client over TCP

### Input Pipeline

1. Client SDL2 events → protocol messages
2. Protocol messages → Wayland virtual input events
3. `zwp_virtual_keyboard_v1` for keyboard (XKB keymap)
4. `zwlr_virtual_pointer_v1` for pointer (absolute + scroll)

## Requirements

### Remote Machine

- `sway` (compositor)
- `swaymsg` (IPC control)
- `grim` (screenshot)
- `wl-clipboard` (clipboard)
- `x264` (encoder)
- `tailscale` (network)

### Client Machine

- `ffmpeg` (decoder)
- `SDL2` (display/input)
- `tailscale` (network)

## Validation

```bash
# Run tests
cargo test --workspace

# Build release
cargo build --release

# Check formatting
cargo fmt --check

# Clippy linting
cargo clippy --workspace
```

## License

MIT OR Apache-2.0