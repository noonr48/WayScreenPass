//! Input Handler Module
//!
//! Maps SDL2 events to protocol ClientMessage types

use remote_desktop_core::protocol::{ClientMessage, KeyEvent, KeyState, PointerEvent, PointerEventType, ButtonState};

use crate::display::SdlEvent;

/// Input handler for converting SDL2 events to protocol messages
pub struct InputHandler;

impl InputHandler {
    /// Process an SDL2 event and convert to a protocol message if applicable
    pub fn process_event(event: &SdlEvent, display_width: u32, display_height: u32) -> Option<ClientMessage> {
        match event {
            SdlEvent::KeyEvent { keycode, pressed, .. } => {
                Some(ClientMessage::KeyEvent(KeyEvent {
                    key_code: *keycode,
                    state: if *pressed { KeyState::Pressed } else { KeyState::Released },
                }))
            }

            SdlEvent::MouseMotion { x, y, .. } => {
                // Normalize mouse position to video coordinates
                let (norm_x, norm_y) = normalize_position(*x, *y, display_width, display_height);
                Some(ClientMessage::PointerEvent(PointerEvent {
                    event_type: PointerEventType::Motion,
                    x: Some(norm_x),
                    y: Some(norm_y),
                    button: None,
                    button_state: None,
                    scroll_delta: None,
                }))
            }

            SdlEvent::MouseButton { button, pressed, x, y } => {
                // Include position with button event for accuracy
                let (norm_x, norm_y) = normalize_position(*x, *y, display_width, display_height);
                Some(ClientMessage::PointerEvent(PointerEvent {
                    event_type: PointerEventType::Button,
                    x: Some(norm_x),
                    y: Some(norm_y),
                    button: Some(*button),
                    button_state: Some(if *pressed { ButtonState::Pressed } else { ButtonState::Released }),
                    scroll_delta: None,
                }))
            }

            SdlEvent::MouseWheel { dy, .. } => {
                // Convert scroll to protocol format
                // SDL2 uses 1 unit per "click" typically
                let delta = (*dy * 120) as i16; // Scale to match typical high-res scroll
                Some(ClientMessage::PointerEvent(PointerEvent {
                    event_type: PointerEventType::Scroll,
                    x: None,
                    y: None,
                    button: None,
                    button_state: None,
                    scroll_delta: Some(delta),
                }))
            }
        }
    }
}

/// Normalize window coordinates to video coordinates
fn normalize_position(x: i32, y: i32, width: u32, height: u32) -> (u16, u16) {
    // Clamp to valid range and convert to u16
    let norm_x = x.clamp(0, width as i32 - 1) as u16;
    let norm_y = y.clamp(0, height as i32 - 1) as u16;
    (norm_x, norm_y)
}

/// Common keyboard keycodes for reference
/// These map to SDL2 Keycode values
#[allow(dead_code)]
pub mod keycodes {
    pub const UNKNOWN: u32 = 0;

    // Letters
    pub const A: u32 = 97;
    pub const B: u32 = 98;
    pub const C: u32 = 99;
    pub const D: u32 = 100;
    pub const E: u32 = 101;
    pub const F: u32 = 102;
    pub const G: u32 = 103;
    pub const H: u32 = 104;
    pub const I: u32 = 105;
    pub const J: u32 = 106;
    pub const K: u32 = 107;
    pub const L: u32 = 108;
    pub const M: u32 = 109;
    pub const N: u32 = 110;
    pub const O: u32 = 111;
    pub const P: u32 = 112;
    pub const Q: u32 = 113;
    pub const R: u32 = 114;
    pub const S: u32 = 115;
    pub const T: u32 = 116;
    pub const U: u32 = 117;
    pub const V: u32 = 118;
    pub const W: u32 = 119;
    pub const X: u32 = 120;
    pub const Y: u32 = 121;
    pub const Z: u32 = 122;

    // Numbers
    pub const NUM_0: u32 = 48;
    pub const NUM_1: u32 = 49;
    pub const NUM_2: u32 = 50;
    pub const NUM_3: u32 = 51;
    pub const NUM_4: u32 = 52;
    pub const NUM_5: u32 = 53;
    pub const NUM_6: u32 = 54;
    pub const NUM_7: u32 = 55;
    pub const NUM_8: u32 = 56;
    pub const NUM_9: u32 = 57;

    // Special keys
    pub const RETURN: u32 = 13;
    pub const ESCAPE: u32 = 27;
    pub const BACKSPACE: u32 = 8;
    pub const TAB: u32 = 9;
    pub const SPACE: u32 = 32;

    // Modifiers
    pub const LSHIFT: u32 = 1073742049;
    pub const RSHIFT: u32 = 1073742053;
    pub const LCTRL: u32 = 1073742048;
    pub const RCTRL: u32 = 1073742052;
    pub const LALT: u32 = 1073742050;
    pub const RALT: u32 = 1073742054;
    pub const LGUI: u32 = 1073742051;
    pub const RGUI: u32 = 1073742055;

    // Arrow keys
    pub const UP: u32 = 1073741906;
    pub const DOWN: u32 = 1073741905;
    pub const LEFT: u32 = 1073741904;
    pub const RIGHT: u32 = 1073741903;

    // Function keys
    pub const F1: u32 = 1073741882;
    pub const F2: u32 = 1073741883;
    pub const F3: u32 = 1073741884;
    pub const F4: u32 = 1073741885;
    pub const F5: u32 = 1073741886;
    pub const F6: u32 = 1073741887;
    pub const F7: u32 = 1073741888;
    pub const F8: u32 = 1073741889;
    pub const F9: u32 = 1073741890;
    pub const F10: u32 = 1073741891;
    pub const F11: u32 = 1073741892;
    pub const F12: u32 = 1073741893;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_conversion() {
        let event = SdlEvent::KeyEvent {
            keycode: keycodes::A,
            pressed: true,
            modifiers: 0,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_some());

        if let Some(ClientMessage::KeyEvent(key_event)) = msg {
            assert_eq!(key_event.key_code, keycodes::A);
            assert_eq!(key_event.state, KeyState::Pressed);
        } else {
            panic!("Expected KeyEvent");
        }
    }

    #[test]
    fn test_mouse_motion_conversion() {
        let event = SdlEvent::MouseMotion {
            x: 100,
            y: 200,
            xrel: 10,
            yrel: 20,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_some());

        if let Some(ClientMessage::PointerEvent(ptr_event)) = msg {
            assert_eq!(ptr_event.event_type, PointerEventType::Motion);
            assert_eq!(ptr_event.x, Some(100));
            assert_eq!(ptr_event.y, Some(200));
        } else {
            panic!("Expected PointerEvent");
        }
    }

    #[test]
    fn test_mouse_button_conversion() {
        let event = SdlEvent::MouseButton {
            button: 1,
            pressed: true,
            x: 50,
            y: 100,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_some());

        if let Some(ClientMessage::PointerEvent(ptr_event)) = msg {
            assert_eq!(ptr_event.event_type, PointerEventType::Button);
            assert_eq!(ptr_event.button, Some(1));
            assert_eq!(ptr_event.button_state, Some(ButtonState::Pressed));
        } else {
            panic!("Expected PointerEvent");
        }
    }
}
