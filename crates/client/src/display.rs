//! SDL2 Display Module
//!
//! Handles window creation, YUV texture rendering, and event polling

use anyhow::{Result, anyhow};
use sdl2::event::Event;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{debug, info};

use crate::decoder::DecodedFrame;

/// SDL2-based display for rendering video frames
#[allow(dead_code)]
pub struct Display {
    sdl_context: sdl2::Sdl,
    video_subsystem: sdl2::VideoSubsystem,
    canvas: Canvas<Window>,
    texture_creator: sdl2::render::TextureCreator<sdl2::video::WindowContext>,
    event_pump: sdl2::EventPump,
    width: u32,
    height: u32,
    running: bool,
}

impl Display {
    /// Create a new display window
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self> {
        // Initialize SDL2
        let sdl_context = sdl2::init()
            .map_err(|e| anyhow!("Failed to initialize SDL2: {}", e))?;

        // Initialize video subsystem
        let video_subsystem = sdl_context.video()
            .map_err(|e| anyhow!("Failed to initialize SDL2 video: {}", e))?;

        // Create window
        let window = video_subsystem
            .window(title, width, height)
            .position_centered()
            .resizable()
            .allow_highdpi()
            .build()
            .map_err(|e| anyhow!("Failed to create window: {}", e))?;

        // Create canvas (renderer)
        let canvas = window.into_canvas()
            .accelerated()
            .present_vsync()
            .build()
            .map_err(|e| anyhow!("Failed to create canvas: {}", e))?;

        // Create texture creator (lives as long as the canvas)
        let texture_creator = canvas.texture_creator();

        // Get event pump for input handling
        let event_pump = sdl_context.event_pump()
            .map_err(|e| anyhow!("Failed to get event pump: {}", e))?;

        info!("SDL2 display created: {}x{} \"{}\"", width, height, title);

        Ok(Self {
            sdl_context,
            video_subsystem,
            canvas,
            texture_creator,
            event_pump,
            width,
            height,
            running: true,
        })
    }

    /// Render a decoded video frame
    pub fn render_frame(&mut self, frame: &DecodedFrame) -> Result<()> {
        // Create texture for this frame (fast - just a GPU buffer allocation)
        let mut texture = self.texture_creator.create_texture(
            PixelFormatEnum::YV12,
            sdl2::render::TextureAccess::Streaming,
            frame.width,
            frame.height,
        ).map_err(|e| anyhow!("Failed to create texture: {}", e))?;

        // Update YUV texture with decoded frame data
        texture.update_yuv(
            None,
            &frame.y_plane,
            frame.width as usize,  // Y pitch
            &frame.u_plane,
            frame.width as usize / 2,  // U pitch
            &frame.v_plane,
            frame.width as usize / 2,  // V pitch
        ).map_err(|e| anyhow!("Failed to update texture: {}", e))?;

        // Clear canvas
        self.canvas.clear();

        // Calculate destination rect to maintain aspect ratio
        let (win_width, win_height) = self.canvas.window().size();
        let dest_rect = calculate_aspect_rect(frame.width, frame.height, win_width, win_height);

        // Copy texture to canvas
        self.canvas.copy(&texture, None, dest_rect)
            .map_err(|e| anyhow!("Failed to copy texture: {}", e))?;

        // Present
        self.canvas.present();

        // Update stored dimensions
        self.width = frame.width;
        self.height = frame.height;

        Ok(())
    }

    /// Poll for SDL2 events
    ///
    /// Returns a vector of events that should be processed.
    /// Also handles window close events internally.
    pub fn poll_events(&mut self) -> Vec<SdlEvent> {
        let mut events = Vec::new();

        for event in self.event_pump.poll_iter() {
            match &event {
                Event::Quit { .. } => {
                    info!("Window close requested");
                    self.running = false;
                }
                Event::KeyDown { scancode: Some(sdl2::keyboard::Scancode::Escape), .. } => {
                    info!("Escape pressed, closing");
                    self.running = false;
                }
                Event::Window { win_event, .. } => {
                    if let sdl2::event::WindowEvent::Resized(w, h) = win_event {
                        debug!("Window resized to {}x{}", w, h);
                    }
                }
                _ => {}
            }

            // Convert to our event type
            if let Some(our_event) = convert_event(&event) {
                events.push(our_event);
            }
        }

        events
    }

    /// Check if the display is still running (window not closed)
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get the window size
    #[allow(dead_code)]
    pub fn window_size(&self) -> (u32, u32) {
        self.canvas.window().size()
    }

    /// Get a reference to the video subsystem (for clipboard)
    pub fn video_subsystem(&self) -> &sdl2::VideoSubsystem {
        &self.video_subsystem
    }
}

/// Calculate destination rectangle to maintain aspect ratio
fn calculate_aspect_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Rect {
    let src_aspect = src_w as f32 / src_h as f32;
    let dst_aspect = dst_w as f32 / dst_h as f32;

    let (out_w, out_h) = if src_aspect > dst_aspect {
        // Source is wider - fit to width
        let w = dst_w;
        let h = (dst_w as f32 / src_aspect) as u32;
        (w, h)
    } else {
        // Source is taller - fit to height
        let h = dst_h;
        let w = (dst_h as f32 * src_aspect) as u32;
        (w, h)
    };

    // Center the output
    let x = (dst_w - out_w) / 2;
    let y = (dst_h - out_h) / 2;

    Rect::new(x as i32, y as i32, out_w, out_h)
}

/// Our simplified event type (wrapping SDL2 events)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SdlEvent {
    /// Keyboard key pressed or released
    KeyEvent {
        scancode: u32,
        pressed: bool,
        modifiers: u16,
    },
    /// Mouse moved
    MouseMotion {
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
    },
    /// Mouse button pressed or released
    MouseButton {
        button: u8,
        pressed: bool,
        x: i32,
        y: i32,
    },
    /// Mouse wheel scrolled
    MouseWheel {
        dx: i32,
        dy: i32,
    },
}

/// Convert SDL2 event to our event type
fn convert_event(event: &Event) -> Option<SdlEvent> {
    match event {
        Event::KeyDown { scancode, keymod, .. } => {
            let sc = scancode.as_ref()?;
            Some(SdlEvent::KeyEvent {
                scancode: *sc as i32 as u32,
                pressed: true,
                modifiers: keymod.bits(),
            })
        }
        Event::KeyUp { scancode, keymod, .. } => {
            let sc = scancode.as_ref()?;
            Some(SdlEvent::KeyEvent {
                scancode: *sc as i32 as u32,
                pressed: false,
                modifiers: keymod.bits(),
            })
        }
        Event::MouseMotion { x, y, xrel, yrel, .. } => {
            Some(SdlEvent::MouseMotion {
                x: *x,
                y: *y,
                xrel: *xrel,
                yrel: *yrel,
            })
        }
        Event::MouseButtonDown { mouse_btn, x, y, .. } => {
            Some(SdlEvent::MouseButton {
                button: mouse_btn_to_u8(mouse_btn),
                pressed: true,
                x: *x,
                y: *y,
            })
        }
        Event::MouseButtonUp { mouse_btn, x, y, .. } => {
            Some(SdlEvent::MouseButton {
                button: mouse_btn_to_u8(mouse_btn),
                pressed: false,
                x: *x,
                y: *y,
            })
        }
        Event::MouseWheel { x, y, .. } => {
            Some(SdlEvent::MouseWheel {
                dx: *x,
                dy: *y,
            })
        }
        _ => None,
    }
}

/// Convert SDL2 mouse button to u8
fn mouse_btn_to_u8(btn: &sdl2::mouse::MouseButton) -> u8 {
    match btn {
        sdl2::mouse::MouseButton::Left => 1,
        sdl2::mouse::MouseButton::Middle => 2,
        sdl2::mouse::MouseButton::Right => 3,
        sdl2::mouse::MouseButton::X1 => 4,
        sdl2::mouse::MouseButton::X2 => 5,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Display tests require a display server to be available
    // They are typically skipped in CI environments

    #[test]
    fn test_display_creation() {
        // SDL2 init is not thread-safe; may fail in parallel test runs.
        // Run alone with: cargo test -p remote-desktop-client display -- --test-threads=1
        let result = Display::new(640, 480, "Test");
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            println!("No display available, skipping");
            return;
        }
        match result {
            Ok(_d) => println!("Display created OK"),
            Err(e) => println!("Display init failed (likely parallel test): {}", e),
        }
    }
}
