//! PipeWire screen capture integration
//!
//! Connects to PipeWire screencast streams from xdg-desktop-portal
//! and provides RGB24 video frames for encoding.

use pipewire as pw;
use pw::spa;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

/// A captured video frame from PipeWire
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Frame data (RGB24 format)
    pub data: Vec<u8>,

    /// Width in pixels
    pub width: u32,

    /// Height in pixels
    pub height: u32,

    /// Timestamp in nanoseconds
    pub timestamp: u64,
}

/// PipeWire screen capture stream
pub struct VideoStream {
    monitor_name: String,
    node_id: Option<u32>,
    frame_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
    running: Arc<Mutex<bool>>,
    width: u32,
    height: u32,
}

impl VideoStream {
    /// Create a new screen capture stream for the given monitor
    pub async fn new(monitor_name: &str) -> super::Result<Self> {
        debug!("Creating PipeWire stream for monitor: {}", monitor_name);

        Ok(Self {
            monitor_name: monitor_name.to_string(),
            node_id: None,
            frame_queue: Arc::new(Mutex::new(VecDeque::with_capacity(5))),
            running: Arc::new(Mutex::new(false)),
            width: 1920,
            height: 1080,
        })
    }

    /// Create a stream for an existing PipeWire node (from portal)
    pub async fn from_node_id(node_id: u32, width: u32, height: u32) -> super::Result<Self> {
        debug!("Creating PipeWire stream from node ID: {}", node_id);

        let frame_queue = Arc::new(Mutex::new(VecDeque::with_capacity(5)));
        let running = Arc::new(Mutex::new(true));

        let frame_queue_clone = frame_queue.clone();
        let running_clone = running.clone();

        // Spawn PipeWire thread
        let _handle = std::thread::spawn(move || {
            Self::run_pipewire_stream(node_id, width, height, frame_queue_clone, running_clone)
        });

        // Wait a bit for the stream to start
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        info!("Created PipeWire stream for node {} ({}x{})", node_id, width, height);

        Ok(Self {
            monitor_name: format!("node-{}", node_id),
            node_id: Some(node_id),
            frame_queue,
            running,
            width,
            height,
        })
    }

    /// Run the PipeWire stream in a separate thread
    fn run_pipewire_stream(
        node_id: u32,
        width: u32,
        height: u32,
        frame_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
        running: Arc<Mutex<bool>>,
    ) -> super::Result<()> {
        pw::init();

        info!("PipeWire thread started for node {}", node_id);

        let mainloop = pw::main_loop::MainLoop::new(None)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to create mainloop: {:?}", e)))?;

        let context = pw::context::Context::new(&mainloop)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to create context: {:?}", e)))?;

        let core = context.connect(None)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to connect: {:?}", e)))?;

        // User data to store video format and state
        #[derive(Clone)]
        struct StreamData {
            width: u32,
            height: u32,
            frame_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
            running: Arc<Mutex<bool>>,
            node_id: u32,
        }

        let data = StreamData {
            width,
            height,
            frame_queue: frame_queue.clone(),
            running: running.clone(),
            node_id,
        };

        let stream = pw::stream::Stream::new(
            &core,
            "remote-desktop-capture",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        ).map_err(|e| super::CoreError::PipeWire(format!("Failed to create stream: {:?}", e)))?;

        let _listener = stream
            .add_local_listener_with_user_data(data.clone())
            .state_changed(|_, user_data, old, new| {
                debug!("Stream state: {:?} -> {:?}", old, new);
                if new == pw::stream::StreamState::Unconnected {
                    warn!("Stream disconnected from node {}", user_data.node_id);
                    *user_data.running.lock().unwrap() = false;
                }
            })
            .param_changed(|_, user_data, id, param| {
                let Some(param) = param else {
                    return;
                };
                if id != pw::spa::param::ParamType::Format.as_raw() {
                    return;
                }

                let (media_type, media_subtype) =
                    match pw::spa::param::format_utils::parse_format(param) {
                        Ok(v) => v,
                        Err(_) => return,
                    };

                if media_type != pw::spa::param::format::MediaType::Video
                    || media_subtype != pw::spa::param::format::MediaSubtype::Raw
                {
                    return;
                }

                debug!("Got video format parameter");
            })
            .process(|stream, user_data| {
                // Process incoming video frames
                while let Some(mut buffer) = stream.dequeue_buffer() {
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        continue;
                    }

                    let data = &mut datas[0];
                    let chunk = data.chunk();

                    if chunk.size() == 0 {
                        continue;
                    }

                    // Get the data as a slice - data() returns Option<&mut [u8]>
                    let Some(raw_slice) = data.data() else {
                        continue;
                    };

                    // Store the frame
                    let frame = VideoFrame {
                        data: raw_slice.to_vec(),
                        width: user_data.width,
                        height: user_data.height,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64,
                    };

                    let frame_len = frame.data.len();

                    let mut queue = user_data.frame_queue.lock().unwrap();
                    if queue.len() >= 5 {
                        queue.pop_front(); // Keep only recent frames
                    }
                    queue.push_back(frame);

                    debug!("Captured frame: {}x{}, {} bytes",
                        user_data.width, user_data.height, frame_len);
                }
            })
            .register()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to register listener: {:?}", e)))?;

        // Define supported formats
        let obj = pw::spa::pod::object!(
            pw::spa::utils::SpaTypes::ObjectParamFormat,
            pw::spa::param::ParamType::EnumFormat,
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaType,
                Id,
                pw::spa::param::format::MediaType::Video
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaSubtype,
                Id,
                pw::spa::param::format::MediaSubtype::Raw
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                pw::spa::param::video::VideoFormat::RGB,
                pw::spa::param::video::VideoFormat::RGB,
                pw::spa::param::video::VideoFormat::RGBA,
                pw::spa::param::video::VideoFormat::BGRx,
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                pw::spa::utils::Rectangle { width, height },
                pw::spa::utils::Rectangle { width: 1, height: 1 },
                pw::spa::utils::Rectangle { width: 4096, height: 4096 }
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoFramerate,
                Choice,
                Range,
                Fraction,
                pw::spa::utils::Fraction { num: 30, denom: 1 },
                pw::spa::utils::Fraction { num: 1, denom: 1 },
                pw::spa::utils::Fraction { num: 60, denom: 1 }
            ),
        );

        let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(obj),
        ).unwrap().0.into_inner();

        let mut params = [pw::spa::pod::Pod::from_bytes(&values).unwrap()];

        // Connect to the target node
        stream.connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        ).map_err(|e| super::CoreError::PipeWire(format!("Failed to connect stream: {:?}", e)))?;

        info!("Stream connected to node {}", node_id);

        // Run the main loop
        mainloop.run();

        Ok(())
    }

    /// Start the screen capture session
    pub fn start(&mut self, _fd: std::os::fd::OwnedFd, width: u32, height: u32) -> super::Result<()> {
        debug!("Starting screen capture: {}x{} for monitor {}",
            width, height, self.monitor_name);

        self.width = width;
        self.height = height;

        if self.node_id.is_some() {
            *self.running.lock().unwrap() = true;
            info!("Screen capture active (PipeWire connected)");
        } else {
            info!("Screen capture in stub mode (no PipeWire node ID)");
        }

        Ok(())
    }

    /// Capture a single frame
    pub fn capture_frame(&mut self) -> super::Result<Option<VideoFrame>> {
        if !self.is_active() {
            // Return stub frame when not connected
            return Ok(Some(VideoFrame {
                data: vec![0u8; (self.width * self.height * 3) as usize],
                width: self.width,
                height: self.height,
                timestamp: 0,
            }));
        }

        // Try to get a frame from the queue
        let mut queue = self.frame_queue.lock().unwrap();
        Ok(queue.pop_front()
            .or_else(|| {
                // No frame available - return stub with correct size
                Some(VideoFrame {
                    data: vec![0u8; (self.width * self.height * 3) as usize],
                    width: self.width,
                    height: self.height,
                    timestamp: 0,
                })
            }))
    }

    /// Stop the stream
    pub fn stop(&mut self) -> super::Result<()> {
        info!("Screen capture stream stopped for {}", self.monitor_name);
        *self.running.lock().unwrap() = false;
        Ok(())
    }

    /// Get the current stream state
    pub fn is_active(&self) -> bool {
        *self.running.lock().unwrap() && self.node_id.is_some()
    }

    /// Get the node ID (if from portal)
    pub fn node_id(&self) -> u32 {
        self.node_id.unwrap_or(0)
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
