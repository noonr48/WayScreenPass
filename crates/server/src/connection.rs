//! Server connection handler
//!
//! Manages individual client connections and protocol message handling

use remote_desktop_core::*;
use remote_desktop_portal::{InputHandler, InputBackend, SessionManager, KeyEvent as PortalKeyEvent, KeyState as PortalKeyState,
                            PointerEvent as PortalPointerEvent, ButtonState as PortalButtonState, ClipboardHandler};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::Framed;
use futures_util::{StreamExt, SinkExt};
use tracing::{debug, error, info, warn};
use xxhash_rust::xxh3::xxh3_64;

/// Video frame to send to client
#[derive(Clone)]
struct VideoFrameToSend {
    frame_type: FrameType,
    timestamp_us: u64,
    data: Vec<u8>,
}

/// Shared server state
#[derive(Clone)]
pub struct ServerState {
    pub clients: Arc<Mutex<std::collections::HashMap<String, ClientInfo>>>,
    pub input_handler: Arc<Mutex<Option<InputHandler>>>,
    pub portal_node_id: Arc<Mutex<Option<u32>>>,
    pub clipboard: Arc<Mutex<Option<ClipboardHandler>>>,
    pub clipboard_enabled: bool,
    pub virtual_mode: Arc<Mutex<bool>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(Mutex::new(std::collections::HashMap::new())),
            input_handler: Arc::new(Mutex::new(None)),
            portal_node_id: Arc::new(Mutex::new(None)),
            clipboard: Arc::new(Mutex::new(None)),
            clipboard_enabled: true,
            virtual_mode: Arc::new(Mutex::new(false)),
        }
    }

    pub fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    pub fn set_virtual_mode(&self, enabled: bool) {
        if let Ok(mut guard) = self.virtual_mode.try_lock() {
            *guard = enabled;
        }
    }

    pub async fn is_virtual_mode(&self) -> bool {
        *self.virtual_mode.lock().await
    }

    pub async fn init_input_handler(&self, backend: InputBackend) {
        let mut handler_guard = self.input_handler.lock().await;
        if handler_guard.is_none() {
            match InputHandler::new(backend) {
                Ok(handler) => {
                    *handler_guard = Some(handler);
                    info!("Input handler initialized with {:?} backend", backend);
                }
                Err(e) => {
                    warn!("Failed to initialize input handler: {}", e);
                }
            }
        }
    }

    pub async fn init_portal_session(&self) -> anyhow::Result<u32> {
        let mut node_id_guard = self.portal_node_id.lock().await;
        if let Some(node_id) = *node_id_guard {
            return Ok(node_id);
        }

        info!("Creating portal session for screen capture");
        let manager = SessionManager::new().await?;
        let session = manager.create_persistent_session().await?;
        let node_id = session.pipewire_node_id();

        *node_id_guard = Some(node_id);
        info!("Portal session created, node ID: {}", node_id);

        Ok(node_id)
    }

    pub async fn init_clipboard(&self) {
        if !self.clipboard_enabled {
            return;
        }

        let mut clipboard_guard = self.clipboard.lock().await;
        if clipboard_guard.is_none() {
            match ClipboardHandler::new() {
                Ok(handler) => {
                    *clipboard_guard = Some(handler);
                    info!("Clipboard handler initialized");
                }
                Err(e) => {
                    warn!("Failed to initialize clipboard handler: {}", e);
                }
            }
        }
    }
}

/// Information about a connected client
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub client_name: String,
    pub connected_at: std::time::Instant,
    pub selected_monitor: Option<String>,
    pub current_quality: QualityLevel,
    pub pipewire_node_id: Option<u32>,
}

/// Current streaming quality level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityLevel {
    Level0, // 1920x1080 @ 30fps, 8000 Kbps
    Level1, // 1920x1080 @ 30fps, 5000 Kbps
    Level2, // 1600x900 @ 30fps, 3000 Kbps
    Level3, // 1280x720 @ 24fps, 2000 Kbps
    Level4, // 1024x576 @ 24fps, 1500 Kbps
}

impl QualityLevel {
    pub fn resolution(&self) -> (u16, u16) {
        match self {
            QualityLevel::Level0 => (1920, 1080),
            QualityLevel::Level1 => (1920, 1080),
            QualityLevel::Level2 => (1600, 900),
            QualityLevel::Level3 => (1280, 720),
            QualityLevel::Level4 => (1024, 576),
        }
    }

    pub fn fps(&self) -> u8 {
        match self {
            QualityLevel::Level0 | QualityLevel::Level1 | QualityLevel::Level2 => 30,
            QualityLevel::Level3 | QualityLevel::Level4 => 24,
        }
    }

    pub fn bitrate_kbps(&self) -> u16 {
        match self {
            QualityLevel::Level0 => 8000,
            QualityLevel::Level1 => 5000,
            QualityLevel::Level2 => 3000,
            QualityLevel::Level3 => 2000,
            QualityLevel::Level4 => 1500,
        }
    }

    pub fn decrease(&self) -> Self {
        match self {
            QualityLevel::Level0 => QualityLevel::Level1,
            QualityLevel::Level1 => QualityLevel::Level2,
            QualityLevel::Level2 => QualityLevel::Level3,
            QualityLevel::Level3 => QualityLevel::Level4,
            QualityLevel::Level4 => QualityLevel::Level4,
        }
    }

    pub fn increase(&self) -> Self {
        match self {
            QualityLevel::Level0 => QualityLevel::Level0,
            QualityLevel::Level1 => QualityLevel::Level0,
            QualityLevel::Level2 => QualityLevel::Level1,
            QualityLevel::Level3 => QualityLevel::Level2,
            QualityLevel::Level4 => QualityLevel::Level3,
        }
    }
}

/// Connection metrics for adaptive quality
#[derive(Debug, Clone)]
pub struct ConnectionMetrics {
    pub rtt_us: u64,
    pub encode_time_us: u64,
    pub bitrate_kbps: u16,
    pub dropped_frames: u32,
}

impl Default for ConnectionMetrics {
    fn default() -> Self {
        Self {
            rtt_us: 50_000, // 50ms default
            encode_time_us: 20_000, // 20ms default
            bitrate_kbps: 5000,
            dropped_frames: 0,
        }
    }
}

/// Handle a single client connection
pub async fn handle_client(
    socket: TcpStream,
    peer_addr: std::net::SocketAddr,
    state: ServerState,
) -> anyhow::Result<()> {
    info!("New client connection from {}", peer_addr);

    // Verify connection is from Tailscale
    if !is_tailscale_connection(peer_addr) {
        warn!("Rejecting non-Tailscale connection from {}", peer_addr);
        return Ok(()); // Silently drop non-tailscale connections
    }

    let mut framed = Framed::new(socket, ProtocolCodec);
    let client_id = peer_addr.to_string();
    let mut client_info = None;
    let mut metrics = ConnectionMetrics::default();
    let mut quality = QualityLevel::Level1; // Start at reasonable quality
    let mut streaming_paused = false; // Track pause state

    // Channel for video frames from streaming task
    let (video_tx, mut video_rx) = mpsc::unbounded_channel::<VideoFrameToSend>();
    let mut streaming_handle: Option<tokio::task::JoinHandle<()>> = None;

    // Channel for clipboard events
    let (clipboard_tx, mut clipboard_rx) = mpsc::unbounded_channel::<remote_desktop_core::protocol::ClipboardEvent>();
    let mut clipboard_handle: Option<tokio::task::JoinHandle<()>> = None;

    // Initialize clipboard handler and start monitoring if enabled
    if state.clipboard_enabled {
        state.init_clipboard().await;

        let clipboard = state.clipboard.clone();
        let clipboard_tx_clone = clipboard_tx.clone();
        let handle = tokio::spawn(async move {
            // Poll clipboard every 500ms for changes
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;

                let mut guard = clipboard.lock().await;
                if let Some(handler) = guard.as_mut() {
                    if handler.has_changed() {
                        // Get the new content
                        if let Ok(Some((mime_type, data))) = handler.get_best_content() {
                            debug!("Clipboard changed, sending to client: {} bytes, type={}", data.len(), mime_type);
                            let _ = clipboard_tx_clone.send(remote_desktop_core::protocol::ClipboardEvent {
                                mime_type,
                                data,
                            });
                        }
                    }
                }
            }
        });
        clipboard_handle = Some(handle);
    }

    // Main message loop with video frame handling
    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg_result = framed.next() => {
                match msg_result {
                    Some(Ok(data)) => {
                        debug!("Received message from {}", peer_addr);

                        // Parse client message
                        match parse_client_message(data) {
                            Ok(msg) => {
                                if let Err(e) = handle_client_message(
                                    &mut framed,
                                    msg,
                                    &state,
                                    &client_id,
                                    &mut client_info,
                                    &mut metrics,
                                    &mut quality,
                                    &video_tx,
                                    &mut streaming_handle,
                                    &mut streaming_paused,
                                ).await {
                                    error!("Error handling message: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse message: {}", e);
                                // Send error response
                                let error_msg = ServerMessage::ErrorMessage(Error {
                                    code: ErrorCode::StreamFailed,
                                    message: format!("Invalid message: {}", e),
                                });
                                if let Ok(encoded) = encode_server_message(&error_msg) {
                                    let _ = framed.send(encoded).await;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("Connection error from {}: {}", peer_addr, e);
                        break;
                    }
                    None => {
                        info!("Client {} disconnected", peer_addr);
                        break;
                    }
                }
            }
            // Handle outgoing video frames
            Some(frame) = video_rx.recv() => {
                if streaming_paused {
                    // Drop frame when paused to prevent queue buildup
                    debug!("Stream paused, dropping frame");
                } else {
                    let frame_msg = ServerMessage::VideoFrame(remote_desktop_core::protocol::VideoFrame {
                        frame_type: frame.frame_type,
                        timestamp_us: frame.timestamp_us,
                        data: frame.data,
                    });
                    if let Ok(encoded) = encode_server_message(&frame_msg) {
                        if let Err(e) = framed.send(encoded).await {
                            error!("Failed to send video frame: {}", e);
                            break;
                        }
                    }
                }
            }
            // Handle outgoing clipboard events
            Some(event) = clipboard_rx.recv() => {
                let clipboard_msg = ServerMessage::ClipboardEvent(event);
                if let Ok(encoded) = encode_server_message(&clipboard_msg) {
                    debug!("Sending clipboard event to client");
                    if let Err(e) = framed.send(encoded).await {
                        error!("Failed to send clipboard event: {}", e);
                        break;
                    }
                }
            }
        }
    }

    // Stop streaming task if running
    if let Some(handle) = streaming_handle {
        handle.abort();
    }

    // Stop clipboard monitoring task if running
    if let Some(handle) = clipboard_handle {
        handle.abort();
    }

    // Clean up client state
    state.clients.lock().await.remove(&client_id);
    info!("Connection closed for {}", peer_addr);

    Ok(())
}

/// Check if a connection is from Tailscale or localhost (for testing)
fn is_tailscale_connection(peer_addr: std::net::SocketAddr) -> bool {
    match peer_addr {
        std::net::SocketAddr::V4(addr) => {
            let ip = addr.ip();
            let octets = ip.octets();
            // Allow localhost for testing
            if octets == [127, 0, 0, 1] {
                return true;
            }
            // Tailscale uses 100.x.y.z addresses (CGNAT range)
            octets[0] == 100
        }
        std::net::SocketAddr::V6(addr) => {
            // Allow localhost IPv6 for testing (::1)
            if addr.ip().is_loopback() {
                return true;
            }
            // Tailscale IPv6 addresses (fd7a:115c:a1e0::/48)
            let segments = addr.ip().segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    }
}

/// Handle a client message
async fn handle_client_message(
    framed: &mut Framed<TcpStream, ProtocolCodec>,
    msg: ClientMessage,
    state: &ServerState,
    client_id: &str,
    client_info: &mut Option<ClientInfo>,
    metrics: &mut ConnectionMetrics,
    quality: &mut QualityLevel,
    video_tx: &mpsc::UnboundedSender<VideoFrameToSend>,
    streaming_handle: &mut Option<tokio::task::JoinHandle<()>>,
    streaming_paused: &mut bool,
) -> anyhow::Result<()> {
    match msg {
        ClientMessage::Hello(hello) => {
            info!("Hello from client: {} (version {})", hello.client_name, hello.protocol_version);

            // Check protocol version
            if hello.protocol_version != PROTOCOL_VERSION {
                warn!("Unsupported protocol version: {}", hello.protocol_version);
                let error_msg = ServerMessage::ErrorMessage(Error {
                    code: ErrorCode::UnsupportedCodec,
                    message: format!("Unsupported protocol version: {}", hello.protocol_version),
                });
                let encoded = encode_server_message(&error_msg)?;
                framed.send(encoded).await?;
                return Ok(());
            }

            // Create client info
            *client_info = Some(ClientInfo {
                client_name: hello.client_name.clone(),
                connected_at: std::time::Instant::now(),
                selected_monitor: None,
                current_quality: *quality,
                pipewire_node_id: None, // Will be set when stream starts
            });

            // Get available monitors
            let monitors = enumerate_monitors()?;

            // Send HelloAck
            let ack = ServerMessage::HelloAck(HelloAck {
                protocol_version: PROTOCOL_VERSION,
                server_name: "remote-desktop-wayland".to_string(),
                selected_codec: "h264".to_string(),
            });
            let encoded = encode_server_message(&ack)?;
            framed.send(encoded).await?;

            // Send MonitorList
            let monitor_list = ServerMessage::MonitorList(MonitorList {
                monitors: monitors.into_iter().map(|m| {
                    remote_desktop_core::protocol::MonitorInfo {
                        name: m.name,
                        width: m.resolution.0 as u16,
                        height: m.resolution.1 as u16,
                        refresh_rate: m.refresh_rate as u8,
                        is_primary: m.is_primary,
                    }
                }).collect(),
            });
            let encoded = encode_server_message(&monitor_list)?;
            framed.send(encoded).await?;

            // Store client in state
            if let Some(info) = client_info.clone() {
                state.clients.lock().await.insert(client_id.to_string(), info);
            }

            info!("Sent HelloAck and MonitorList to {}", hello.client_name);
        }

        ClientMessage::SelectMonitor(sel) => {
            info!("Client selected monitor: {}", sel.monitor_name);

            // Find the requested monitor
            let monitors = enumerate_monitors()?;
            let monitor = monitors.iter()
                .find(|m| m.name == sel.monitor_name)
                .ok_or_else(|| anyhow::anyhow!("Monitor not found: {}", sel.monitor_name))?;

            // Clone the monitor name before spawning
            let monitor_name = monitor.name.clone();

            // Update client info
            if let Some(info) = client_info.as_mut() {
                info.selected_monitor = Some(sel.monitor_name.clone());
            }

            // Determine stream parameters
            let (width, height) = sel.requested_width.and_then(|w|
                sel.requested_height.map(|h| (w, h))
            ).unwrap_or_else(|| {
                // Use client's quality level
                quality.resolution()
            });

            let fps = sel.requested_fps.unwrap_or_else(|| {
                quality.fps()
            });

            // Send StreamStart
            let stream_start = ServerMessage::StreamStart(StreamStart {
                width,
                height,
                fps,
                codec: "h264".to_string(),
                profile: 66, // Baseline profile
                level: 31,   // 3.1
            });
            let encoded = encode_server_message(&stream_start)?;
            framed.send(encoded).await?;

            info!("Started stream for monitor {} ({}x{}@{}fps)", sel.monitor_name, width, height, fps);

            // Stop any existing streaming task
            if let Some(handle) = streaming_handle.take() {
                handle.abort();
            }

            // Check if in virtual mode
            let is_virtual = state.is_virtual_mode().await;

            // Start video streaming task with channel sender
            let video_tx_clone = video_tx.clone();

            let handle = if is_virtual {
                // Virtual mode - use test pattern generator (no portal needed)
                info!("Using virtual display mode");
                tokio::spawn(async move {
                    if let Err(e) = start_virtual_streaming(video_tx_clone, width, height, fps).await {
                        error!("Virtual streaming error: {}", e);
                    }
                })
            } else {
                // Normal mode - use portal session
                let node_id = match state.init_portal_session().await {
                    Ok(nid) => Some(nid),
                    Err(e) => {
                        warn!("Failed to create portal session: {}, using stub mode", e);
                        None
                    }
                };

                let node_id_for_task = node_id;
                tokio::spawn(async move {
                    if let Err(e) = start_video_streaming(video_tx_clone, monitor_name, width, height, fps, node_id_for_task).await {
                        error!("Video streaming error: {}", e);
                    }
                })
            };
            *streaming_handle = Some(handle);
        }

        ClientMessage::KeyEvent(event) => {
            debug!("Key event: code={}, state={:?}", event.key_code, event.state);

            // Convert protocol event to portal event
            let portal_event = PortalKeyEvent {
                keycode: event.key_code,
                state: match event.state {
                    remote_desktop_core::protocol::KeyState::Pressed => PortalKeyState::Pressed,
                    remote_desktop_core::protocol::KeyState::Released => PortalKeyState::Released,
                },
            };

            // Send to input handler if available
            if let Some(mut handler) = state.input_handler.lock().await.as_mut() {
                if let Err(e) = handler.send_key(&portal_event) {
                    error!("Failed to send key event: {}", e);
                }
            }
        }

        ClientMessage::PointerEvent(event) => {
            debug!("Pointer event: type={:?}, x={:?}, y={:?}", event.event_type, event.x, event.y);

            // Convert protocol event to portal event
            let portal_event = match event.event_type {
                remote_desktop_core::protocol::PointerEventType::Motion => {
                    // Use absolute positioning with normalized coordinates
                    PortalPointerEvent::MotionAbsolute {
                        x: event.x.unwrap_or(0) as f64 / 1920.0,
                        y: event.y.unwrap_or(0) as f64 / 1080.0,
                    }
                }
                remote_desktop_core::protocol::PointerEventType::Button => {
                    PortalPointerEvent::Button {
                        button: event.button.unwrap_or(1) as u32,
                        state: match event.button_state {
                            Some(remote_desktop_core::protocol::ButtonState::Pressed) => PortalButtonState::Pressed,
                            Some(remote_desktop_core::protocol::ButtonState::Released) => PortalButtonState::Released,
                            None => PortalButtonState::Pressed,
                        },
                    }
                }
                remote_desktop_core::protocol::PointerEventType::Scroll => {
                    PortalPointerEvent::Scroll {
                        delta_x: 0,
                        delta_y: event.scroll_delta.unwrap_or(0) as i32,
                    }
                }
            };

            // Send to input handler if available
            if let Some(mut handler) = state.input_handler.lock().await.as_mut() {
                if let Err(e) = handler.send_pointer(&portal_event) {
                    error!("Failed to send pointer event: {}", e);
                }
            }
        }

        ClientMessage::QualityRequest(req) => {
            info!("Quality request: bitrate={:?}, resolution={:?}, fps={:?}",
                req.target_bitrate_kbps, req.target_resolution, req.target_fps);

            if let Some(bitrate) = req.target_bitrate_kbps {
                metrics.bitrate_kbps = bitrate;
                // Adjust quality level based on requested bitrate
                *quality = match bitrate {
                    8000.. => QualityLevel::Level0,
                    5000..=7999 => QualityLevel::Level1,
                    3000..=4999 => QualityLevel::Level2,
                    2000..=2999 => QualityLevel::Level3,
                    _ => QualityLevel::Level4,
                };
            }
        }

        ClientMessage::Ping(ping) => {
            // Send Pong with timestamps for RTT measurement
            let pong = ServerMessage::Pong(Pong {
                timestamp_us: ping.timestamp_us,
                server_timestamp_us: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_micros() as u64,
            });
            let encoded = encode_server_message(&pong)?;
            framed.send(encoded).await?;
        }

        ClientMessage::PauseStream => {
            info!("Client requested pause");
            *streaming_paused = true;
        }

        ClientMessage::ResumeStream => {
            info!("Client requested resume");
            *streaming_paused = false;
        }

        ClientMessage::ClipboardEvent(event) => {
            debug!("Received clipboard: {} bytes, type={}", event.data.len(), event.mime_type);

            // Set clipboard content
            let clipboard_guard = state.clipboard.lock().await;
            if let Some(handler) = clipboard_guard.as_ref() {
                if let Err(e) = handler.set_content(&event.mime_type, &event.data) {
                    warn!("Failed to set clipboard: {}", e);
                } else {
                    // Update hash to prevent echo back to clients
                    let hash = xxh3_64(&event.data);
                    drop(clipboard_guard);

                    // Update the hash in a new guard
                    let mut clipboard_guard = state.clipboard.lock().await;
                    if let Some(handler) = clipboard_guard.as_mut() {
                        handler.set_expected_hash(hash, &event.mime_type);
                    }

                    info!("Clipboard updated from client: {} bytes, type={}", event.data.len(), event.mime_type);
                }
            }
        }
    }

    Ok(())
}

/// Start video streaming for a monitor
async fn start_video_streaming(
    video_tx: mpsc::UnboundedSender<VideoFrameToSend>,
    monitor_name: String,
    width: u16,
    height: u16,
    fps: u8,
    node_id: Option<u32>,
) -> anyhow::Result<()> {
    info!("Starting video stream for {} ({}x{}@{}fps)", monitor_name, width, height, fps);

    // Create video stream from portal node ID, or fall back to stub
    let mut stream = if let Some(nid) = node_id {
        info!("Using PipeWire node ID: {}", nid);
        VideoStream::from_node_id(nid, width as u32, height as u32).await?
    } else {
        warn!("No portal node ID, using stub mode");
        VideoStream::new(&monitor_name).await?
    };

    // Start screen capture
    use std::os::fd::FromRawFd;
    let dummy_fd = unsafe { OwnedFd::from_raw_fd(0) };
    stream.start(dummy_fd, width as u32, height as u32)?;

    // Create encoder
    let mut encoder = H264Encoder::new(width as u32, height as u32)?;

    let frame_duration_us = 1_000_000 / fps as u64;
    let mut frame_count = 0u64;
    let mut last_keyframe = 0u64;

    loop {
        let start = std::time::Instant::now();

        // Capture frame
        match stream.capture_frame()? {
            Some(frame) => {
                // Calculate stride (RGB24 = 3 bytes per pixel)
                let stride = frame.width * 3;

                // Encode frame
                match encoder.encode(&frame.data, stride) {
                    Ok(encoded) => {
                        let data_len = encoded.data.len();
                        let is_keyframe = encoded.is_keyframe;

                        // Determine frame type
                        let frame_type = if is_keyframe {
                            FrameType::IFrame
                        } else {
                            FrameType::PFrame
                        };

                        // Send video frame through channel
                        let _ = video_tx.send(VideoFrameToSend {
                            frame_type,
                            timestamp_us: frame_count * frame_duration_us,
                            data: encoded.data,
                        });

                        if frame_count % 30 == 0 {
                            debug!("Encoded frame {}: {} bytes, keyframe={}",
                                frame_count, data_len, is_keyframe);
                        }

                        frame_count += 1;
                        if is_keyframe {
                            last_keyframe = frame_count;
                        }

                        // Force keyframe every 2 seconds (at 30fps = every 60 frames)
                        if frame_count - last_keyframe > 60 {
                            encoder.request_keyframe()?;
                            last_keyframe = frame_count;
                        }
                    }
                    Err(e) => {
                        warn!("Encoding error: {}", e);
                    }
                }
            }
            None => {
                // No frame available yet - wait a bit
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        // Maintain frame rate
        let elapsed = start.elapsed().as_micros() as u64;
        if elapsed < frame_duration_us {
            tokio::time::sleep(std::time::Duration::from_micros(frame_duration_us - elapsed)).await;
        }
    }
}

/// Start video streaming for virtual display (generates test patterns)
async fn start_virtual_streaming(
    video_tx: mpsc::UnboundedSender<VideoFrameToSend>,
    width: u16,
    height: u16,
    fps: u8,
) -> anyhow::Result<()> {
    info!("Starting virtual video stream ({}x{}@{}fps)", width, height, fps);

    // Create encoder
    let mut encoder = H264Encoder::new(width as u32, height as u32)?;

    let frame_duration_us = 1_000_000 / fps as u64;
    let mut frame_count = 0u64;
    let mut last_keyframe = 0u64;

    // Generate test pattern frames
    let stride = width as usize * 3; // RGB24
    let frame_size = stride * height as usize;
    let mut frame_data = vec![0u8; frame_size];

    loop {
        let start = std::time::Instant::now();

        // Generate a moving test pattern
        let offset = (frame_count % 256) as u8;

        for y in 0..height as usize {
            for x in 0..width as usize {
                let idx = (y * stride) + (x * 3);
                // Create a moving gradient pattern
                frame_data[idx] = ((x as u8).wrapping_add(offset)) ^ 0x55;     // R
                frame_data[idx + 1] = ((y as u8).wrapping_add(offset)) ^ 0xAA; // G
                frame_data[idx + 2] = offset;                                   // B
            }
        }

        // Add a "frame counter" text-like pattern in the corner
        let counter_x = 20;
        let counter_y = 20;
        let counter_str = format!("Frame: {}", frame_count);
        for (i, ch) in counter_str.chars().enumerate() {
            let cx = counter_x + i * 8;
            let cy = counter_y;
            if cx < width as usize && cy < height as usize {
                // Draw a simple representation
                let idx = (cy * stride) + (cx * 3);
                if idx + 2 < frame_data.len() {
                    frame_data[idx] = 255;     // R - white text
                    frame_data[idx + 1] = 255; // G
                    frame_data[idx + 2] = 255; // B
                }
            }
        }

        // Encode frame
        match encoder.encode(&frame_data, stride as u32) {
            Ok(encoded) => {
                let data_len = encoded.data.len();
                let is_keyframe = encoded.is_keyframe;

                // Determine frame type
                let frame_type = if is_keyframe {
                    FrameType::IFrame
                } else {
                    FrameType::PFrame
                };

                // Send video frame through channel
                let _ = video_tx.send(VideoFrameToSend {
                    frame_type,
                    timestamp_us: frame_count * frame_duration_us,
                    data: encoded.data,
                });

                if frame_count % 30 == 0 {
                    info!("Virtual frame {}: {} bytes, keyframe={}",
                        frame_count, data_len, is_keyframe);
                }

                frame_count += 1;
                if is_keyframe {
                    last_keyframe = frame_count;
                }

                // Force keyframe every 2 seconds
                if frame_count - last_keyframe > 60 {
                    encoder.request_keyframe()?;
                    last_keyframe = frame_count;
                }
            }
            Err(e) => {
                warn!("Virtual encoding error: {}", e);
            }
        }

        // Maintain frame rate
        let elapsed = start.elapsed().as_micros() as u64;
        if elapsed < frame_duration_us {
            tokio::time::sleep(std::time::Duration::from_micros(frame_duration_us - elapsed)).await;
        }
    }
}

/// Start the TCP server
pub async fn start_server(bind_addr: &str, state: ServerState) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    info!("Remote desktop server listening on {}", bind_addr);

    loop {
        match listener.accept().await {
            Ok((socket, peer_addr)) => {
                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(socket, peer_addr, state_clone).await {
                        error!("Client handler error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }
}
