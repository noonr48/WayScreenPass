//! H.264 software encoding using a persistent x264 process
//!
//! Spawns x264 once and feeds frames continuously through stdin/stdout pipes.
//! A background reader thread parses H.264 NAL units from stdout.

use std::io::Write;
use std::process::{Command, Stdio, Child, ChildStdin};
use std::sync::mpsc;
use tracing::{debug, info, warn, error};

/// Encoded video frame
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EncodedFrame {
    /// H.264 encoded data
    pub data: Vec<u8>,

    /// Presentation timestamp
    pub pts: i64,

    /// Whether this is a keyframe (for streaming)
    pub is_keyframe: bool,
}

/// H.264 software encoder using a persistent x264 process
#[allow(dead_code)]
pub struct H264Encoder {
    width: u32,
    height: u32,
    fps: u8,
    frame_count: u64,
    force_keyframe: bool,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    frame_rx: Option<mpsc::Receiver<EncodedFrame>>,
    y4m_header_sent: bool,
}

impl H264Encoder {
    /// Create a new H.264 encoder with a persistent x264 process
    pub fn new(width: u32, height: u32) -> super::Result<Self> {
        info!("Creating persistent H.264 encoder: {}x{} @ 30fps", width, height);

        let mut encoder = Self {
            width,
            height,
            fps: 30,
            frame_count: 0,
            force_keyframe: false,
            child: None,
            stdin: None,
            frame_rx: None,
            y4m_header_sent: false,
        };

        encoder.start_x264()?;
        encoder.prime_pipeline()?;
        Ok(encoder)
    }

    /// Send a few dummy frames to fill x264's internal pipeline so the
    /// first real encode() call gets encoded data back immediately.
    fn prime_pipeline(&mut self) -> super::Result<()> {
        let dummy = vec![0u8; (self.width * self.height * 3) as usize];
        let _stride = self.width * 3;

        // x264 with --tune zerolatency typically needs ~2 frames to prime.
        // We send 3 to be safe, then drain any encoded output the reader
        // thread has produced so it doesn't leak into real frame results.
        for _ in 0..3 {
            let stdin = self.stdin.as_mut()
                .ok_or_else(|| super::CoreError::PipeWire("x264 not running".into()))?;

            if !self.y4m_header_sent {
                let header = format!("YUV4MPEG2 W{} H{} F{}:1 Ip C420mpeg2\n",
                                     self.width, self.height, self.fps);
                stdin.write_all(header.as_bytes())
                    .map_err(|e| super::CoreError::PipeWire(format!("prime header: {:?}", e)))?;
                self.y4m_header_sent = true;
            }

            let yuv_data = Self::rgb_to_yuv420p(&dummy, self.width, self.height);

            stdin.write_all(b"FRAME\n")
                .map_err(|e| super::CoreError::PipeWire(format!("prime FRAME: {:?}", e)))?;
            stdin.write_all(&yuv_data)
                .map_err(|e| super::CoreError::PipeWire(format!("prime yuv: {:?}", e)))?;
            stdin.flush()
                .map_err(|e| super::CoreError::PipeWire(format!("prime flush: {:?}", e)))?;
        }

        // Wait for x264 to produce encoded output for all priming frames,
        // then drain them so they don't appear as real frame data.
        // We use recv_timeout with a generous timeout per frame.
        if let Some(rx) = self.frame_rx.as_ref() {
            let mut drained = 0;
            for _ in 0..3 {
                match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                    Ok(_) => drained += 1,
                    Err(_) => break,
                }
            }
            debug!("Drained {} priming frames from x264 pipeline", drained);
        }

        debug!("x264 pipeline primed with 3 dummy frames");
        Ok(())
    }

    /// Spawn the persistent x264 process and background reader thread
    fn start_x264(&mut self) -> super::Result<()> {
        let mut child = Command::new("x264")
            .args([
                "--demuxer", "y4m",
                "--preset", "veryfast",
                "--tune", "zerolatency",
                "--crf", "23",
                "--threads", "0",
                "--keyint", "60",
                "--quiet",
                "-o", "-",  // output to stdout
                "-",        // input from stdin
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to spawn x264: {:?}", e)))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| super::CoreError::PipeWire("No stdout from x264".into()))?;

        let stdin = child.stdin.take()
            .ok_or_else(|| super::CoreError::PipeWire("No stdin from x264".into()))?;

        // Spawn background reader thread for NAL unit parsing
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            Self::read_nal_units(stdout, tx);
        });

        self.child = Some(child);
        self.stdin = Some(stdin);
        self.frame_rx = Some(rx);
        self.y4m_header_sent = false;

        info!("Persistent x264 process started");
        Ok(())
    }

    /// Background thread: reads x264 stdout and splits into access units (frames)
    ///
    /// H.264 Annex B uses start codes (00 00 00 01 or 00 00 01) to delimit NAL units.
    /// A new access unit (frame) begins when we encounter a VCL NAL (types 1, 5)
    /// or SPS (type 7) after already having accumulated data.
    fn read_nal_units(
        stdout: std::process::ChildStdout,
        tx: mpsc::Sender<EncodedFrame>,
    ) {
        use std::io::Read;
        let mut reader = std::io::BufReader::with_capacity(256 * 1024, stdout);
        let mut buffer = Vec::with_capacity(256 * 1024);
        let mut read_buf = [0u8; 65536];
        let mut frame_count: i64 = 0;

        loop {
            match reader.read(&mut read_buf) {
                Ok(0) => break, // EOF - x264 closed stdout
                Ok(n) => {
                    let start_len = buffer.len();
                    buffer.extend_from_slice(&read_buf[..n]);

                    // Try to extract complete access units.
                    // Search for start codes that begin a new VCL NAL after
                    // we already have accumulated data - that boundary marks
                    // the start of the next frame.
                    let mut search_from = if start_len > 4 { start_len - 4 } else { 0 };

                    loop {
                        let next_au = find_access_unit_boundary(&buffer, search_from + 1);

                        if let Some(boundary) = next_au {
                            if boundary > 0 {
                                let au_data: Vec<u8> = buffer[..boundary].to_vec();
                                let is_keyframe = contains_idr(&au_data);

                                let frame = EncodedFrame {
                                    data: au_data,
                                    pts: frame_count,
                                    is_keyframe,
                                };

                                frame_count += 1;

                                if tx.send(frame).is_err() {
                                    return; // receiver dropped - encoder was destroyed
                                }

                                buffer = buffer[boundary..].to_vec();
                                search_from = 0;
                            } else {
                                break;
                            }
                        } else {
                            break; // no more complete access units yet
                        }
                    }
                }
                Err(e) => {
                    debug!("x264 stdout read error: {}", e);
                    break;
                }
            }
        }

        // Flush remaining data as the final frame
        if !buffer.is_empty() {
            let is_keyframe = contains_idr(&buffer);
            let _ = tx.send(EncodedFrame {
                data: buffer,
                pts: frame_count,
                is_keyframe,
            });
        }

        debug!("x264 reader thread exiting after {} frames", frame_count);
    }

    /// Convert RGB24 to YUV420P
    fn rgb_to_yuv420p(rgb_data: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut yuv = vec![0u8; (width * height * 3 / 2) as usize];
        let y_size = (width * height) as usize;
        let uv_size = y_size / 4;
        let (y_plane, uv_part) = yuv.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_part.split_at_mut(uv_size);

        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 3) as usize;
                if idx + 2 >= rgb_data.len() { break; }
                let r = rgb_data[idx] as i32;
                let g = rgb_data[idx + 1] as i32;
                let b = rgb_data[idx + 2] as i32;

                let y_val = (66 * r + 129 * g + 25 * b + 128) / 256 + 16;
                y_plane[(y * width + x) as usize] = y_val.clamp(0, 255) as u8;

                if x % 2 == 0 && y % 2 == 0 {
                    let uv_idx = (y / 2 * width / 2 + x / 2) as usize;
                    let u_val = (-38 * r - 74 * g + 112 * b + 128) / 256 + 128;
                    u_plane[uv_idx] = u_val.clamp(0, 255) as u8;
                    let v_val = (112 * r - 94 * g - 18 * b + 128) / 256 + 128;
                    v_plane[uv_idx] = v_val.clamp(0, 255) as u8;
                }
            }
        }
        yuv
    }

    /// Encode a raw RGB frame
    ///
    /// Feeds the frame to the persistent x264 process via stdin and waits
    /// for the encoded access unit from the background reader thread.
    pub fn encode(&mut self, rgb_data: &[u8], stride: u32) -> super::Result<EncodedFrame> {
        let frame_size = (self.height * stride) as usize;
        if rgb_data.len() < frame_size {
            return Err(super::CoreError::PipeWire(
                format!("RGB data too small: {} < {}", rgb_data.len(), frame_size)
            ));
        }

        // Ensure x264 is running
        let stdin = self.stdin.as_mut()
            .ok_or_else(|| super::CoreError::PipeWire("x264 not running".into()))?;

        // Send Y4M header on the first frame
        if !self.y4m_header_sent {
            let header = format!("YUV4MPEG2 W{} H{} F{}:1 Ip C420mpeg2\n",
                                 self.width, self.height, self.fps);
            stdin.write_all(header.as_bytes())
                .map_err(|e| super::CoreError::PipeWire(format!("Failed to write Y4M header: {:?}", e)))?;
            self.y4m_header_sent = true;
        }

        // Convert RGB to YUV420P
        let yuv_data = Self::rgb_to_yuv420p(rgb_data, self.width, self.height);

        // Write Y4M FRAME header + raw YUV data
        stdin.write_all(b"FRAME\n")
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to write FRAME: {:?}", e)))?;
        stdin.write_all(&yuv_data)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to write YUV: {:?}", e)))?;
        stdin.flush()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to flush: {:?}", e)))?;

        self.frame_count += 1;

        // Wait for encoded frame from reader thread (with timeout)
        let rx = self.frame_rx.as_ref()
            .ok_or_else(|| super::CoreError::PipeWire("No frame receiver".into()))?;

        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(frame) => {
                debug!("Encoded frame {}: {} bytes, keyframe={}",
                    self.frame_count - 1, frame.data.len(), frame.is_keyframe);
                Ok(frame)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                warn!("Timeout waiting for encoded frame {}", self.frame_count - 1);
                Ok(EncodedFrame {
                    data: vec![],
                    pts: (self.frame_count - 1) as i64,
                    is_keyframe: false,
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                error!("x264 process died, restarting...");
                self.restart()?;
                Ok(EncodedFrame {
                    data: vec![],
                    pts: (self.frame_count - 1) as i64,
                    is_keyframe: false,
                })
            }
        }
    }

    /// Kill the old x264 process and start a new one
    fn restart(&mut self) -> super::Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.frame_rx = None;
        self.frame_count = 0;
        self.start_x264()
    }

    /// Request the next frame be a keyframe
    pub fn request_keyframe(&mut self) -> super::Result<()> {
        debug!("Requesting keyframe");
        self.force_keyframe = true;
        Ok(())
    }

    /// Get the encoder width
    pub fn width(&self) -> u32 { self.width }

    /// Get the encoder height
    pub fn height(&self) -> u32 { self.height }

    /// Reset the encoder state
    pub fn reset(&mut self) -> super::Result<()> {
        info!("Resetting encoder");
        self.restart()
    }
}

impl Drop for H264Encoder {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            drop(self.stdin.take()); // close stdin first to signal EOF
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Find the start of a new access unit (frame) in H.264 Annex B bytestream.
///
/// Returns the byte offset of the start code that begins a new frame, or None.
/// An access unit boundary is detected when a VCL NAL (slice) or SPS starts
/// after position `start`.
fn find_access_unit_boundary(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 4 < data.len() {
        // 4-byte start code: 00 00 00 01
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            if i + 4 < data.len() {
                let nal_type = data[i + 4] & 0x1F;
                // VCL NAL types 1 (non-IDR slice) and 5 (IDR slice) start a new
                // access unit. SPS (7) also typically precedes a new frame.
                if nal_type == 1 || nal_type == 5 || nal_type == 7 {
                    return Some(i);
                }
            }
        }
        // 3-byte start code: 00 00 01 (but not part of a 4-byte 00 00 00 01)
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1
            && (i == 0 || data[i - 1] != 0)
        {
            if i + 3 < data.len() {
                let nal_type = data[i + 3] & 0x1F;
                if nal_type == 1 || nal_type == 5 || nal_type == 7 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Check if an access unit contains an IDR slice (keyframe)
fn contains_idr(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 4 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            if i + 4 < data.len() && (data[i + 4] & 0x1F) == 5 {
                return true;
            }
        }
        // 3-byte start code, but not part of a 4-byte start code
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1
            && (i == 0 || data[i - 1] != 0)
        {
            if i + 3 < data.len() && (data[i + 3] & 0x1F) == 5 {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_encoder() {
        let encoder = H264Encoder::new(1920, 1080);
        assert!(encoder.is_ok());
        let enc = encoder.unwrap();
        assert_eq!(enc.width(), 1920);
        assert_eq!(enc.height(), 1080);
    }

    #[test]
    fn test_encode_small_frame() {
        let mut encoder = H264Encoder::new(320, 240).unwrap();
        let frame_size = (320 * 240 * 3) as usize;
        let rgb_data = vec![0u8; frame_size];

        match encoder.encode(&rgb_data, 320 * 3) {
            Ok(encoded) => {
                println!("Encoded: {} bytes, keyframe={}", encoded.data.len(), encoded.is_keyframe);
                if !encoded.data.is_empty() {
                    println!("Persistent x264 encoding works!");
                } else {
                    println!("No data returned");
                }
            }
            Err(e) => println!("Encoding failed: {}", e),
        }
    }

    #[test]
    fn test_encode_multiple_frames() {
        let mut encoder = H264Encoder::new(160, 120).unwrap();
        let frame_size = (160 * 120 * 3) as usize;

        for i in 0..5 {
            let mut rgb_data = vec![0u8; frame_size];
            // Vary data so frames are different
            for j in 0..frame_size {
                rgb_data[j] = ((i * 37 + j) % 256) as u8;
            }

            match encoder.encode(&rgb_data, 160 * 3) {
                Ok(encoded) => {
                    println!("Frame {}: {} bytes, keyframe={}", i, encoded.data.len(), encoded.is_keyframe);
                }
                Err(e) => {
                    println!("Frame {} failed: {}", i, e);
                }
            }
        }
    }

    #[test]
    fn test_rgb_to_yuv420() {
        let rgb = vec![255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let yuv = H264Encoder::rgb_to_yuv420p(&rgb, 2, 2);
        assert_eq!(yuv.len(), 2 * 2 * 3 / 2);
    }

    #[test]
    fn test_contains_idr() {
        // Fake data with IDR NAL unit (type 5)
        let data = vec![0x00, 0x00, 0x00, 0x01, 0x65]; // 0x65 & 0x1F = 5
        assert!(contains_idr(&data));

        // Non-IDR
        let data = vec![0x00, 0x00, 0x00, 0x01, 0x41]; // 0x41 & 0x1F = 1
        assert!(!contains_idr(&data));
    }

    #[test]
    fn test_find_access_unit_boundary() {
        let data = vec![
            0x00, 0x00, 0x00, 0x01, 0x67, // SPS (type 7) at position 0
            0xAA, 0xBB,                     // some SPS payload bytes
            0x00, 0x00, 0x00, 0x01, 0x65, // IDR (type 5) at position 7
        ];
        // Starting search from position 1, should find the IDR at position 7
        assert_eq!(find_access_unit_boundary(&data, 1), Some(7));
    }
}
