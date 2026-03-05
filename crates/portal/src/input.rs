//! Input injection for remote desktop
//!
//! # Backends
//! - uinput: Uses evdev crate for /dev/uinput virtual devices
//! - Stub: Logs events (for testing)

use serde::{Deserialize, Serialize};
use evdev::{uinput::VirtualDeviceBuilder, InputEvent, KeyCode, EventType};
use tracing::{debug, info, warn, error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEvent {
    pub keycode: u32,
    pub state: KeyState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PointerEvent {
    MotionAbsolute { x: f64, y: f64 },
    MotionRelative { dx: i32, dy: i32 },
    Button { button: u32, state: ButtonState },
    Scroll { delta_x: i32, delta_y: i32 },
}

#[derive(Debug, Clone, Copy)]
pub enum InputBackend {
    Uinput,
    Libei,
    XTest,
    Stub,
}

pub struct InputHandler {
    backend: InputBackend,
    keyboard_device: Option<evdev::uinput::VirtualDevice>,
    mouse_device: Option<evdev::uinput::VirtualDevice>,
}

impl InputHandler {
    pub fn new(backend: InputBackend) -> super::Result<Self> {
        match backend {
            InputBackend::Uinput => {
                match Self::create_uinput_devices() {
                    Ok((keyboard, mouse)) => {
                        info!("uinput backend: Created virtual keyboard and mouse");
                        Ok(Self {
                            backend,
                            keyboard_device: Some(keyboard),
                            mouse_device: Some(mouse),
                        })
                    }
                    Err(e) => {
                        warn!("uinput creation failed: {:?}, using stub mode", e);
                        warn!("To enable uinput:");
                        warn!("  1. sudo modprobe uinput");
                        warn!("  2. sudo usermod -aG input $USER");
                        warn!("  3. Log out and back in");
                        Ok(Self {
                            backend: InputBackend::Stub,
                            keyboard_device: None,
                            mouse_device: None,
                        })
                    }
                }
            }
            _ => Ok(Self {
                backend,
                keyboard_device: None,
                mouse_device: None,
            }),
        }
    }

    fn create_uinput_devices() -> super::Result<(evdev::uinput::VirtualDevice, evdev::uinput::VirtualDevice)> {
        // Create virtual keyboard
        let keyboard = VirtualDeviceBuilder::new()?
            .name(b"Remote Desktop Keyboard")
            .build()
            .map_err(|e| super::PortalError::Portal(format!("Failed to create keyboard: {:?}", e)))?;

        // Create virtual mouse
        let mouse = VirtualDeviceBuilder::new()?
            .name(b"Remote Desktop Mouse")
            .build()
            .map_err(|e| super::PortalError::Portal(format!("Failed to create mouse: {:?}", e)))?;

        Ok((keyboard, mouse))
    }

    pub fn send_key(&mut self, event: &KeyEvent) -> super::Result<()> {
        if let Some(keyboard) = &mut self.keyboard_device {
            let key = KeyCode::new(event.keycode as u16);

            let value = match event.state {
                KeyState::Pressed => 1,
                KeyState::Released => 0,
            };

            let input_event = InputEvent::new(EventType::KEY.0, key.0, value);
            keyboard.emit(&[input_event])
                .map_err(|e| super::PortalError::Portal(format!("Emit failed: {:?}", e)))?;

            debug!("uinput: Injected key {} state={}", event.keycode, value);
            return Ok(());
        }

        // Stub mode
        debug!("Stub: key {} {:?}", event.keycode, event.state);
        Ok(())
    }

    pub fn send_pointer(&mut self, event: &PointerEvent) -> super::Result<()> {
        if let Some(mouse) = &mut self.mouse_device {
            let events: Vec<InputEvent> = match event {
                PointerEvent::MotionRelative { dx, dy } => {
                    vec![
                        InputEvent::new(EventType::RELATIVE.0, 0x00, *dx),
                        InputEvent::new(EventType::RELATIVE.0, 0x01, *dy),
                    ]
                }
                PointerEvent::Button { button, state } => {
                    let key = match button {
                        1 => KeyCode::BTN_LEFT,
                        2 => KeyCode::BTN_MIDDLE,
                        3 => KeyCode::BTN_RIGHT,
                        4 => KeyCode::BTN_SIDE,
                        5 => KeyCode::BTN_EXTRA,
                        _ => return Err(super::PortalError::Portal(format!("Invalid button: {}", button))),
                    };
                    let value = match state {
                        ButtonState::Pressed => 1,
                        ButtonState::Released => 0,
                    };
                    vec![InputEvent::new(EventType::KEY.0, key.0, value)]
                }
                PointerEvent::Scroll { delta_y, .. } => {
                    vec![InputEvent::new(EventType::RELATIVE.0, 0x08, *delta_y)]
                }
                PointerEvent::MotionAbsolute { .. } => {
                    vec![]
                }
            };

            if !events.is_empty() {
                mouse.emit(&events)?;
                debug!("uinput: Injected pointer event: {:?}", event);
            }
            return Ok(());
        }

        // Stub mode
        debug!("Stub: pointer {:?}", event);
        Ok(())
    }

    pub fn backend(&self) -> InputBackend {
        self.backend
    }
}

pub mod keycodes {
    pub const KEY_ENTER: u32 = 28;
    pub const KEY_ESC: u32 = 1;
    pub const KEY_SPACE: u32 = 57;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub() {
        let mut handler = InputHandler::new(InputBackend::Stub).unwrap();
        let event = KeyEvent { keycode: 28, state: KeyState::Pressed };
        assert!(handler.send_key(&event).is_ok());
    }
}
