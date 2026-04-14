//! Input injection for the dedicated headless Wayland session.
//!
//! The supported production backend is a compositor-scoped virtual keyboard and
//! virtual pointer attached directly to the headless Sway session socket.

use serde::{Deserialize, Serialize};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::tempfile;
use tracing::{debug, info};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_keyboard, wl_pointer, wl_registry, wl_seat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

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

#[derive(Debug, Clone)]
pub enum InputBackend {
    HeadlessWayland {
        runtime_dir: String,
        wayland_display: String,
    },
    Stub,
}

pub struct InputHandler {
    backend: InputBackend,
    command_tx: Option<mpsc::Sender<InputCommand>>,
}

impl InputHandler {
    pub fn new(backend: InputBackend) -> super::Result<Self> {
        match backend {
            InputBackend::HeadlessWayland {
                ref runtime_dir,
                ref wayland_display,
            } => {
                let (command_tx, command_rx) = mpsc::channel();
                let (ready_tx, ready_rx) = mpsc::channel();
                let runtime_dir = runtime_dir.clone();
                let wayland_display = wayland_display.clone();
                let log_display = wayland_display.clone();

                thread::spawn(move || {
                    match HeadlessWaylandInput::connect(&runtime_dir, &wayland_display) {
                        Ok(mut headless) => {
                            let _ = ready_tx.send(Ok(()));
                            while let Ok(command) = command_rx.recv() {
                                let result = match command {
                                    InputCommand::Key(event) => headless.send_key(&event),
                                    InputCommand::Pointer(event) => headless.send_pointer(&event),
                                };

                                if let Err(error) = result {
                                    debug!("Headless input command failed: {}", error);
                                }
                            }
                        }
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                        }
                    }
                });

                match ready_rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(Ok(())) => {
                            info!(
                                "Headless Wayland input backend connected to {}",
                                log_display
                            );
                            Ok(Self {
                                backend,
                            command_tx: Some(command_tx),
                        })
                    }
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(super::PortalError::Portal(
                        "Timed out waiting for the headless input thread to initialize".into(),
                    )),
                }
            }
            InputBackend::Stub => Ok(Self {
                backend,
                command_tx: None,
            }),
        }
    }

    pub fn send_key(&mut self, event: &KeyEvent) -> super::Result<()> {
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(InputCommand::Key(event.clone()))
                .map_err(|_| {
                    super::PortalError::Portal(
                        "Headless input thread is no longer available".into(),
                    )
                })?;
            debug!("Headless input: key {} {:?}", event.keycode, event.state);
            return Ok(());
        }

        // Stub mode
        debug!("Stub: key {} {:?}", event.keycode, event.state);
        Ok(())
    }

    pub fn send_pointer(&mut self, event: &PointerEvent) -> super::Result<()> {
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(InputCommand::Pointer(event.clone()))
                .map_err(|_| {
                    super::PortalError::Portal(
                        "Headless input thread is no longer available".into(),
                    )
                })?;
            debug!("Headless input: pointer {:?}", event);
            return Ok(());
        }

        // Stub mode
        debug!("Stub: pointer {:?}", event);
        Ok(())
    }

    pub fn backend(&self) -> InputBackend {
        self.backend.clone()
    }
}

enum InputCommand {
    Key(KeyEvent),
    Pointer(PointerEvent),
}

struct HeadlessWaylandInput {
    connection: Connection,
    event_queue: EventQueue<WaylandInputState>,
    protocol_state: WaylandInputState,
    _seat: wl_seat::WlSeat,
    _keyboard_manager: ZwpVirtualKeyboardManagerV1,
    _pointer_manager: ZwlrVirtualPointerManagerV1,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    keyboard_state: xkb::State,
    started_at: Instant,
}

impl HeadlessWaylandInput {
    fn connect(runtime_dir: &str, wayland_display: &str) -> super::Result<Self> {
        let socket_path = Path::new(runtime_dir).join(wayland_display);
        let stream = UnixStream::connect(&socket_path).map_err(|e| {
            super::PortalError::Portal(format!(
                "Failed to connect to headless Wayland socket {}: {}",
                socket_path.display(),
                e
            ))
        })?;
        let connection = Connection::from_socket(stream).map_err(|e| {
            super::PortalError::Portal(format!(
                "Failed to create Wayland connection for {}: {:?}",
                socket_path.display(),
                e
            ))
        })?;

        let (globals, mut event_queue) =
            registry_queue_init::<WaylandInputState>(&connection).map_err(|e| {
                super::PortalError::Portal(format!(
                    "Failed to query Wayland globals from {}: {:?}",
                    socket_path.display(),
                    e
                ))
            })?;
        let qh = event_queue.handle();

        let seat_info = globals
            .contents()
            .with_list(|entries| entries.iter().find(|global| global.interface == "wl_seat").cloned())
            .ok_or_else(|| {
                super::PortalError::Portal(
                    "Headless session does not expose a wl_seat for virtual input".to_string(),
                )
            })?;
        let seat_version = seat_info.version.min(7);
        let seat = globals
            .registry()
            .bind(seat_info.name, seat_version, &qh, ());

        let keyboard_manager: ZwpVirtualKeyboardManagerV1 =
            globals.bind(&qh, 1..=1, ()).map_err(|e| {
                super::PortalError::Portal(format!(
                    "Headless session is missing zwp_virtual_keyboard_manager_v1: {:?}",
                    e
                ))
            })?;
        let pointer_manager: ZwlrVirtualPointerManagerV1 =
            globals.bind(&qh, 1..=2, ()).map_err(|e| {
                super::PortalError::Portal(format!(
                    "Headless session is missing zwlr_virtual_pointer_manager_v1: {:?}",
                    e
                ))
            })?;

        let keyboard = keyboard_manager.create_virtual_keyboard(&seat, &qh, ());
        let pointer = pointer_manager.create_virtual_pointer(Some(&seat), &qh, ());

        let mut xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb_context.include_path_reset_defaults();
        let keymap = xkb::Keymap::new_from_names(
            &xkb_context,
            "",
            "",
            "us",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            super::PortalError::Portal("Failed to compile XKB keymap for virtual keyboard".into())
        })?;
        let mut keymap_bytes = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1).into_bytes();
        if !keymap_bytes.ends_with(&[0]) {
            keymap_bytes.push(0);
        }

        let mut keymap_file = tempfile().map_err(super::PortalError::Io)?;
        keymap_file
            .write_all(&keymap_bytes)
            .map_err(super::PortalError::Io)?;
        keymap_file
            .seek(SeekFrom::Start(0))
            .map_err(super::PortalError::Io)?;
        keyboard.keymap(
            1,
            keymap_file.as_fd(),
            u32::try_from(keymap_bytes.len()).unwrap_or(u32::MAX),
        );

        let mut protocol_state = WaylandInputState;
        event_queue.roundtrip(&mut protocol_state).map_err(|e| {
            super::PortalError::Portal(format!(
                "Failed to finish headless Wayland input initialization: {:?}",
                e
            ))
        })?;
        connection.flush().map_err(|e| {
            super::PortalError::Portal(format!(
                "Failed to flush initial Wayland input requests: {:?}",
                e
            ))
        })?;

        Ok(Self {
            connection,
            event_queue,
            protocol_state,
            _seat: seat,
            _keyboard_manager: keyboard_manager,
            _pointer_manager: pointer_manager,
            keyboard,
            pointer,
            keyboard_state: xkb::State::new(&keymap),
            started_at: Instant::now(),
        })
    }

    fn send_key(&mut self, event: &KeyEvent) -> super::Result<()> {
        let timestamp = self.timestamp_ms();
        let keycode = event.keycode.saturating_add(8);
        let key_state = match event.state {
            KeyState::Pressed => wl_keyboard::KeyState::Pressed,
            KeyState::Released => wl_keyboard::KeyState::Released,
        };
        let direction = match event.state {
            KeyState::Pressed => xkb::KeyDirection::Down,
            KeyState::Released => xkb::KeyDirection::Up,
        };

        self.keyboard.key(timestamp, keycode, key_state.into());
        self.keyboard_state
            .update_key(xkb::Keycode::new(keycode), direction);
        self.keyboard.modifiers(
            self.keyboard_state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.keyboard_state.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.keyboard_state.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.keyboard_state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        );
        self.flush_requests()
    }

    fn send_pointer(&mut self, event: &PointerEvent) -> super::Result<()> {
        let timestamp = self.timestamp_ms();

        match event {
            PointerEvent::MotionAbsolute { x, y } => {
                let x = normalized_axis(*x);
                let y = normalized_axis(*y);
                self.pointer.motion_absolute(timestamp, x, y, AXIS_EXTENT, AXIS_EXTENT);
                self.pointer.frame();
            }
            PointerEvent::MotionRelative { dx, dy } => {
                self.pointer.motion(timestamp, f64::from(*dx), f64::from(*dy));
                self.pointer.frame();
            }
            PointerEvent::Button { button, state } => {
                let button_code = linux_pointer_button(*button)?;
                let state = match state {
                    ButtonState::Pressed => wl_pointer::ButtonState::Pressed,
                    ButtonState::Released => wl_pointer::ButtonState::Released,
                };
                self.pointer.button(timestamp, button_code, state);
                self.pointer.frame();
            }
            PointerEvent::Scroll { delta_x, delta_y } => {
                if *delta_x == 0 && *delta_y == 0 {
                    return Ok(());
                }

                self.pointer.axis_source(wl_pointer::AxisSource::Wheel);

                if *delta_x != 0 {
                    let steps = scroll_steps(*delta_x);
                    self.pointer.axis_discrete(
                        timestamp,
                        wl_pointer::Axis::HorizontalScroll,
                        f64::from(*delta_x) / 120.0,
                        steps,
                    );
                }

                if *delta_y != 0 {
                    let steps = scroll_steps(*delta_y);
                    self.pointer.axis_discrete(
                        timestamp,
                        wl_pointer::Axis::VerticalScroll,
                        f64::from(*delta_y) / 120.0,
                        steps,
                    );
                }

                self.pointer.frame();
            }
        }

        self.flush_requests()
    }

    fn timestamp_ms(&self) -> u32 {
        self.started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u32::MAX)) as u32
    }

    fn flush_requests(&mut self) -> super::Result<()> {
        self.connection.flush().map_err(|e| {
            super::PortalError::Portal(format!(
                "Failed to flush headless Wayland input request: {:?}",
                e
            ))
        })?;
        let _ = self.event_queue.dispatch_pending(&mut self.protocol_state);
        Ok(())
    }
}

const AXIS_EXTENT: u32 = 65_535;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

fn normalized_axis(value: f64) -> u32 {
    let scaled = (value.clamp(0.0, 1.0) * f64::from(AXIS_EXTENT)).round();
    scaled as u32
}

fn scroll_steps(delta: i32) -> i32 {
    let steps = delta / 120;
    if steps == 0 {
        delta.signum()
    } else {
        steps
    }
}

fn linux_pointer_button(button: u32) -> super::Result<u32> {
    match button {
        1 => Ok(BTN_LEFT),
        2 => Ok(BTN_MIDDLE),
        3 => Ok(BTN_RIGHT),
        4 => Ok(BTN_SIDE),
        5 => Ok(BTN_EXTRA),
        _ => Err(super::PortalError::Portal(format!(
            "Unsupported pointer button: {}",
            button
        ))),
    }
}

struct WaylandInputState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandInputState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(WaylandInputState: ignore wl_seat::WlSeat);
delegate_noop!(WaylandInputState: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(WaylandInputState: ignore ZwpVirtualKeyboardV1);
delegate_noop!(WaylandInputState: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(WaylandInputState: ignore ZwlrVirtualPointerV1);

pub mod keycodes {
    #[allow(dead_code)]
    pub const KEY_ENTER: u32 = 28;
    #[allow(dead_code)]
    pub const KEY_ESC: u32 = 1;
    #[allow(dead_code)]
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
