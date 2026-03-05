//! Remote Desktop Protocol Implementation
//!
//! Custom binary protocol for low-latency remote desktop over Tailscale

use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{Decoder, Encoder};

// Protocol version
pub const PROTOCOL_VERSION: u16 = 1;

// Maximum frame size (16MB)
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Supported MIME types for clipboard synchronization
pub const SUPPORTED_CLIPBOARD_TYPES: &[&str] = &[
    "text/plain",
    "text/html",
    "image/png",
];

/// Message types (client → server)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClientMessageType {
    Hello = 0x01,
    SelectMonitor = 0x02,
    KeyEvent = 0x03,
    PointerEvent = 0x04,
    ClipboardEvent = 0x05,
    QualityRequest = 0x06,
    Ping = 0x07,
    PauseStream = 0x08,
    ResumeStream = 0x09,
}

impl TryFrom<u8> for ClientMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(ClientMessageType::Hello),
            0x02 => Ok(ClientMessageType::SelectMonitor),
            0x03 => Ok(ClientMessageType::KeyEvent),
            0x04 => Ok(ClientMessageType::PointerEvent),
            0x05 => Ok(ClientMessageType::ClipboardEvent),
            0x06 => Ok(ClientMessageType::QualityRequest),
            0x07 => Ok(ClientMessageType::Ping),
            0x08 => Ok(ClientMessageType::PauseStream),
            0x09 => Ok(ClientMessageType::ResumeStream),
            _ => Err(ProtocolError::UnknownMessageType(value)),
        }
    }
}

/// Message types (server → client)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ServerMessageType {
    HelloAck = 0x10,
    MonitorList = 0x11,
    StreamStart = 0x12,
    VideoFrame = 0x13,
    AudioData = 0x14,
    Pong = 0x15,
    StreamStats = 0x16,
    ProtocolError = 0x17,
    ClipboardEvent = 0x18,
}

impl TryFrom<u8> for ServerMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x10 => Ok(ServerMessageType::HelloAck),
            0x11 => Ok(ServerMessageType::MonitorList),
            0x12 => Ok(ServerMessageType::StreamStart),
            0x13 => Ok(ServerMessageType::VideoFrame),
            0x14 => Ok(ServerMessageType::AudioData),
            0x15 => Ok(ServerMessageType::Pong),
            0x16 => Ok(ServerMessageType::StreamStats),
            0x17 => Ok(ServerMessageType::ProtocolError),
            0x18 => Ok(ServerMessageType::ClipboardEvent),
            _ => Err(ProtocolError::UnknownMessageType(value)),
        }
    }
}

/// Protocol messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello(Hello),
    SelectMonitor(SelectMonitor),
    KeyEvent(KeyEvent),
    PointerEvent(PointerEvent),
    ClipboardEvent(ClipboardEvent),
    QualityRequest(QualityRequest),
    Ping(Ping),
    PauseStream,
    ResumeStream,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    HelloAck(HelloAck),
    MonitorList(MonitorList),
    StreamStart(StreamStart),
    VideoFrame(VideoFrame),
    AudioData(AudioData),
    Pong(Pong),
    StreamStats(StreamStats),
    ErrorMessage(Error),
    ClipboardEvent(ClipboardEvent),
}

// Message payloads

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u16,
    pub client_name: String,
    pub supported_codecs: Vec<String>,
    pub max_width: u16,
    pub max_height: u16,
    pub max_fps: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: u16,
    pub server_name: String,
    pub selected_codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorList {
    pub monitors: Vec<MonitorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub refresh_rate: u8,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectMonitor {
    pub monitor_name: String,
    pub requested_width: Option<u16>,
    pub requested_height: Option<u16>,
    pub requested_fps: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStart {
    pub width: u16,
    pub height: u16,
    pub fps: u8,
    pub codec: String,
    pub profile: u8,
    pub level: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFrame {
    pub frame_type: FrameType,
    pub timestamp_us: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameType {
    PFrame = 0,
    IFrame = 1,
}

impl From<u8> for FrameType {
    fn from(value: u8) -> Self {
        match value {
            0 => FrameType::PFrame,
            1 => FrameType::IFrame,
            _ => FrameType::PFrame,
        }
    }
}

impl From<FrameType> for u8 {
    fn from(frame_type: FrameType) -> Self {
        frame_type as u8
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioData {
    pub timestamp_us: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key_code: u32,
    pub state: KeyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Released = 0,
    Pressed = 1,
}

impl From<u8> for KeyState {
    fn from(value: u8) -> Self {
        match value {
            0 => KeyState::Released,
            1 => KeyState::Pressed,
            _ => KeyState::Released,
        }
    }
}

impl From<KeyState> for u8 {
    fn from(state: KeyState) -> Self {
        state as u8
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointerEvent {
    pub event_type: PointerEventType,
    pub x: Option<u16>,
    pub y: Option<u16>,
    pub button: Option<u8>,
    pub button_state: Option<ButtonState>,
    pub scroll_delta: Option<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerEventType {
    Motion = 0,
    Button = 1,
    Scroll = 2,
}

impl From<u8> for PointerEventType {
    fn from(value: u8) -> Self {
        match value {
            0 => PointerEventType::Motion,
            1 => PointerEventType::Button,
            2 => PointerEventType::Scroll,
            _ => PointerEventType::Motion,
        }
    }
}

impl From<PointerEventType> for u8 {
    fn from(event_type: PointerEventType) -> Self {
        event_type as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonState {
    Released = 0,
    Pressed = 1,
}

impl From<u8> for ButtonState {
    fn from(value: u8) -> Self {
        match value {
            0 => ButtonState::Released,
            1 => ButtonState::Pressed,
            _ => ButtonState::Released,
        }
    }
}

impl From<ButtonState> for u8 {
    fn from(state: ButtonState) -> Self {
        state as u8
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardEvent {
    /// MIME type of the clipboard data (e.g., "text/plain", "text/html", "image/png")
    pub mime_type: String,
    /// Raw clipboard data
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRequest {
    pub target_bitrate_kbps: Option<u16>,
    pub target_resolution: Option<(u16, u16)>,
    pub target_fps: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ping {
    pub timestamp_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub timestamp_us: u64,
    pub server_timestamp_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub current_bitrate_kbps: u16,
    pub current_width: u16,
    pub current_height: u16,
    pub current_fps: u8,
    pub dropped_frames: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    MonitorNotFound = 1,
    UnsupportedCodec = 2,
    InvalidResolution = 3,
    StreamFailed = 4,
    RateLimited = 5,
}

impl TryFrom<u8> for ErrorCode {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ErrorCode::MonitorNotFound),
            2 => Ok(ErrorCode::UnsupportedCodec),
            3 => Ok(ErrorCode::InvalidResolution),
            4 => Ok(ErrorCode::StreamFailed),
            5 => Ok(ErrorCode::RateLimited),
            _ => Err(ProtocolError::UnknownErrorCode(value)),
        }
    }
}

/// Protocol errors
#[derive(thiserror::Error, Debug)]
pub enum ProtocolError {
    #[error("Unknown message type: {0}")]
    UnknownMessageType(u8),

    #[error("Unknown error code: {0}")]
    UnknownErrorCode(u8),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Frame too large: {0} bytes")]
    FrameTooLarge(usize),
}

/// Codec for protocol messages
pub struct ProtocolCodec;

impl Decoder for ProtocolCodec {
    type Item = BytesMut;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Read length prefix (u32 big-endian)
        if src.len() < 4 {
            return Ok(None);
        }

        let length = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;

        if length > MAX_FRAME_SIZE {
            return Err(ProtocolError::FrameTooLarge(length));
        }

        if src.len() < 4 + length {
            return Ok(None);
        }

        src.advance(4);
        Ok(Some(src.split_to(length)))
    }
}

impl Encoder<BytesMut> for ProtocolCodec {
    type Error = ProtocolError;

    fn encode(&mut self, item: BytesMut, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let length = item.len() as u32;
        dst.reserve(4 + item.len());
        dst.put_slice(&length.to_be_bytes());
        dst.extend_from_slice(&item);
        Ok(())
    }
}

/// Parse message type and payload
pub fn parse_client_message(mut data: BytesMut) -> Result<ClientMessage, ProtocolError> {
    if data.is_empty() {
        return Err(ProtocolError::Serialization("Empty message".to_string()));
    }

    let msg_type = data.get_u8();
    let msg_type = ClientMessageType::try_from(msg_type)?;

    // Deserialize payload based on message type
    let payload: Vec<u8> = data.to_vec();

    let message = match msg_type {
        ClientMessageType::Hello => {
            let hello: Hello = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::Hello(hello)
        }
        ClientMessageType::SelectMonitor => {
            let sel: SelectMonitor = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::SelectMonitor(sel)
        }
        ClientMessageType::KeyEvent => {
            let event: KeyEvent = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::KeyEvent(event)
        }
        ClientMessageType::PointerEvent => {
            let event: PointerEvent = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::PointerEvent(event)
        }
        ClientMessageType::ClipboardEvent => {
            let event: ClipboardEvent = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::ClipboardEvent(event)
        }
        ClientMessageType::QualityRequest => {
            let req: QualityRequest = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::QualityRequest(req)
        }
        ClientMessageType::Ping => {
            let ping: Ping = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ClientMessage::Ping(ping)
        }
        ClientMessageType::PauseStream => ClientMessage::PauseStream,
        ClientMessageType::ResumeStream => ClientMessage::ResumeStream,
    };

    Ok(message)
}

/// Parse server message
pub fn parse_server_message(mut data: BytesMut) -> Result<ServerMessage, ProtocolError> {
    if data.is_empty() {
        return Err(ProtocolError::Serialization("Empty message".to_string()));
    }

    let msg_type = data.get_u8();
    let msg_type = ServerMessageType::try_from(msg_type)?;

    let payload: Vec<u8> = data.to_vec();

    let message = match msg_type {
        ServerMessageType::HelloAck => {
            let ack: HelloAck = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::HelloAck(ack)
        }
        ServerMessageType::MonitorList => {
            let list: MonitorList = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::MonitorList(list)
        }
        ServerMessageType::StreamStart => {
            let start: StreamStart = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::StreamStart(start)
        }
        ServerMessageType::VideoFrame => {
            let frame: VideoFrame = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::VideoFrame(frame)
        }
        ServerMessageType::AudioData => {
            let audio: AudioData = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::AudioData(audio)
        }
        ServerMessageType::Pong => {
            let pong: Pong = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::Pong(pong)
        }
        ServerMessageType::StreamStats => {
            let stats: StreamStats = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::StreamStats(stats)
        }
        ServerMessageType::ProtocolError => {
            let err: Error = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::ErrorMessage(err)
        }
        ServerMessageType::ClipboardEvent => {
            let event: ClipboardEvent = bincode::deserialize(&payload)
                .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
            ServerMessage::ClipboardEvent(event)
        }
    };

    Ok(message)
}

/// Encode client message
pub fn encode_client_message(message: &ClientMessage) -> Result<BytesMut, ProtocolError> {
    let mut buffer = BytesMut::new();

    let (msg_type, payload): (u8, Vec<u8>) = match message {
        ClientMessage::Hello(h) => (
            ClientMessageType::Hello as u8,
            bincode::serialize(h).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::SelectMonitor(s) => (
            ClientMessageType::SelectMonitor as u8,
            bincode::serialize(s).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::KeyEvent(e) => (
            ClientMessageType::KeyEvent as u8,
            bincode::serialize(e).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::PointerEvent(e) => (
            ClientMessageType::PointerEvent as u8,
            bincode::serialize(e).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::ClipboardEvent(e) => (
            ClientMessageType::ClipboardEvent as u8,
            bincode::serialize(e).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::QualityRequest(r) => (
            ClientMessageType::QualityRequest as u8,
            bincode::serialize(r).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::Ping(p) => (
            ClientMessageType::Ping as u8,
            bincode::serialize(p).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ClientMessage::PauseStream => (ClientMessageType::PauseStream as u8, vec![]),
        ClientMessage::ResumeStream => (ClientMessageType::ResumeStream as u8, vec![]),
    };

    buffer.put_u8(msg_type);
    buffer.extend_from_slice(&payload);

    Ok(buffer)
}

/// Encode server message
pub fn encode_server_message(message: &ServerMessage) -> Result<BytesMut, ProtocolError> {
    let mut buffer = BytesMut::new();

    let (msg_type, payload): (u8, Vec<u8>) = match message {
        ServerMessage::HelloAck(a) => (
            ServerMessageType::HelloAck as u8,
            bincode::serialize(a).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::MonitorList(l) => (
            ServerMessageType::MonitorList as u8,
            bincode::serialize(l).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::StreamStart(s) => (
            ServerMessageType::StreamStart as u8,
            bincode::serialize(s).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::VideoFrame(f) => (
            ServerMessageType::VideoFrame as u8,
            bincode::serialize(f).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::AudioData(a) => (
            ServerMessageType::AudioData as u8,
            bincode::serialize(a).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::Pong(p) => (
            ServerMessageType::Pong as u8,
            bincode::serialize(p).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::StreamStats(s) => (
            ServerMessageType::StreamStats as u8,
            bincode::serialize(s).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::ErrorMessage(e) => (
            ServerMessageType::ProtocolError as u8,
            bincode::serialize(e).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
        ServerMessage::ClipboardEvent(c) => (
            ServerMessageType::ClipboardEvent as u8,
            bincode::serialize(c).map_err(|e| ProtocolError::Serialization(e.to_string()))?,
        ),
    };

    buffer.put_u8(msg_type);
    buffer.extend_from_slice(&payload);

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hello_encoding() {
        let hello = Hello {
            protocol_version: 1,
            client_name: "TestClient".to_string(),
            supported_codecs: vec!["h264".to_string()],
            max_width: 1920,
            max_height: 1080,
            max_fps: 30,
        };

        let message = ClientMessage::Hello(hello);
        let encoded = encode_client_message(&message).unwrap();
        let decoded = parse_client_message(encoded).unwrap();

        match decoded {
            ClientMessage::Hello(h) => {
                assert_eq!(h.protocol_version, 1);
                assert_eq!(h.client_name, "TestClient");
                assert_eq!(h.max_width, 1920);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_video_frame_encoding() {
        let frame = VideoFrame {
            frame_type: FrameType::IFrame,
            timestamp_us: 12345678,
            data: vec![1, 2, 3, 4, 5],
        };

        let message = ServerMessage::VideoFrame(frame);
        let encoded = encode_server_message(&message).unwrap();
        let decoded = parse_server_message(encoded).unwrap();

        match decoded {
            ServerMessage::VideoFrame(f) => {
                assert_eq!(f.frame_type, FrameType::IFrame);
                assert_eq!(f.timestamp_us, 12345678);
                assert_eq!(f.data, vec![1, 2, 3, 4, 5]);
            }
            _ => panic!("Wrong message type"),
        }
    }
}
