# Remote Desktop Protocol Specification

## Overview

Custom binary protocol optimized for:
- **Low latency**: Inspired by Sunshine/Moonlight (< 50ms target)
- **Adaptive quality**: Inspired by NoMachine's dynamic adjustment
- **Simplicity**: Single TCP connection over Tailscale (encryption handled by Tailscale)

## Transport Layer

**Protocol**: TCP over Tailscale
**Default Port**: 3389
**Encryption**: WireGuard (via Tailscale) - no application-layer encryption needed

## Message Framing

All messages use length-prefixed binary framing:

```
[Length: u32] [Type: u8] [Payload: bytes]
```

- **Length**: Total payload size (Type + Payload bytes) in network byte order
- **Type**: Message type identifier (see below)
- **Payload**: Message-specific data

## Message Types

### Client → Server

| Type | Name | Description |
|------|------|-------------|
| 0x01 | Hello | Initial handshake, version negotiation |
| 0x02 | SelectMonitor | Choose which monitor to stream |
| 0x03 | KeyEvent | Keyboard event |
| 0x04 | PointerEvent | Mouse movement/click |
| 0x05 | ClipboardEvent | Clipboard data |
| 0x06 | QualityRequest | Request quality change (bitrate/resolution) |
| 0x07 | Ping | Keep-alive/RTT measurement |
| 0x08 | PauseStream | Pause video streaming |
| 0x09 | ResumeStream | Resume video streaming |

### Server → Client

| Type | Name | Description |
|------|------|-------------|
| 0x10 | HelloAck | Handshake response |
| 0x11 | MonitorList | Available monitors |
| 0x12 | StreamStart | Stream beginning (codec info) |
| 0x13 | VideoFrame | H.264 encoded frame |
| 0x14 | AudioData | Opus encoded audio (future) |
| 0x15 | Pong | Ping response |
| 0x16 | StreamStats | Current stream quality stats |
| 0x17 | Error | Error condition |

## Message Payloads

### Hello (0x01) - Client → Server

```rust
struct Hello {
    protocol_version: u16,      // Always 1
    client_name: String,         // UTF-8 string
    supported_codecs: Vec<String>, // ["h264"]
    max_width: u16,
    max_height: u16,
    max_fps: u8,
}
```

### HelloAck (0x10) - Server → Client

```rust
struct HelloAck {
    protocol_version: u16,
    server_name: String,
    selected_codec: String,
}
```

### MonitorList (0x11) - Server → Client

```rust
struct MonitorList {
    monitors: Vec<MonitorInfo>,
}

struct MonitorInfo {
    name: String,           // e.g., "DP-1"
    width: u16,
    height: u16,
    refresh_rate: u8,
    is_primary: bool,
}
```

### SelectMonitor (0x02) - Client → Server

```rust
struct SelectMonitor {
    monitor_name: String,
    requested_width: Option<u16>,
    requested_height: Option<u16>,
    requested_fps: Option<u8>,
}
```

### StreamStart (0x12) - Server → Client

```rust
struct StreamStart {
    width: u16,
    height: u16,
    fps: u8,
    codec: String,          // "h264"
    // H.264 specific
    profile: u8,            // 66 = Baseline, 77 = Main
    level: u8,              // 31 = 3.1
}
```

### VideoFrame (0x13) - Server → Client

```rust
struct VideoFrame {
    frame_type: u8,         // 0 = P-frame, 1 = I-frame (keyframe)
    timestamp_us: u64,      // Microseconds
    data: Vec<u8>,          // H.264 NAL unit
}
```

### KeyEvent (0x03) - Client → Server

```rust
struct KeyEvent {
    key_code: u32,          // Linux key code
    state: u8,              // 0 = released, 1 = pressed
}
```

### PointerEvent (0x04) - Client → Server

```rust
struct PointerEvent {
    event_type: u8,         // 0 = motion, 1 = button, 2 = scroll
    // For motion:
    x: Option<u16>,
    y: Option<u16>,
    // For button:
    button: Option<u8>,     // 1=left, 2=middle, 3=right
    button_state: Option<u8>, // 0=released, 1=pressed
    // For scroll:
    scroll_delta: Option<i16>,
}
```

### QualityRequest (0x06) - Client → Server

```rust
struct QualityRequest {
    target_bitrate_kbps: Option<u16>,  // Target bitrate in kbps
    target_resolution: Option<(u16, u16)>, // (width, height)
    target_fps: Option<u8>,
}
```

### Ping (0x07) / Pong (0x15)

```rust
struct Ping {
    timestamp_us: u64,      // Client timestamp
}

struct Pong {
    timestamp_us: u64,      // Echoed client timestamp
    server_timestamp_us: u64, // Server time
}
```

### StreamStats (0x16) - Server → Client

```rust
struct StreamStats {
    current_bitrate_kbps: u16,
    current_width: u16,
    current_height: u16,
    current_fps: u8,
    dropped_frames: u32,
}
```

## Adaptive Quality Algorithm

Inspired by NoMachine and WebRTC:

### Server-Side Adaptation

**Measurements:**
1. **RTT** (Round-Trip Time): From ping/pong messages
2. **Frame encoding time**: Time to encode H.264 frame
3. **Network bandwidth**: Measured by throughput
4. **Client decode time**: Reported by client

**Adaptation Strategy:**

```rust
fn should_adapt_quality(measurements: &Measurements) -> Option<QualityAdjustment> {
    // High latency or slow encoding
    if measurements.rtt > 100_000 // 100ms
        || measurements.encode_time_us > 33_000 // > 33ms (30fps)
    {
        return Some(QualityAdjustment::Decrease);
    }

    // Good conditions, can increase quality
    if measurements.rtt < 30_000 // 30ms
        && measurements.encode_time_us < 20_000 // 20ms
        && measurements.bitrate_kbps < MAX_BITRATE
    {
        return Some(QualityAdjustment::Increase);
    }

    None
}

enum QualityAdjustment {
    Decrease {
        // Reduce resolution or bitrate
        new_bitrate_kbps: u16,
        or_new_resolution: (u16, u16),
    },
    Increase {
        new_bitrate_kbps: u16,
        or_new_resolution: (u16, u16),
    },
}
```

**Quality Levels:**

| Level | Resolution | Bitrate | FPS | Use Case |
|-------|------------|---------|-----|----------|
| 0 | 1920x1080 | 8000 Kbps | 30 | Excellent network |
| 1 | 1920x1080 | 5000 Kbps | 30 | Good network |
| 2 | 1600x900 | 3000 Kbps | 30 | Fair network |
| 3 | 1280x720 | 2000 Kbps | 24 | Poor network |
| 4 | 1024x576 | 1500 Kbps | 24 | Very poor network |

## Flow Diagram

```
Client                              Server
  |                                   |
  |----------- Hello ---------------->|
  |<------ HelloAck + MonitorList ----|
  |                                   |
  |-------- SelectMonitor (DP-1) ---->|
  |                                   |
  |<-------- StreamStart (H.264) -----|
  |<---- VideoFrame (I-frame) --------|
  |<---- VideoFrame (P-frame) --------|
  |<---- VideoFrame (P-frame) --------|
  |                                   |
  |----------- KeyEvent -------------->|
  |----------- PointerEvent ---------->|
  |                                   |
  |----------- Ping ------------------>|
  |<----------- Pong ------------------|
  |                                   |
  |<-------- StreamStats -------------|
  |------- QualityRequest (lower) ---->|
  |                                   |
  |<---- VideoFrame (lower bitrate) --|
```

## Error Handling

### Error Codes (0x17)

| Code | Name | Description |
|------|------|-------------|
| 1 | MonitorNotFound | Selected monitor doesn't exist |
| 2 | UnsupportedCodec | Client doesn't support server codec |
| 3 | InvalidResolution | Requested resolution not available |
| 4 | StreamFailed | Screen capture or encoding failed |
| 5 | RateLimited | Too many connections |

```rust
struct Error {
    code: u8,
    message: String,
}
```

## Performance Targets

| Metric | Target | Excellent |
|--------|--------|-----------|
| End-to-end latency | < 100ms | < 50ms |
| RTT | < 50ms | < 20ms |
| Frame encode time | < 33ms | < 20ms |
| Bitrate (1080p@30) | 5000 Kbps | 8000 Kbps |
| Keyframe interval | 2 seconds | 1 second |

## Security Considerations

1. **Tailscale**: All traffic encrypted via WireGuard
2. **nftables**: Firewall blocks non-tailscale0 connections
3. **No auth in protocol**: Tailscale provides authentication
4. **Rate limiting**: Server enforces max clients

## Future Enhancements

1. **UDP video channel**: For lower latency (optional)
2. **FEC**: Forward error correction for packet loss
3. **Audio**: Opus codec support
4. **Clipboard**: Bidirectional sync
5. **File transfer**: Virtual filesystem
6. **Multiple monitors**: Simultaneous streaming
