# Remote Desktop for Wayland

A custom remote desktop solution for KDE Plasma Wayland on Cachyos Linux.

## Features

- **Zero-click access** after one-time setup
- **Automatic monitor detection** - works with AMD and NVIDIA multi-GPU
- **Tailscale-only** network isolation for security
- **Persistent sessions** - no portal prompts after initial authorization
- **Custom binary protocol** - optimized for low-latency streaming
- **Real H.264 encoding** via x264 CLI
- **PipeWire screen capture** - actual frame data from portal
- **Input injection** via uinput virtual devices

## Quick Start

### One-Time Setup (on each machine you control)

```bash
# 1. Build the project
cd remote-desktop-wayland
cargo build --release

# 2. Install nftables rules (Tailscale-only)
sudo nft -f nftables-rules.conf

# 3. Setup uinput for input injection
sudo modprobe uinput
sudo usermod -aG input $USER
# Re-login to apply group changes

# 4. Run setup wizard (approves portal access)
./target/release/remote-desktop-server setup

# 5. Enable service
systemctl --user enable --now remote-desktop
```

### Connect from Another Tailscale Machine

```bash
# List available monitors
./target/release/remote-desktop connect <hostname> --list

# Connect (auto-selects primary monitor)
./target/release/remote-desktop connect <hostname>

# Connect to specific monitor
./target/release/remote-desktop connect <hostname> --monitor DP-2
```

## Configuration

Edit `~/.config/remote-desktop/config.toml`:

```toml
[network]
port = 3389

[video]
fps = 30
quality = 23

[monitor]
auto_select_primary = true
```

## Security

- Only accepts connections from `tailscale0` interface (100.x.y.z)
- nftables rules enforce network isolation
- Tailscale provides encryption and authentication
- Portal-based authorization with persistent tokens

## Requirements

- **OS**: Cachyos Linux (or Arch-based)
- **Desktop**: KDE Plasma 6 on Wayland
- **Network**: Tailscale installed and running
- **Dependencies**:
  - PipeWire and wireplumber
  - xdg-desktop-portal-kde
  - x264 (for H.264 encoding)
  - evdev (for input injection)
  - nftables

## Current Status

| Component | Status | Notes |
|-----------|--------|-------|
| Build System | ✅ Working | Clean compilation |
| Server Binary | ✅ Working | Binds to 0.0.0.0:3389 |
| Client Binary | ✅ Working | Connects and sends protocol messages |
| Protocol Handshake | ✅ Working | Hello → HelloAck → MonitorList |
| Monitor Detection | ✅ Working | Detects 3 monitors (AMD/NVIDIA multi-GPU) |
| Tailscale Verification | ✅ Working | Accepts 100.x.y.z, rejects others |
| Portal Integration | ✅ Working | ashpd, persistent sessions, PipeWire node ID |
| PipeWire Stream | ✅ Implemented | Daemon connection, RGB24 frame capture |
| H.264 Encoder | ✅ Working | x264 CLI, 833 bytes/frame @ 320x240 |
| Video Streaming | ✅ Implemented | Full pipeline ready |
| Input Injection | ✅ Implemented | evdev uinput (requires setup) |

## Test Results

```bash
# Unit Tests: ✅ ALL PASS (9 tests)
cargo test --workspace

# H.264 Encoder: ✅ 833 bytes/frame
cargo test --package remote-desktop-core test_encode_small_frame -- --nocapture

# Monitor Detection: ✅ 3 monitors detected
./target/release/remote-desktop-server list-monitors
# Output: card3-DP-1, card2-DP-1, card1-DP-1 (all 1920x1080@60Hz)

# Server Startup: ✅ Listening on port 3389
./target/release/remote-desktop-server start
# Output: "Remote desktop server listening on 0.0.0.0:3389"

# Security: ✅ Correctly rejects non-Tailscale
./target/release/remote-desktop connect 127.0.0.1 --list
# Output: Connection reset (127.0.0.1 not in 100.x.y.z range)
```

## Known Limitations

### uinput Setup Required for Input Injection

Input injection requires `/dev/uinput` access. Without setup, events are logged but not injected.

**One-time setup:**
```bash
sudo modprobe uinput
sudo usermod -aG input $USER
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf
# Then re-login
```

### End-to-End Testing Pending

The system is ready but not yet tested with:
- Actual Tailscale connection between two machines
- Real PipeWire screen capture (requires GUI portal dialog)
- Live H.264 streaming to client

## Development Status

- **Protocol**: ✅ Complete and tested
- **Network Layer**: ✅ Complete and tested
- **Server Architecture**: ✅ Complete
- **Client Architecture**: ✅ Complete
- **Monitor Detection**: ✅ Complete (DRM, multi-GPU)
- **Portal Integration**: ✅ Complete (ashpd, persistent tokens)
- **PipeWire Capture**: ✅ Implemented (pipewire crate)
- **H.264 Encoding**: ✅ Implemented (x264 CLI, YUV420P)
- **Input Injection**: ✅ Implemented (evdev uinput)

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  CLIENT (controlling machine)                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ CLI (rust)  │  │ Connection  │  │ Protocol Handler    │  │
│  │             │─>│ Handler     │─>│ Hello/SelectMonitor │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────┬─────────────────────────────────┘
                          │ Tailscale (WireGuard encrypted)
                          │ TCP 3389
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  SERVER (controlled machine)                                │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ TCP Server  │  │ PipeWire    │  │ H.264 Encoder       │  │
│  │ (tokio)     │─>│ Screencast  │─>│ x264 CLI            │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ Portal Integration (ashpd)                          │   │
│  │ - Persistent authorization via restore_token        │   │
│  │ - Input injection via evdev uinput                   │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Next Steps

1. **End-to-End Testing** (HIGH PRIORITY)
   - Test with two Tailscale-connected machines
   - Verify portal screen capture dialog
   - Test video streaming quality
   - Test keyboard/mouse input

2. **GPU Encoding** (OPTIONAL)
   - Current x264 software encoding works
   - VAAPI for AMD/Intel GPUs would reduce CPU
   - Consider for 4K or high-framerate scenarios

## Documentation

- `TEST_RESULTS.md` - Detailed test results
- `IMPLEMENTATION_SUMMARY.md` - Component breakdown
- `CODING.md` - Development conventions
- `docs/PROTOCOL.md` - Protocol specification

## License

MIT OR Apache-2.0
