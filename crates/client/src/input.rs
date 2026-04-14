//! Input Handler Module
//!
//! Maps SDL2 events to protocol ClientMessage types

use remote_desktop_core::protocol::{ClientMessage, KeyEvent, KeyState, PointerEvent, PointerEventType, ButtonState};

use crate::display::SdlEvent;

/// Convert SDL2 scancode (USB HID based) to Linux evdev keycode
fn sdl2_scancode_to_evdev(scancode: u32) -> Option<u32> {
    Some(match scancode {
        // Letters (SDL2: 4-29 -> evdev: QWERTY layout)
        4 => 30,   // A
        5 => 48,   // B
        6 => 46,   // C
        7 => 32,   // D
        8 => 18,   // E
        9 => 33,   // F
        10 => 34,  // G
        11 => 35,  // H
        12 => 23,  // I
        13 => 36,  // J
        14 => 37,  // K
        15 => 38,  // L
        16 => 50,  // M
        17 => 49,  // N
        18 => 24,  // O
        19 => 25,  // P
        20 => 16,  // Q
        21 => 19,  // R
        22 => 31,  // S
        23 => 20,  // T
        24 => 22,  // U
        25 => 47,  // V
        26 => 17,  // W
        27 => 45,  // X
        28 => 21,  // Y
        29 => 44,  // Z

        // Numbers (SDL2: 30-39 -> evdev: 2-11)
        30 => 2,   // 1
        31 => 3,   // 2
        32 => 4,   // 3
        33 => 5,   // 4
        34 => 6,   // 5
        35 => 7,   // 6
        36 => 8,   // 7
        37 => 9,   // 8
        38 => 10,  // 9
        39 => 11,  // 0

        // Control keys
        40 => 28,   // Return/Enter
        41 => 1,    // Escape
        42 => 14,   // Backspace
        43 => 15,   // Tab
        44 => 57,   // Space
        45 => 12,   // Minus
        46 => 13,   // Equals
        47 => 26,   // Left Bracket
        48 => 27,   // Right Bracket
        49 => 43,   // Backslash
        51 => 39,   // Semicolon
        52 => 40,   // Apostrophe
        53 => 41,   // Grave/Backtick
        54 => 51,   // Comma
        55 => 52,   // Period
        56 => 53,   // Slash

        // Function keys (SDL2: 58-69 -> evdev: 59-68, 87-88)
        58 => 59,   // F1
        59 => 60,   // F2
        60 => 61,   // F3
        61 => 62,   // F4
        62 => 63,   // F5
        63 => 64,   // F6
        64 => 65,   // F7
        65 => 66,   // F8
        66 => 67,   // F9
        67 => 68,   // F10
        68 => 87,   // F11
        69 => 88,   // F12

        // Navigation
        73 => 110,  // Insert
        74 => 102,  // Home
        75 => 104,  // PageUp
        76 => 111,  // Delete
        77 => 107,  // End
        78 => 109,  // PageDown
        79 => 106,  // Right Arrow
        80 => 105,  // Left Arrow
        81 => 108,  // Down Arrow
        82 => 103,  // Up Arrow

        // Modifiers
        224 => 29,  // Left Ctrl
        225 => 42,  // Left Shift
        226 => 56,  // Left Alt
        227 => 125, // Left Super/Meta
        228 => 97,  // Right Ctrl
        229 => 54,  // Right Shift
        230 => 100, // Right Alt
        231 => 126, // Right Super/Meta

        // Print Screen, Scroll Lock, Pause
        70 => 99,   // PrintScreen
        71 => 70,   // ScrollLock
        72 => 119,  // Pause

        // Caps Lock, Num Lock
        57 => 58,   // CapsLock
        83 => 69,   // NumLock

        _ => return None,
    })
}

/// Input handler for converting SDL2 events to protocol messages
pub struct InputHandler;

impl InputHandler {
    /// Process an SDL2 event and convert to a protocol message if applicable
    pub fn process_event(event: &SdlEvent, display_width: u32, display_height: u32) -> Option<ClientMessage> {
        match event {
            SdlEvent::KeyEvent { scancode, pressed, .. } => {
                let evdev_code = sdl2_scancode_to_evdev(*scancode)?;
                Some(ClientMessage::KeyEvent(KeyEvent {
                    key_code: evdev_code,
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

/// SDL2 scancode constants for reference
/// These are the SDL2 scancode values (USB HID based) used as input
/// to sdl2_scancode_to_evdev() for translation to Linux evdev keycodes
#[allow(dead_code)]
pub mod scancodes {
    // Letters (SDL2 scancodes)
    pub const A: u32 = 4;
    pub const B: u32 = 5;
    pub const C: u32 = 6;
    pub const D: u32 = 7;
    pub const E: u32 = 8;
    pub const F: u32 = 9;
    pub const G: u32 = 10;
    pub const H: u32 = 11;
    pub const I: u32 = 12;
    pub const J: u32 = 13;
    pub const K: u32 = 14;
    pub const L: u32 = 15;
    pub const M: u32 = 16;
    pub const N: u32 = 17;
    pub const O: u32 = 18;
    pub const P: u32 = 19;
    pub const Q: u32 = 20;
    pub const R: u32 = 21;
    pub const S: u32 = 22;
    pub const T: u32 = 23;
    pub const U: u32 = 24;
    pub const V: u32 = 25;
    pub const W: u32 = 26;
    pub const X: u32 = 27;
    pub const Y: u32 = 28;
    pub const Z: u32 = 29;

    // Numbers (SDL2 scancodes)
    pub const NUM_1: u32 = 30;
    pub const NUM_2: u32 = 31;
    pub const NUM_3: u32 = 32;
    pub const NUM_4: u32 = 33;
    pub const NUM_5: u32 = 34;
    pub const NUM_6: u32 = 35;
    pub const NUM_7: u32 = 36;
    pub const NUM_8: u32 = 37;
    pub const NUM_9: u32 = 38;
    pub const NUM_0: u32 = 39;

    // Special keys (SDL2 scancodes)
    pub const RETURN: u32 = 40;
    pub const ESCAPE: u32 = 41;
    pub const BACKSPACE: u32 = 42;
    pub const TAB: u32 = 43;
    pub const SPACE: u32 = 44;

    // Modifiers (SDL2 scancodes)
    pub const LCTRL: u32 = 224;
    pub const LSHIFT: u32 = 225;
    pub const LALT: u32 = 226;
    pub const LGUI: u32 = 227;
    pub const RCTRL: u32 = 228;
    pub const RSHIFT: u32 = 229;
    pub const RALT: u32 = 230;
    pub const RGUI: u32 = 231;

    // Arrow keys (SDL2 scancodes)
    pub const RIGHT: u32 = 79;
    pub const LEFT: u32 = 80;
    pub const DOWN: u32 = 81;
    pub const UP: u32 = 82;

    // Function keys (SDL2 scancodes)
    pub const F1: u32 = 58;
    pub const F2: u32 = 59;
    pub const F3: u32 = 60;
    pub const F4: u32 = 61;
    pub const F5: u32 = 62;
    pub const F6: u32 = 63;
    pub const F7: u32 = 64;
    pub const F8: u32 = 65;
    pub const F9: u32 = 66;
    pub const F10: u32 = 67;
    pub const F11: u32 = 68;
    pub const F12: u32 = 69;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_conversion() {
        // SDL2 scancode for A is 4, evdev keycode for A is 30
        let event = SdlEvent::KeyEvent {
            scancode: scancodes::A,
            pressed: true,
            modifiers: 0,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_some());

        if let Some(ClientMessage::KeyEvent(key_event)) = msg {
            assert_eq!(key_event.key_code, 30); // evdev KEY_A
            assert_eq!(key_event.state, KeyState::Pressed);
        } else {
            panic!("Expected KeyEvent");
        }
    }

    #[test]
    fn test_key_event_return() {
        // SDL2 scancode for Return is 40, evdev keycode is 28
        let event = SdlEvent::KeyEvent {
            scancode: scancodes::RETURN,
            pressed: true,
            modifiers: 0,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_some());

        if let Some(ClientMessage::KeyEvent(key_event)) = msg {
            assert_eq!(key_event.key_code, 28); // evdev KEY_ENTER
            assert_eq!(key_event.state, KeyState::Pressed);
        } else {
            panic!("Expected KeyEvent");
        }
    }

    #[test]
    fn test_key_event_unknown_scancode() {
        // Unknown scancode should return None
        let event = SdlEvent::KeyEvent {
            scancode: 999,
            pressed: true,
            modifiers: 0,
        };

        let msg = InputHandler::process_event(&event, 640, 480);
        assert!(msg.is_none());
    }

    #[test]
    fn test_scancode_to_evdev_letters() {
        assert_eq!(sdl2_scancode_to_evdev(4), Some(30));   // A
        assert_eq!(sdl2_scancode_to_evdev(29), Some(44));  // Z
        assert_eq!(sdl2_scancode_to_evdev(20), Some(16));  // Q
    }

    #[test]
    fn test_scancode_to_evdev_modifiers() {
        assert_eq!(sdl2_scancode_to_evdev(225), Some(42));  // Left Shift
        assert_eq!(sdl2_scancode_to_evdev(224), Some(29));  // Left Ctrl
        assert_eq!(sdl2_scancode_to_evdev(226), Some(56));  // Left Alt
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
