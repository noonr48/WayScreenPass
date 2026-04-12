# WayScreenPass

Headless Wayland remote desktop over Tailscale.

WayScreenPass runs a dedicated headless Wayland session on the remote machine, captures that session with `grim`, streams it as H.264 over Tailscale, and lets you launch applications into the remoted desktop without relying on a portal prompt for each connection.

## Architecture

```
CLIENT (your machine)                    SERVER (remote machine)
┌──────────────────────┐                ┌──────────────────────────────┐
│ SDL2 Window          │                │ Headless Sway Session        │
│ H.264 Decoder (ffmpeg)│◄──TCP 3389──►│ H.264 Encoder (persistent x264)│
│ Keyboard/Mouse Input │  (Tailscale)   │ grim PNG capture → RGB24     │
│ Clipboard Sync       │                │ Clipboard Sync (wl-clipboard)│
└──────────────────────┘                └──────────────────────────────┘
```

## Features

- **Headless session model** — dedicated remoted desktop on the same machine and user account
- **No per-connection portal prompt** — headless mode avoids repeated screen-share approval dialogs
- **Persistent x264 encoder** — single process, NAL unit parser, no per-frame fork
- **Real monitor detection** — reads `/sys/class/drm/` sysfs for actual connected displays
- **Headless app launch** — `remote-desktop-server launch <command>` starts apps inside the remoted session
- **Proper input injection** — uinput remains available for the physical-session path
- **Correct keycode mapping** — SDL2 scancodes translated to Linux evdev codes (80+ keys)
- **Bidirectional clipboard** — event-driven via `wl-paste --watch`, supports text/html/image
- **Auto-reconnection** — client reconnects with exponential backoff on disconnect
- **Graceful shutdown** — SIGTERM/SIGINT signal handling with PID file cleanup
- **Dedicated headless compositor** — sway + grim based session runtime
- **Tailscale-only security** — nftables + IP verification (100.x.y.z range)
- **Adaptive quality** — 5 quality levels from 1080p@8Mbps to 576p@1.5Mbps
- **System tray app** — ksni-based tray for managing remote hosts

## Quick Start

### Build

```bash
cargo build --release
```

### Server Setup (one-time, on remote machine)

```bash
# 1. Install firewall rules
sudo nft -f nftables-rules.conf

# 2. Install headless-session runtime
# Arch/CachyOS:
sudo pacman -S sway grim

# 3. Optional: enable uinput for the physical-session path
sudo modprobe uinput
sudo usermod -aG input $USER
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf
# Re-login after this

# 4. Run setup wizard
./target/release/remote-desktop-server setup --authorize

# 5. Start the headless session server
./target/release/remote-desktop-server start --virtual

# 6. Launch an app into the headless session
./target/release/remote-desktop-server launch foot
```

### Connect (from your machine)

```bash
# List monitors on remote
./target/release/remote-desktop connect <tailscale-hostname> --list

# Connect (auto-selects the headless output)
./target/release/remote-desktop connect <tailscale-hostname>

# Connect to specific monitor
./target/release/remote-desktop connect <tailscale-hostname> --monitor HEADLESS-1
```

### System Tray

```bash
# Add a host
./target/release/remote-desktop-tray add mypc 100.77.233.119

# Launch tray icon
./target/release/remote-desktop-tray
```

### Headless Session Mode

```bash
# Requires: sway, swaymsg, grim
sudo pacman -S sway grim

./target/release/remote-desktop-server start --virtual

# Launch another application into the remoted desktop
./target/release/remote-desktop-server launch alacritty
```

## Configuration

`~/.config/remote-desktop/config.toml`:

```toml
[network]
port = 3389

[video]
fps = 30
quality = 23

[monitor]
auto_select_primary = true
```

## Crate Structure

| Crate | Type | Purpose |
|-------|------|---------|
| `core` | lib | Protocol, encoder, optional PipeWire capture, monitor detection |
| `portal` | lib | Optional portal sessions, input injection (evdev), clipboard |
| `server` | bin | TCP server, connection handler, headless session runtime |
| `client` | bin | TCP client, SDL2 display, ffmpeg decoder, input mapping |
| `tray` | bin | System tray (ksni), host management |

## Dependencies

| Dependency | Purpose |
|------------|---------|
| x264 | H.264 encoding (persistent pipe) |
| sway | Dedicated headless Wayland session |
| grim | Capture frames from the headless session |
| ashpd | Optional xdg-desktop-portal integration for physical-session capture |
| evdev | uinput virtual keyboard/mouse |
| SDL2 | Client display and input |
| ffmpeg | Client H.264 decoding |
| wl-clipboard | Clipboard sync (wl-paste --watch) |
| Tailscale | Network encryption and auth |
| nftables | Firewall (Tailscale-only) |
| PipeWire (optional) | Future portal/physical-session capture path |

## Tests

```bash
# Run all tests (set WAYLAND_DISPLAY for clipboard/display tests)
export WAYLAND_DISPLAY=wayland-0
cargo test --workspace

# 26 tests, 0 failures
```

## Security Model

- **Network**: Tailscale-only (100.x.y.z CGNAT range verified per-connection)
- **Encryption**: WireGuard via Tailscale (no app-layer crypto needed)
- **Authorization**: headless mode avoids per-connection screen-capture prompts by remoting a dedicated session instead of the live desktop
- **Input**: current default headless build prioritizes safe session launch/capture; uinput remains available for the physical-session path
- **Firewall**: nftables blocks all non-tailscale0 traffic to port 3389

## License

MIT OR Apache-2.0
