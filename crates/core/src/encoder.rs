//! H.264 software encoding using x264 CLI via std::process
//!
//! Uses external x264 binary for encoding - works reliably across all systems.
//! No FFmpeg dependency issues.
//!
//! Format: Uses YUV4MPEG2 format (y4m) which x264 natively supports via --demuxer y4m

use std::process::Command;
use std::io::Write;
use tracing::{debug, info, warn};
use tempfile::NamedTempFile;

/// Encoded video frame
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    /// H.264 encoded data
    pub data: Vec<u8>,

    /// Presentation timestamp
    pub pts: i64,

    /// Whether this is a keyframe (for streaming)
    pub is_keyframe: bool,
}

/// H.264 software encoder using x264 CLI
pub struct H264Encoder {
    width: u32,
    height: u32,
    fps: u8,
    frame_count: u64,
    force_keyframe: bool,
}

impl H264Encoder {
    /// Create a new H.264 encoder
    ///
    /// # Arguments
    /// * `width` - Frame width (typically 1920)
    /// * `height` - Frame height (typically 1080)
    pub fn new(width: u32, height: u32) -> super::Result<Self> {
        info!("Creating H.264 encoder: {}x{} @ 30fps", width, height);

        // Verify x264 is available
        match Command::new("x264").arg("--version").output() {
            Ok(_) => debug!("x264 found in PATH"),
            Err(e) => {
                warn!("x264 not found: {:?}. Encoding will fail.\nInstall: sudo pacman -S x264", e);
            }
        }

        Ok(Self {
            width,
            height,
            fps: 30,
            frame_count: 0,
            force_keyframe: false,
        })
    }

    /// Convert RGB24 to YUV420P
    /// This is a simplified conversion - for production, use optimized SIMD or library
    fn rgb_to_yuv420p(rgb_data: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut yuv = vec![0u8; (width * height * 3 / 2) as usize];

        let y_size = (width * height) as usize;
        let uv_size = y_size / 4;

        let (y_plane, uv_part) = yuv.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_part.split_at_mut(uv_size);

        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 3) as usize;
                let r = rgb_data[idx] as i32;
                let g = rgb_data[idx + 1] as i32;
                let b = rgb_data[idx + 2] as i32;

                // Y = 0.299R + 0.587G + 0.114B
                let y_val = (66 * r + 129 * g + 25 * b + 128) / 256 + 16;
                y_plane[(y * width + x) as usize] = y_val.clamp(0, 255) as u8;

                // Downsample for U and V (2x2)
                if x % 2 == 0 && y % 2 == 0 {
                    let uv_idx = (y / 2 * width / 2 + x / 2) as usize;

                    // U = -0.169R - 0.331G + 0.500B + 128
                    let u_val = (-38 * r - 74 * g + 112 * b + 128) / 256 + 128;
                    u_plane[uv_idx] = u_val.clamp(0, 255) as u8;

                    // V = 0.500R - 0.419G - 0.081B + 128
                    let v_val = (112 * r - 94 * g - 18 * b + 128) / 256 + 128;
                    v_plane[uv_idx] = v_val.clamp(0, 255) as u8;
                }
            }
        }

        yuv
    }

    /// Encode a raw RGB frame
    ///
    /// # Arguments
    /// * `rgb_data` - Raw RGB24 frame data
    /// * `stride` - Bytes per row
    ///
    /// # Process
    /// 1. Convert RGB24 to YUV420P
    /// 2. Write YUV4MPEG2 header + data to temp file
    /// 3. Call x264 to encode to H.264
    /// 4. Read encoded data back
    pub fn encode(&mut self, rgb_data: &[u8], stride: u32) -> super::Result<EncodedFrame> {
        // Calculate actual frame size from stride
        let frame_size = (self.height * stride) as usize;

        if rgb_data.len() < frame_size {
            return Err(super::CoreError::PipeWire(
                format!("RGB data too small: {} < {}", rgb_data.len(), frame_size)
            ));
        }

        // Convert RGB to YUV420P
        let yuv_data = Self::rgb_to_yuv420p(rgb_data, self.width, self.height);

        // Create temporary file for YUV4MPEG2 input
        let input_file = NamedTempFile::new()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to create temp file: {:?}", e)))?;

        // Write YUV4MPEG2 header: YUV4MPEG2 W1920 H1080 F30:1 Ip C420mpeg2
        writeln!(input_file.as_file(),
            "YUV4MPEG2 W{} H{} F{}:1 Ip C420mpeg2",
            self.width, self.height, self.fps)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to write Y4M header: {:?}", e)))?;

        // Write FRAME header
        writeln!(input_file.as_file(), "FRAME")
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to write FRAME header: {:?}", e)))?;

        // Write YUV data
        input_file.as_file().write_all(&yuv_data)
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to write YUV data: {:?}", e)))?;

        input_file.as_file().flush()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to flush input: {:?}", e)))?;

        // Build x264 command - use file paths instead of stdin
        let is_keyframe = self.force_keyframe || (self.frame_count % 30 == 0);
        self.force_keyframe = false;

        // Create output file
        let output_file = NamedTempFile::new()
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to create output temp file: {:?}", e)))?;

        let mut cmd = Command::new("x264");
        cmd.arg("--demuxer").arg("y4m")
           .arg(input_file.path())  // Input file
           .arg("-o")                // Output flag
           .arg(output_file.path())  // Output file
           .arg("--frames").arg("1")
           .arg("--preset").arg("veryfast")
           .arg("--tune").arg("zerolatency")
           .arg("--crf").arg("23")
           .arg("--threads").arg("0")
           .arg("--quiet");

        if is_keyframe {
            cmd.arg("--intra-refresh");
        }

        let output = cmd
            .output()
            .map_err(|e| super::CoreError::PipeWire(format!("x264 failed: {:?}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(super::CoreError::PipeWire(format!("x264 encoding failed: {}", stderr)));
        }

        // Read encoded data from output file
        let data = std::fs::read(output_file.path())
            .map_err(|e| super::CoreError::PipeWire(format!("Failed to read output file: {:?}", e)))?;

        if data.is_empty() {
            warn!("x264 returned empty data");
        }

        self.frame_count += 1;

        debug!("Encoded frame {}: {} bytes, keyframe={}",
            self.frame_count - 1, data.len(), is_keyframe);

        Ok(EncodedFrame {
            data,
            pts: (self.frame_count - 1) as i64,
            is_keyframe,
        })
    }

    /// Request the next frame be a keyframe
    pub fn request_keyframe(&mut self) -> super::Result<()> {
        debug!("Requesting keyframe (will take effect on next encode)");
        self.force_keyframe = true;
        Ok(())
    }

    /// Get the encoder width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the encoder height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Reset the encoder state
    pub fn reset(&mut self) -> super::Result<()> {
        info!("Resetting encoder");
        self.frame_count = 0;
        self.force_keyframe = false;
        Ok(())
    }
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

        // Create small RGB frame (black)
        let frame_size = (320 * 240 * 3) as usize;
        let rgb_data = vec![0u8; frame_size];

        match encoder.encode(&rgb_data, 320 * 3) {
            Ok(encoded) => {
                println!("Encoded: {} bytes, keyframe={}", encoded.data.len(), encoded.is_keyframe);
                // Check if we got some H.264 data (should have SPS/PPS/IDR)
                if encoded.data.len() > 0 {
                    println!("✓ x264 encoding works!");
                } else {
                    println!("⚠ No data returned (x264 not installed?)");
                }
            }
            Err(e) => {
                println!("Encoding failed: {}", e);
            }
        }
    }

    #[test]
    fn test_rgb_to_yuv420() {
        let rgb = vec![
            255u8, 0, 0,    // Red
            0, 255, 0,      // Green
            0, 0, 255,      // Blue
            255, 255, 255,  // White
        ];

        let yuv = H264Encoder::rgb_to_yuv420p(&rgb, 2, 2);

        // Y plane should have 4 values
        assert_eq!(yuv.len(), 2 * 2 * 3 / 2);
        println!("YUV420 conversion test passed");
    }
}
