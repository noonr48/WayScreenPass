# WayScreenPass

Headless Wayland remote desktop over Tailscale.

WayScreenPass starts a dedicated headless Sway session on the remote machine, captures that session with `grim`, encodes H.264 through a persistent `x264` process, and streams it to the client over Tailscale. The supported server model is now **headless-only**: it does not depend on xdg-desktop-portal approval prompts and does not try to remote the live local desktop.

## Current Status

- **Implemented:** headless session startup, video streaming, app launch, clipboard sync, and input injection.
- **Input path:** keyboard and pointer control target the dedicated session through Wayland virtual keyboard/pointer protocols.
- **Deployment:** setup guidance, status output, and packaged systemd service are aligned with unattended boot/loginless use.
- **Validated:** `cargo test --workspace` and `cargo build --release` both pass.
- **Still recommended:** a live smoke test on the target machine to confirm compositor/runtime specifics.

## Architecture

```text
CLIENT (your machine)                    SERVER (remote machine)
┌──────────────────────┐                ┌──────────────────────────────┐
│ SDL2 Window          │                │ Headless Sway Session        │
│ H.264 Decoder        │◄──TCP 3389──►│ grim capture -> RGB24        │
│ Keyboard/Mouse Input │  (Tailscale)   │ x264 encoder (persistent)    │
│ Clipboard Sync       │                │ Wayland virtual input        │
└──────────────────────┘                │ wl-clipboard sync            │
                                        └──────────────────────────────┘
```

## What You Get

- **Dedicated remoted desktop** on the same machine and user account
- **No per-connection screen-share prompt**
- **Wayland-scoped virtual input** for the headless session
- **App launch inside the remoted session** with `remote-desktop-server launch`
- **Bidirectional clipboard sync**
- **Persistent x264 encoding** instead of per-frame process spawning
- **Tailscale-first security model**

## Quick Start

### 1. Build

```bash
cargo build --release
```

### 2. Install runtime dependencies on the remote machine

Example for Arch/CachyOS:

```bash
sudo pacman -S sway grim wl-clipboard x264 ffmpeg sdl2 tailscale nftables
```

### 3. Run setup and start the headless server

```bash
./target/release/remote-desktop-server setup
./target/release/remote-desktop-server start
```

### 4. Optional: enable unattended boot

```bash
systemctl --user enable --now remote-desktop.service
sudo loginctl enable-linger "$USER"
```

### 5. Launch an application into the headless session

```bash
./target/release/remote-desktop-server launch foot
```

### 6. Connect from your machine

```bash
./target/release/remote-desktop connect <tailscale-hostname> --list
./target/release/remote-desktop connect <tailscale-hostname>
```

To target the explicit headless output:

```bash
./target/release/remote-desktop connect <tailscale-hostname> --monitor HEADLESS-1
```

## Useful Server Commands

```bash
./target/release/remote-desktop-server status
./target/release/remote-desktop-server list-monitors
./target/release/remote-desktop-server launch <command> [args...]
```

## Crate Layout

| Crate | Purpose |
|-------|---------|
| `core` | Shared protocol, encoder, monitor/capture helpers |
| `portal` | Headless input backend, clipboard helpers, retained portal session code |
| `server` | Headless session runtime, TCP server, setup/status commands |
| `client` | SDL2 client, decoder, input capture |
| `tray` | System tray frontend for saved hosts |

## Runtime Dependencies

| Dependency | Purpose |
|------------|---------|
| `sway` | Dedicated headless Wayland session |
| `grim` | Frame capture from the headless output |
| `wl-clipboard` | Clipboard sync |
| `x264` | H.264 encoding |
| `ffmpeg` | Client-side decoding |
| `SDL2` | Client window and input capture |
| `Tailscale` | Network security and access |
| `nftables` | Optional Tailscale-only firewall policy |

## Security Model

- **Network:** Tailscale-only access is the intended deployment model
- **Desktop isolation:** the remoted desktop is a dedicated headless session, not the live local session
- **Input scope:** injected input targets the headless session socket, not the whole system
- **Firewall:** `nftables-rules.conf` can restrict port 3389 to `tailscale0`

## License

MIT OR Apache-2.0
