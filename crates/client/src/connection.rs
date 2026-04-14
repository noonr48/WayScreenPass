//! Client connection handler
//!
//! Manages connection to remote desktop server

use remote_desktop_core::protocol::*;
use remote_desktop_core::{encode_client_message, parse_server_message, ServerMessage, PROTOCOL_VERSION, ProtocolCodec};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::Framed;
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, error, info, warn};
use anyhow::Result;

use crate::decoder::VideoDecoder;
use crate::display::Display;
use crate::input::InputHandler;
use crate::clipboard::ClipboardHandler;

/// Messages that can be received from the server
#[derive(Debug)]
#[allow(dead_code)]
pub enum ReceivedMessage {
    VideoFrame(remote_desktop_core::protocol::VideoFrame),
    ClipboardEvent(ClipboardEvent),
    StreamStats(StreamStats),
    Pong(Pong),
}

/// Client connection state
#[allow(dead_code)]
pub struct ClientConnection {
    framed: Framed<TcpStream, ProtocolCodec>,
    server_addr: String,
    connected: bool,
    selected_monitor: Option<String>,
    stream_active: bool,
}

#[allow(dead_code)]
impl ClientConnection {
    /// Create a new client connection
    pub async fn connect(server_addr: &str) -> Result<Self> {
        info!("Connecting to remote desktop server at {}", server_addr);

        // Connect with timeout
        let socket = timeout(Duration::from_secs(5), TcpStream::connect(server_addr)).await??;

        let framed = Framed::new(socket, ProtocolCodec);

        let mut conn = Self {
            framed,
            server_addr: server_addr.to_string(),
            connected: false,
            selected_monitor: None,
            stream_active: false,
        };

        // Perform handshake
        conn.handshake().await?;

        Ok(conn)
    }

    /// Perform protocol handshake
    async fn handshake(&mut self) -> Result<()> {
        // Send Hello
        let hello = ClientMessage::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "remote-desktop-client".to_string(),
            supported_codecs: vec!["h264".to_string()],
            max_width: 1920,
            max_height: 1080,
            max_fps: 30,
        });

        let encoded = encode_client_message(&hello)?;
        self.framed.send(encoded).await?;
        info!("Sent Hello to {}", self.server_addr);

        // Receive HelloAck
        let hello_ack_msg = self.expect_message_type(ServerMessageType::HelloAck).await?;
        if let ServerMessage::HelloAck(hello_ack) = hello_ack_msg {
            info!("Received HelloAck from {} (protocol version {})",
                self.server_addr, hello_ack.protocol_version);
        }

        // Receive MonitorList
        let monitor_list_msg = self.expect_message_type(ServerMessageType::MonitorList).await?;
        if let ServerMessage::MonitorList(monitor_list) = monitor_list_msg {
            info!("Received monitor list with {} monitors", monitor_list.monitors.len());
        }

        self.connected = true;

        Ok(())
    }

    /// Get available monitors
    pub async fn get_monitors(&mut self) -> Result<Vec<remote_desktop_core::protocol::MonitorInfo>> {
        if !self.connected {
            return Err(anyhow::anyhow!("Not connected to server"));
        }

        // Request monitor list by sending a dummy Hello
        let hello = ClientMessage::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "remote-desktop-client".to_string(),
            supported_codecs: vec!["h264".to_string()],
            max_width: 1920,
            max_height: 1080,
            max_fps: 30,
        });

        let encoded = encode_client_message(&hello)?;
        self.framed.send(encoded).await?;

        // Skip HelloAck and get MonitorList
        let _hello_ack = self.expect_message_type(ServerMessageType::HelloAck).await?;
        let monitor_list_msg = self.expect_message_type(ServerMessageType::MonitorList).await?;

        if let ServerMessage::MonitorList(monitor_list) = monitor_list_msg {
            Ok(monitor_list.monitors)
        } else {
            Err(anyhow::anyhow!("Expected MonitorList"))
        }
    }

    /// Select a monitor and start streaming
    pub async fn select_monitor(&mut self, monitor_name: &str) -> Result<StreamStart> {
        if !self.connected {
            return Err(anyhow::anyhow!("Not connected to server"));
        }

        info!("Selecting monitor: {}", monitor_name);

        let select = ClientMessage::SelectMonitor(SelectMonitor {
            monitor_name: monitor_name.to_string(),
            requested_width: None,
            requested_height: None,
            requested_fps: None,
        });

        let encoded = encode_client_message(&select)?;
        self.framed.send(encoded).await?;

        // Receive StreamStart
        let stream_start_msg = self.expect_message_type(ServerMessageType::StreamStart).await?;

        if let ServerMessage::StreamStart(stream_start) = stream_start_msg {
            info!("Stream started: {}x{}@{}fps, codec={}",
                stream_start.width, stream_start.height, stream_start.fps, stream_start.codec);

            self.selected_monitor = Some(monitor_name.to_string());
            self.stream_active = true;

            Ok(stream_start)
        } else {
            Err(anyhow::anyhow!("Expected StreamStart"))
        }
    }

    /// Send a key event
    pub async fn send_key(&mut self, key_code: u32, pressed: bool) -> Result<()> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        let state = if pressed { KeyState::Pressed } else { KeyState::Released };
        let event = ClientMessage::KeyEvent(KeyEvent {
            key_code,
            state,
        });

        let encoded = encode_client_message(&event)?;
        self.framed.send(encoded).await?;

        debug!("Sent key event: code={}, state={:?}", key_code, state);
        Ok(())
    }

    /// Send a pointer motion event
    pub async fn send_pointer_motion(&mut self, x: u16, y: u16) -> Result<()> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        let event = ClientMessage::PointerEvent(PointerEvent {
            event_type: PointerEventType::Motion,
            x: Some(x),
            y: Some(y),
            button: None,
            button_state: None,
            scroll_delta: None,
        });

        let encoded = encode_client_message(&event)?;
        self.framed.send(encoded).await?;

        debug!("Sent pointer motion: x={}, y={}", x, y);
        Ok(())
    }

    /// Send a pointer button event
    pub async fn send_pointer_button(&mut self, button: u8, pressed: bool) -> Result<()> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        let event = ClientMessage::PointerEvent(PointerEvent {
            event_type: PointerEventType::Button,
            x: None,
            y: None,
            button: Some(button),
            button_state: Some(state),
            scroll_delta: None,
        });

        let encoded = encode_client_message(&event)?;
        self.framed.send(encoded).await?;

        debug!("Sent pointer button: button={}, state={:?}", button, state);
        Ok(())
    }

    /// Send a scroll event
    pub async fn send_scroll(&mut self, delta: i16) -> Result<()> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        let event = ClientMessage::PointerEvent(PointerEvent {
            event_type: PointerEventType::Scroll,
            x: None,
            y: None,
            button: None,
            button_state: None,
            scroll_delta: Some(delta),
        });

        let encoded = encode_client_message(&event)?;
        self.framed.send(encoded).await?;

        debug!("Sent scroll: delta={}", delta);
        Ok(())
    }

    /// Request quality change
    pub async fn request_quality(&mut self, bitrate_kbps: u16) -> Result<()> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        let request = ClientMessage::QualityRequest(QualityRequest {
            target_bitrate_kbps: Some(bitrate_kbps),
            target_resolution: None,
            target_fps: None,
        });

        let encoded = encode_client_message(&request)?;
        self.framed.send(encoded).await?;

        info!("Requested quality change: {} Kbps", bitrate_kbps);
        Ok(())
    }

    /// Receive a video frame (with timeout)
    pub async fn receive_video_frame(&mut self) -> Result<Option<remote_desktop_core::protocol::VideoFrame>> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        match timeout(Duration::from_secs(5), self.framed.next()).await {
            Ok(Some(Ok(data))) => {
                match parse_server_message(data) {
                    Ok(msg) => {
                        match msg {
                            ServerMessage::VideoFrame(frame) => {
                                debug!("Received video frame: {} bytes, keyframe={:?}",
                                    frame.data.len(), frame.frame_type);
                                Ok(Some(frame))
                            }
                            ServerMessage::StreamStats(stats) => {
                                debug!("Stream stats: {} Kbps, {}x{}@{}fps, dropped={}",
                                    stats.current_bitrate_kbps, stats.current_width,
                                    stats.current_height, stats.current_fps,
                                    stats.dropped_frames);
                                Ok(None)
                            }
                            ServerMessage::Pong(pong) => {
                                debug!("Received Pong: latency={} us",
                                    pong.server_timestamp_us.saturating_sub(pong.timestamp_us));
                                Ok(None)
                            }
                            ServerMessage::ClipboardEvent(event) => {
                                debug!("Received clipboard event: {} bytes, type={}",
                                    event.data.len(), event.mime_type);
                                // Store for later retrieval
                                Ok(None)
                            }
                            ServerMessage::ErrorMessage(err) => {
                                error!("Server error: {:?} - {}", err.code, err.message);
                                Err(anyhow::anyhow!("Server error: {}", err.message))
                            }
                            _ => {
                                debug!("Received unexpected message");
                                Ok(None)
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse message: {}", e);
                        Err(e.into())
                    }
                }
            }
            Ok(Some(Err(e))) => {
                error!("Connection error: {}", e);
                Err(e.into())
            }
            Ok(None) => {
                warn!("Connection closed by server");
                self.stream_active = false;
                Err(anyhow::anyhow!("Connection closed"))
            }
            Err(_) => {
                warn!("Timeout waiting for video frame");
                Ok(None)
            }
        }
    }

    /// Receive any message from the server (with timeout)
    pub async fn receive_message(&mut self) -> Result<Option<ReceivedMessage>> {
        if !self.stream_active {
            return Err(anyhow::anyhow!("No active stream"));
        }

        match timeout(Duration::from_secs(5), self.framed.next()).await {
            Ok(Some(Ok(data))) => {
                match parse_server_message(data) {
                    Ok(msg) => {
                        match msg {
                            ServerMessage::VideoFrame(frame) => {
                                debug!("Received video frame: {} bytes, keyframe={:?}",
                                    frame.data.len(), frame.frame_type);
                                Ok(Some(ReceivedMessage::VideoFrame(frame)))
                            }
                            ServerMessage::StreamStats(stats) => {
                                debug!("Stream stats: {} Kbps", stats.current_bitrate_kbps);
                                Ok(Some(ReceivedMessage::StreamStats(stats)))
                            }
                            ServerMessage::Pong(pong) => {
                                debug!("Received Pong");
                                Ok(Some(ReceivedMessage::Pong(pong)))
                            }
                            ServerMessage::ClipboardEvent(event) => {
                                debug!("Received clipboard event: {} bytes, type={}",
                                    event.data.len(), event.mime_type);
                                Ok(Some(ReceivedMessage::ClipboardEvent(event)))
                            }
                            ServerMessage::ErrorMessage(err) => {
                                error!("Server error: {:?} - {}", err.code, err.message);
                                Err(anyhow::anyhow!("Server error: {}", err.message))
                            }
                            _ => {
                                debug!("Received unexpected message");
                                Ok(None)
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse message: {}", e);
                        Err(e.into())
                    }
                }
            }
            Ok(Some(Err(e))) => {
                error!("Connection error: {}", e);
                Err(e.into())
            }
            Ok(None) => {
                warn!("Connection closed by server");
                self.stream_active = false;
                Err(anyhow::anyhow!("Connection closed"))
            }
            Err(_) => {
                // Timeout - no message available
                Ok(None)
            }
        }
    }

    /// Expect a specific message type, skipping over async messages like ClipboardEvent
    async fn expect_message_type(&mut self, expected_type: ServerMessageType) -> Result<ServerMessage> {
        loop {
            match timeout(Duration::from_secs(5), self.framed.next()).await {
                Ok(Some(Ok(data))) => {
                    match parse_server_message(data) {
                        Ok(msg) => {
                            // Check if it's the expected type
                            let msg_type = match &msg {
                                ServerMessage::HelloAck(_) => ServerMessageType::HelloAck,
                                ServerMessage::MonitorList(_) => ServerMessageType::MonitorList,
                                ServerMessage::StreamStart(_) => ServerMessageType::StreamStart,
                                ServerMessage::VideoFrame(_) => ServerMessageType::VideoFrame,
                                ServerMessage::AudioData(_) => ServerMessageType::AudioData,
                                ServerMessage::Pong(_) => ServerMessageType::Pong,
                                ServerMessage::StreamStats(_) => ServerMessageType::StreamStats,
                                ServerMessage::ErrorMessage(_) => ServerMessageType::ProtocolError,
                                ServerMessage::ClipboardEvent(_) => ServerMessageType::ClipboardEvent,
                            };

                            if msg_type == expected_type {
                                return Ok(msg);
                            }

                            // Skip async messages that can arrive at any time
                            match msg_type {
                                ServerMessageType::ClipboardEvent |
                                ServerMessageType::StreamStats |
                                ServerMessageType::Pong => {
                                    debug!("Skipping async message {:?} while waiting for {:?}", msg_type, expected_type);
                                    continue;
                                }
                                _ => {
                                    return Err(anyhow::anyhow!("Expected {:?}, got {:?}", expected_type, msg_type));
                                }
                            }
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(Some(Err(e))) => return Err(e.into()),
                Ok(None) => return Err(anyhow::anyhow!("Connection closed")),
                Err(_) => return Err(anyhow::anyhow!("Timeout waiting for message")),
            }
        }
    }

    /// Pause the stream
    pub async fn pause(&mut self) -> Result<()> {
        if !self.stream_active {
            return Ok(());
        }

        let msg = ClientMessage::PauseStream;
        let encoded = encode_client_message(&msg)?;
        self.framed.send(encoded).await?;

        info!("Stream paused");
        Ok(())
    }

    /// Resume the stream
    pub async fn resume(&mut self) -> Result<()> {
        if !self.stream_active {
            return Ok(());
        }

        let msg = ClientMessage::ResumeStream;
        let encoded = encode_client_message(&msg)?;
        self.framed.send(encoded).await?;

        info!("Stream resumed");
        Ok(())
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Check if stream is active
    pub fn is_streaming(&self) -> bool {
        self.stream_active
    }

    /// Get selected monitor
    pub fn selected_monitor(&self) -> Option<&str> {
        self.selected_monitor.as_deref()
    }

    /// Send a generic client message (for input events from VideoPlayer)
    pub async fn send_message(&mut self, msg: ClientMessage) -> Result<()> {
        let encoded = encode_client_message(&msg)?;
        self.framed.send(encoded).await?;
        Ok(())
    }
}

/// Video player that decodes and displays frames, and captures input
#[allow(dead_code)]
pub struct VideoPlayer {
    decoder: VideoDecoder,
    display: Display,
    clipboard: Option<ClipboardHandler>,
    width: u32,
    height: u32,
    fps: u8,
    frame_count: u64,
    start_time: Instant,
    keyframe_received: bool,
    last_clipboard_check: Instant,
}

impl VideoPlayer {
    /// Create a new video player with the given stream dimensions
    pub fn new(width: u16, height: u16, fps: u8) -> Result<Self> {
        let width = width as u32;
        let height = height as u32;

        // Initialize decoder
        let mut decoder = VideoDecoder::new()?;
        decoder.set_dimensions(width, height);

        // Initialize display
        let display = Display::new(width, height, "Remote Desktop")?;

        // Initialize clipboard using display's video subsystem
        let clipboard = match ClipboardHandler::new(display.video_subsystem()) {
            Ok(handler) => {
                info!("Clipboard handler initialized");
                Some(handler)
            }
            Err(e) => {
                warn!("Failed to initialize clipboard handler: {}", e);
                None
            }
        };

        info!("VideoPlayer initialized: {}x{}@{}fps", width, height, fps);

        Ok(Self {
            decoder,
            display,
            clipboard,
            width,
            height,
            fps,
            frame_count: 0,
            start_time: Instant::now(),
            keyframe_received: false,
            last_clipboard_check: Instant::now(),
        })
    }

    /// Process a video frame: decode, render, and capture input
    ///
    /// Returns a vector of input/clipboard messages to send back to the server
    pub fn process_frame(&mut self, frame: &remote_desktop_core::protocol::VideoFrame) -> Result<Vec<ClientMessage>> {
        // Wait for a keyframe before starting to decode
        if !self.keyframe_received {
            if matches!(frame.frame_type, FrameType::IFrame) {
                self.keyframe_received = true;
                info!("Received first keyframe, starting decode");
            } else {
                debug!("Skipping non-keyframe (waiting for keyframe)");
                return Ok(Vec::new());
            }
        }

        // Decode the H.264 frame
        let is_keyframe = matches!(frame.frame_type, FrameType::IFrame);
        let decoded = self.decoder.decode(&frame.data, is_keyframe)?;

        // Render if we got a decoded frame
        if let Some(decoded_frame) = decoded {
            self.display.render_frame(&decoded_frame)?;
        }

        // Poll for input events and convert to protocol messages
        let events = self.display.poll_events();
        let mut messages: Vec<ClientMessage> = events
            .iter()
            .filter_map(|event| InputHandler::process_event(event, self.width, self.height))
            .collect();

        // Check for clipboard changes (every 500ms)
        if self.last_clipboard_check.elapsed() >= Duration::from_millis(500) {
            self.last_clipboard_check = Instant::now();

            if let Some(clipboard) = self.clipboard.as_mut() {
                if clipboard.has_changed() {
                    if let Ok(Some((mime_type, data))) = clipboard.get_content() {
                        debug!("Local clipboard changed, sending to server: {} bytes", data.len());
                        messages.push(ClientMessage::ClipboardEvent(ClipboardEvent {
                            mime_type,
                            data,
                        }));
                    }
                }
            }
        }

        // Update stats
        self.frame_count += 1;

        // Log stats every 30 frames
        if self.frame_count % 30 == 0 {
            let elapsed = self.start_time.elapsed().as_secs_f64();
            let actual_fps = if elapsed > 0.0 {
                self.frame_count as f64 / elapsed
            } else {
                0.0
            };
            info!("Stats: {} frames, {:.1} fps, last frame: {} bytes, keyframe={}, input_events={}",
                self.frame_count, actual_fps, frame.data.len(),
                is_keyframe, messages.len());
        }

        Ok(messages)
    }

    /// Handle an incoming clipboard event from the server
    pub fn handle_clipboard(&mut self, event: &ClipboardEvent) -> Result<()> {
        if let Some(clipboard) = self.clipboard.as_mut() {
            clipboard.set_content(&event.mime_type, &event.data)?;
            info!("Clipboard updated from server: {} bytes, type={}", event.data.len(), event.mime_type);
        }
        Ok(())
    }

    /// Check if the display is still running (window not closed)
    pub fn is_running(&self) -> bool {
        self.display.is_running()
    }

    /// Get current FPS
    pub fn fps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.frame_count as f64 / elapsed
        } else {
            0.0
        }
    }

    /// Get frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}
