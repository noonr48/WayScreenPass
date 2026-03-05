//! H.264 Video Decoder
//!
//! Uses ffmpeg to decode H.264 NAL units into YUV420P frames

use anyhow::{Result, anyhow};
use ffmpeg_next as ffmpeg;
use tracing::{debug, warn};

/// Decoded video frame in YUV420P format
#[derive(Debug)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    /// Y plane (luminance)
    pub y_plane: Vec<u8>,
    /// U plane (chrominance)
    pub u_plane: Vec<u8>,
    /// V plane (chrominance)
    pub v_plane: Vec<u8>,
}

/// H.264 video decoder using ffmpeg
pub struct VideoDecoder {
    decoder: ffmpeg::decoder::Video,
    width: u32,
    height: u32,
}

impl VideoDecoder {
    /// Create a new H.264 decoder
    pub fn new() -> Result<Self> {
        // Initialize ffmpeg (safe to call multiple times)
        ffmpeg::init().map_err(|e| anyhow!("Failed to initialize ffmpeg: {}", e))?;

        // Find H.264 decoder
        let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::H264)
            .ok_or_else(|| anyhow!("H.264 decoder not found"))?;

        // Create decoder context and get video decoder immediately
        let context = ffmpeg::codec::context::Context::new_with_codec(codec);
        let decoder = context.decoder().video()
            .map_err(|e| anyhow!("Failed to create video decoder: {}", e))?;

        debug!("H.264 decoder initialized");

        Ok(Self {
            decoder,
            width: 0,
            height: 0,
        })
    }

    /// Set expected video dimensions (called when stream starts)
    pub fn set_dimensions(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        debug!("Decoder dimensions set: {}x{}", width, height);
    }

    /// Decode an H.264 NAL unit into a YUV420P frame
    ///
    /// Returns Some(DecodedFrame) if a complete frame was decoded,
    /// None if more data is needed.
    pub fn decode(&mut self, data: &[u8], _is_keyframe: bool) -> Result<Option<DecodedFrame>> {
        // Create a packet from the input data
        let packet = ffmpeg::Packet::copy(data);

        // Send packet to decoder
        if let Err(e) = self.decoder.send_packet(&packet) {
            warn!("Decoder send_packet error (continuing): {}", e);
        }

        // Try to receive a decoded frame
        let mut decoded_frame = ffmpeg::util::frame::Video::empty();
        match self.decoder.receive_frame(&mut decoded_frame) {
            Ok(()) => {
                // Successfully decoded a frame
                let frame = self.extract_frame(&decoded_frame)?;
                Ok(Some(frame))
            }
            Err(e) => {
                // Check if it's just "need more data" (EAGAIN) or EOF
                let err_str = format!("{}", e);
                if err_str.contains("Resource temporarily unavailable") || err_str.contains("EAGAIN") {
                    // Need more input data - this is normal
                    Ok(None)
                } else if err_str.contains("EOF") {
                    Ok(None)
                } else {
                    // Real error - but don't fail completely, just skip this frame
                    warn!("Decoder receive_frame error: {}", e);
                    Ok(None)
                }
            }
        }
    }

    /// Extract YUV420P data from a decoded ffmpeg frame
    fn extract_frame(&self, frame: &ffmpeg::util::frame::Video) -> Result<DecodedFrame> {
        let width = frame.width();
        let height = frame.height();

        if width == 0 || height == 0 {
            return Err(anyhow!("Decoded frame has zero dimensions"));
        }

        // Ensure we have YUV420P format
        let format = frame.format();
        if format != ffmpeg::util::format::pixel::Pixel::YUV420P {
            warn!("Unexpected pixel format: {:?}, expected YUV420P", format);
        }

        // Calculate plane sizes
        let y_size = (width * height) as usize;
        let uv_width = width / 2;
        let uv_height = height / 2;
        let uv_size = (uv_width * uv_height) as usize;

        // Extract planes
        let mut y_plane = Vec::with_capacity(y_size);
        let mut u_plane = Vec::with_capacity(uv_size);
        let mut v_plane = Vec::with_capacity(uv_size);

        // Y plane (full resolution)
        let y_data = frame.data(0);
        let y_linesize = frame.stride(0) as usize;
        for row in 0..height as usize {
            let start = row * y_linesize;
            y_plane.extend_from_slice(&y_data[start..start + width as usize]);
        }

        // U plane (half resolution)
        let u_data = frame.data(1);
        let u_linesize = frame.stride(1) as usize;
        for row in 0..uv_height as usize {
            let start = row * u_linesize;
            u_plane.extend_from_slice(&u_data[start..start + uv_width as usize]);
        }

        // V plane (half resolution)
        let v_data = frame.data(2);
        let v_linesize = frame.stride(2) as usize;
        for row in 0..uv_height as usize {
            let start = row * v_linesize;
            v_plane.extend_from_slice(&v_data[start..start + uv_width as usize]);
        }

        debug!("Decoded frame: {}x{}, Y={}B, U={}B, V={}B",
            width, height, y_plane.len(), u_plane.len(), v_plane.len());

        Ok(DecodedFrame {
            width,
            height,
            y_plane,
            u_plane,
            v_plane,
        })
    }

    /// Flush the decoder and get any remaining frames
    pub fn flush(&mut self) -> Result<Option<DecodedFrame>> {
        // Send EOF to signal end of stream
        let _ = self.decoder.send_eof();

        // Try to get remaining frames
        let mut decoded_frame = ffmpeg::util::frame::Video::empty();
        match self.decoder.receive_frame(&mut decoded_frame) {
            Ok(()) => {
                let frame = self.extract_frame(&decoded_frame)?;
                Ok(Some(frame))
            }
            _ => Ok(None),
        }
    }
}

impl Default for VideoDecoder {
    fn default() -> Self {
        Self::new().expect("Failed to create default VideoDecoder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = VideoDecoder::new();
        assert!(decoder.is_ok());
    }
}
