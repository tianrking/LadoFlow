//! Windows input injection for the selected `LadoFlow` capture source.

#![allow(unsafe_code)]

use std::{collections::HashSet, mem::size_of, sync::OnceLock};

use ladoflow_protocol::{
    ButtonState, InputEvent, InputEventKind, MAX_TOUCH_CONTACTS, PointerButton, TouchPhase,
};
use windows::Win32::{
    Foundation::{POINT, RECT},
    UI::{
        Input::{
            KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
                KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
                MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
                MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
                MOUSEEVENTF_MOVE_NOCOALESCE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
                MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
                MOUSEINPUT, SendInput, VIRTUAL_KEY,
            },
            Pointer::{
                InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_CANCELED,
                POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP,
                POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_TOUCH_INFO, TOUCH_FEEDBACK_NONE,
            },
        },
        WindowsAndMessaging::{
            GetSystemMetrics, PT_TOUCH, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN, TOUCH_MASK_CONTACTAREA, TOUCH_MASK_ORIENTATION, TOUCH_MASK_PRESSURE,
            XBUTTON1, XBUTTON2,
        },
    },
};

use super::{enumerate_monitors, select_monitor};

const NORMALIZED_MOUSE_MAX: i64 = 65_535;
const TOUCH_CONTACT_RADIUS: i32 = 2;
const TOUCH_PRESSURE_MAX: u32 = 1_024;

static TOUCH_INITIALIZATION: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenPoint {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy)]
struct CoordinateMapper {
    source_width: u16,
    source_height: u16,
    target: RECT,
    virtual_desktop: RECT,
}

impl CoordinateMapper {
    fn new(
        source_width: u16,
        source_height: u16,
        target: RECT,
        virtual_desktop: RECT,
    ) -> Result<Self, String> {
        if source_width == 0 || source_height == 0 {
            return Err("input source dimensions must be non-zero".to_owned());
        }
        validate_rect(target, "selected Windows display")?;
        validate_rect(virtual_desktop, "Windows virtual desktop")?;
        Ok(Self {
            source_width,
            source_height,
            target,
            virtual_desktop,
        })
    }

    fn screen_point(self, x: u16, y: u16) -> Result<ScreenPoint, String> {
        Ok(ScreenPoint {
            x: scale_coordinate(
                x,
                self.source_width,
                self.target.left,
                self.target.right,
                "x",
            )?,
            y: scale_coordinate(
                y,
                self.source_height,
                self.target.top,
                self.target.bottom,
                "y",
            )?,
        })
    }

    fn normalized_mouse(self, point: ScreenPoint) -> Result<ScreenPoint, String> {
        Ok(ScreenPoint {
            x: normalize_virtual_coordinate(
                point.x,
                self.virtual_desktop.left,
                self.virtual_desktop.right,
            )?,
            y: normalize_virtual_coordinate(
                point.y,
                self.virtual_desktop.top,
                self.virtual_desktop.bottom,
            )?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveTouch {
    point: ScreenPoint,
    pressure: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyMapping {
    virtual_key: u16,
    extended: bool,
}

/// Owns remote input state for one negotiated Windows display session.
pub struct NativeInputController {
    mapper: CoordinateMapper,
    pressed_buttons: HashSet<PointerButton>,
    pressed_keys: HashSet<u16>,
    touches: [Option<ActiveTouch>; MAX_TOUCH_CONTACTS as usize],
}

impl NativeInputController {
    pub fn new(
        display_id: Option<&str>,
        stream_width: u16,
        stream_height: u16,
    ) -> Result<Self, String> {
        let monitors = enumerate_monitors()?;
        let selected = select_monitor(&monitors, display_id)?;
        let mapper = CoordinateMapper::new(
            stream_width,
            stream_height,
            selected.rect,
            virtual_desktop_rect()?,
        )?;
        Ok(Self {
            mapper,
            pressed_buttons: HashSet::new(),
            pressed_keys: HashSet::new(),
            touches: [None; MAX_TOUCH_CONTACTS as usize],
        })
    }

    pub fn inject(&mut self, event: InputEvent) -> Result<(), String> {
        match event.kind() {
            InputEventKind::PointerMove { x, y } => self.pointer_move(x, y),
            InputEventKind::PointerButton { button, state } => self.pointer_button(button, state),
            InputEventKind::Wheel { delta_x, delta_y } => Self::wheel(delta_x, delta_y),
            InputEventKind::Key { usage, state, .. } => self.key(usage, state),
            InputEventKind::Touch {
                contact_id,
                phase,
                x,
                y,
                pressure,
            } => self.touch(contact_id, phase, x, y, pressure),
            InputEventKind::Focus { focused: true } => Ok(()),
            InputEventKind::Focus { focused: false } => self.release_all(),
        }
    }

    fn pointer_move(&self, x: u16, y: u16) -> Result<(), String> {
        let point = self
            .mapper
            .normalized_mouse(self.mapper.screen_point(x, y)?)?;
        send_mouse(MOUSEINPUT {
            dx: point.x,
            dy: point.y,
            dwFlags: MOUSEEVENTF_MOVE
                | MOUSEEVENTF_MOVE_NOCOALESCE
                | MOUSEEVENTF_ABSOLUTE
                | MOUSEEVENTF_VIRTUALDESK,
            ..MOUSEINPUT::default()
        })
    }

    fn pointer_button(&mut self, button: PointerButton, state: ButtonState) -> Result<(), String> {
        let pressed = state == ButtonState::Pressed;
        if self.pressed_buttons.contains(&button) == pressed {
            return Ok(());
        }
        let (down, up, data) = mouse_button_mapping(button);
        send_mouse(MOUSEINPUT {
            mouseData: data,
            dwFlags: if pressed { down } else { up },
            ..MOUSEINPUT::default()
        })?;
        if pressed {
            self.pressed_buttons.insert(button);
        } else {
            self.pressed_buttons.remove(&button);
        }
        Ok(())
    }

    fn wheel(delta_x: i16, delta_y: i16) -> Result<(), String> {
        let mut inputs = Vec::with_capacity(2);
        if delta_y != 0 {
            inputs.push(mouse_input(MOUSEINPUT {
                mouseData: u32::from_ne_bytes(i32::from(delta_y).to_ne_bytes()),
                dwFlags: MOUSEEVENTF_WHEEL,
                ..MOUSEINPUT::default()
            }));
        }
        if delta_x != 0 {
            inputs.push(mouse_input(MOUSEINPUT {
                mouseData: u32::from_ne_bytes(i32::from(delta_x).to_ne_bytes()),
                dwFlags: MOUSEEVENTF_HWHEEL,
                ..MOUSEINPUT::default()
            }));
        }
        send_inputs(&inputs)
    }

    fn key(&mut self, usage: u16, state: ButtonState) -> Result<(), String> {
        let mapping = hid_usage_to_key(usage)
            .ok_or_else(|| format!("unsupported USB HID keyboard usage 0x{usage:04x}"))?;
        let pressed = state == ButtonState::Pressed;
        if self.pressed_keys.contains(&usage) == pressed {
            return Ok(());
        }
        send_key(mapping, state)?;
        if pressed {
            self.pressed_keys.insert(usage);
        } else {
            self.pressed_keys.remove(&usage);
        }
        Ok(())
    }

    fn touch(
        &mut self,
        contact_id: u8,
        phase: TouchPhase,
        x: u16,
        y: u16,
        pressure: u16,
    ) -> Result<(), String> {
        ensure_touch_initialized()?;
        let index = usize::from(contact_id);
        let contact = ActiveTouch {
            point: self.mapper.screen_point(x, y)?,
            pressure,
        };
        match phase {
            TouchPhase::Begin if self.touches[index].is_some() => {
                return Err(format!("touch contact {contact_id} began twice"));
            }
            TouchPhase::Move | TouchPhase::End | TouchPhase::Cancel
                if self.touches[index].is_none() =>
            {
                return Err(format!("touch contact {contact_id} changed before begin"));
            }
            _ => {}
        }

        let mut next = self.touches;
        next[index] = Some(contact);
        let contacts = touch_frame(
            &next,
            Some((contact_id, phase)),
            self.mapper.virtual_desktop,
        );
        inject_touch_contacts(&contacts)?;
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
            next[index] = None;
        }
        self.touches = next;
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self.cancel_touches() {
            errors.push(error);
        }
        for button in self.pressed_buttons.clone() {
            if let Err(error) = self.pointer_button(button, ButtonState::Released) {
                errors.push(error);
            }
        }
        for usage in self.pressed_keys.clone() {
            if let Err(error) = self.key(usage, ButtonState::Released) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "failed to release remote Windows input state: {}",
                errors.join("; ")
            ))
        }
    }

    fn cancel_touches(&mut self) -> Result<(), String> {
        if self.touches.iter().all(Option::is_none) {
            return Ok(());
        }
        ensure_touch_initialized()?;
        let contacts = touch_frame(&self.touches, None, self.mapper.virtual_desktop);
        inject_touch_contacts(&contacts)?;
        self.touches.fill(None);
        Ok(())
    }
}

impl Drop for NativeInputController {
    fn drop(&mut self) {
        let _released = self.release_all();
    }
}

fn virtual_desktop_rect() -> Result<RECT, String> {
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let right = left
        .checked_add(width)
        .ok_or_else(|| "Windows virtual desktop width overflowed".to_owned())?;
    let bottom = top
        .checked_add(height)
        .ok_or_else(|| "Windows virtual desktop height overflowed".to_owned())?;
    let rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    validate_rect(rect, "Windows virtual desktop")?;
    Ok(rect)
}

fn validate_rect(rect: RECT, name: &str) -> Result<(), String> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        Err(format!("{name} has invalid geometry"))
    } else {
        Ok(())
    }
}

fn scale_coordinate(
    value: u16,
    source_extent: u16,
    target_start: i32,
    target_end: i32,
    axis: &str,
) -> Result<i32, String> {
    if value >= source_extent {
        return Err(format!(
            "remote {axis} coordinate {value} exceeds negotiated extent {source_extent}"
        ));
    }
    let target_extent = i64::from(target_end) - i64::from(target_start);
    if source_extent == 1 || target_extent == 1 {
        return Ok(target_start);
    }
    let scaled = i64::from(value) * (target_extent - 1) / i64::from(source_extent - 1);
    i32::try_from(i64::from(target_start) + scaled)
        .map_err(|_| format!("mapped Windows {axis} coordinate overflowed"))
}

fn normalize_virtual_coordinate(value: i32, start: i32, end: i32) -> Result<i32, String> {
    if value < start || value >= end {
        return Err(format!(
            "mapped input coordinate {value} lies outside Windows virtual desktop {start}..{end}"
        ));
    }
    let extent = i64::from(end) - i64::from(start);
    let normalized = if extent == 1 {
        0
    } else {
        (i64::from(value) - i64::from(start)) * NORMALIZED_MOUSE_MAX / (extent - 1)
    };
    i32::try_from(normalized).map_err(|_| "normalized mouse coordinate overflowed".to_owned())
}

fn mouse_input(mouse: MOUSEINPUT) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: mouse },
    }
}

fn send_mouse(mouse: MOUSEINPUT) -> Result<(), String> {
    send_inputs(&[mouse_input(mouse)])
}

fn send_key(mapping: KeyMapping, state: ButtonState) -> Result<(), String> {
    let mut flags = if mapping.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    if state == ButtonState::Released {
        flags |= KEYEVENTF_KEYUP;
    }
    send_inputs(&[INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(mapping.virtual_key),
                dwFlags: flags,
                ..KEYBDINPUT::default()
            },
        },
    }])
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let input_size = i32::try_from(size_of::<INPUT>())
        .map_err(|_| "Win32 INPUT structure size exceeds i32".to_owned())?;
    // SAFETY: every union is initialized for the matching INPUT type, the
    // slice remains valid for the synchronous call, and `input_size` is exact.
    let sent = unsafe { SendInput(inputs, input_size) };
    if usize::try_from(sent).ok() == Some(inputs.len()) {
        Ok(())
    } else {
        Err(format!(
            "SendInput accepted {sent}/{} events (Windows UIPI or input policy may have blocked injection): {}",
            inputs.len(),
            windows::core::Error::from_win32()
        ))
    }
}

fn mouse_button_mapping(button: PointerButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS, u32) {
    match button {
        PointerButton::Primary => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
        PointerButton::Secondary => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        PointerButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        PointerButton::Back => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
        PointerButton::Forward => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
    }
}

fn ensure_touch_initialized() -> Result<(), String> {
    TOUCH_INITIALIZATION
        .get_or_init(|| {
            // SAFETY: this is process-global initialization guarded by
            // `OnceLock`; the protocol's bound is within Windows' 256-contact
            // maximum.
            unsafe { InitializeTouchInjection(u32::from(MAX_TOUCH_CONTACTS), TOUCH_FEEDBACK_NONE) }
                .map_err(|error| format!("failed to initialize Windows touch injection: {error}"))
        })
        .clone()
}

fn inject_touch_contacts(contacts: &[POINTER_TOUCH_INFO]) -> Result<(), String> {
    // SAFETY: all contacts are fully initialized, use screen coordinates inside
    // the virtual desktop, and remain live for the synchronous call.
    unsafe { InjectTouchInput(contacts) }
        .map_err(|error| format!("failed to inject Windows touch frame: {error}"))
}

fn touch_frame(
    touches: &[Option<ActiveTouch>; MAX_TOUCH_CONTACTS as usize],
    changed: Option<(u8, TouchPhase)>,
    bounds: RECT,
) -> Vec<POINTER_TOUCH_INFO> {
    touches
        .iter()
        .enumerate()
        .filter_map(|(index, contact)| {
            contact.map(|contact| {
                let contact_id = u8::try_from(index).expect("touch array index fits u8");
                let flags = changed.map_or(
                    POINTER_FLAG_UP | POINTER_FLAG_CANCELED,
                    |(changed_id, phase)| {
                        if contact_id == changed_id {
                            touch_pointer_flags(phase)
                        } else {
                            POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT
                        }
                    },
                );
                touch_info(contact_id, contact, flags, bounds)
            })
        })
        .collect()
}

fn touch_pointer_flags(phase: TouchPhase) -> POINTER_FLAGS {
    match phase {
        TouchPhase::Begin => POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
        TouchPhase::Move => POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
        TouchPhase::End => POINTER_FLAG_UP,
        TouchPhase::Cancel => POINTER_FLAG_UP | POINTER_FLAG_CANCELED,
    }
}

fn touch_info(
    contact_id: u8,
    contact: ActiveTouch,
    pointer_flags: POINTER_FLAGS,
    bounds: RECT,
) -> POINTER_TOUCH_INFO {
    let point = POINT {
        x: contact.point.x,
        y: contact.point.y,
    };
    let contact_rect = RECT {
        left: point
            .x
            .saturating_sub(TOUCH_CONTACT_RADIUS)
            .max(bounds.left),
        top: point.y.saturating_sub(TOUCH_CONTACT_RADIUS).max(bounds.top),
        right: point
            .x
            .saturating_add(TOUCH_CONTACT_RADIUS)
            .min(bounds.right),
        bottom: point
            .y
            .saturating_add(TOUCH_CONTACT_RADIUS)
            .min(bounds.bottom),
    };
    let pressure = u32::from(contact.pressure) * TOUCH_PRESSURE_MAX / u32::from(u16::MAX);
    let mut info = POINTER_TOUCH_INFO::default();
    info.pointerInfo.pointerType = PT_TOUCH;
    info.pointerInfo.pointerId = u32::from(contact_id) + 1;
    info.pointerInfo.pointerFlags = pointer_flags;
    info.pointerInfo.ptPixelLocation = point;
    info.pointerInfo.ptPixelLocationRaw = point;
    info.touchMask = TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE;
    info.rcContact = contact_rect;
    info.rcContactRaw = contact_rect;
    info.orientation = 90;
    info.pressure = pressure;
    info
}

const fn hid_usage_to_key(usage: u16) -> Option<KeyMapping> {
    let (virtual_key, extended) = match usage {
        0x04..=0x1d => (0x41 + usage - 0x04, false),
        0x1e..=0x26 => (0x31 + usage - 0x1e, false),
        0x27 => (0x30, false),
        0x28 => (0x0d, false),
        0x29 => (0x1b, false),
        0x2a => (0x08, false),
        0x2b => (0x09, false),
        0x2c => (0x20, false),
        0x2d => (0xbd, false),
        0x2e => (0xbb, false),
        0x2f => (0xdb, false),
        0x30 => (0xdd, false),
        0x31 => (0xdc, false),
        0x33 => (0xba, false),
        0x34 => (0xde, false),
        0x35 => (0xc0, false),
        0x36 => (0xbc, false),
        0x37 => (0xbe, false),
        0x38 => (0xbf, false),
        0x39 => (0x14, false),
        0x3a..=0x45 => (0x70 + usage - 0x3a, false),
        0x46 => (0x2c, true),
        0x47 => (0x91, false),
        0x48 => (0x13, false),
        0x49 => (0x2d, true),
        0x4a => (0x24, true),
        0x4b => (0x21, true),
        0x4c => (0x2e, true),
        0x4d => (0x23, true),
        0x4e => (0x22, true),
        0x4f => (0x27, true),
        0x50 => (0x25, true),
        0x51 => (0x28, true),
        0x52 => (0x26, true),
        0x53 => (0x90, true),
        0x54 => (0x6f, true),
        0x55 => (0x6a, false),
        0x56 => (0x6d, false),
        0x57 => (0x6b, false),
        0x58 => (0x0d, true),
        0x59..=0x61 => (0x61 + usage - 0x59, false),
        0x62 => (0x60, false),
        0x63 => (0x6e, false),
        0x65 => (0x5d, true),
        0xe0 => (0xa2, false),
        0xe1 => (0xa0, false),
        0xe2 => (0xa4, false),
        0xe3 => (0x5b, true),
        0xe4 => (0xa3, true),
        0xe5 => (0xa1, false),
        0xe6 => (0xa5, true),
        0xe7 => (0x5c, true),
        _ => return None,
    };
    Some(KeyMapping {
        virtual_key,
        extended,
    })
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use ladoflow_protocol::TouchPhase;
    use windows::Win32::{
        Foundation::{POINT, RECT},
        UI::{
            Input::Pointer::{POINTER_FLAG_CANCELED, POINTER_FLAG_DOWN, POINTER_FLAG_UP},
            WindowsAndMessaging::{GetCursorPos, SetCursorPos},
        },
    };

    use super::{
        ActiveTouch, CoordinateMapper, KeyMapping, NativeInputController, ScreenPoint,
        hid_usage_to_key, touch_frame,
    };

    struct CursorRestore(POINT);

    impl Drop for CursorRestore {
        fn drop(&mut self) {
            let _restored = unsafe { SetCursorPos(self.0.x, self.0.y) };
        }
    }

    #[test]
    fn maps_stream_corners_into_a_negative_origin_monitor_and_virtual_desktop() {
        let mapper = CoordinateMapper::new(
            1_280,
            720,
            RECT {
                left: -1_280,
                top: 0,
                right: 0,
                bottom: 1_024,
            },
            RECT {
                left: -1_280,
                top: 0,
                right: 1_920,
                bottom: 1_080,
            },
        )
        .expect("valid mapper");

        assert_eq!(
            mapper.screen_point(0, 0).expect("top-left"),
            ScreenPoint { x: -1_280, y: 0 }
        );
        assert_eq!(
            mapper.screen_point(1_279, 719).expect("bottom-right"),
            ScreenPoint { x: -1, y: 1_023 }
        );
        assert_eq!(
            mapper
                .normalized_mouse(ScreenPoint { x: -1_280, y: 0 })
                .expect("normalized top-left"),
            ScreenPoint { x: 0, y: 0 }
        );
        assert_eq!(
            mapper
                .normalized_mouse(ScreenPoint { x: 1_919, y: 1_079 })
                .expect("normalized bottom-right"),
            ScreenPoint {
                x: 65_535,
                y: 65_535
            }
        );
        assert!(mapper.screen_point(1_280, 0).is_err());
    }

    #[test]
    fn maps_every_android_keyboard_usage_family_to_a_windows_key() {
        for usage in 0x04..=0x1d {
            assert!(hid_usage_to_key(usage).is_some());
        }
        for usage in 0x1e..=0x31 {
            assert!(hid_usage_to_key(usage).is_some());
        }
        for usage in 0x33..=0x63 {
            assert!(hid_usage_to_key(usage).is_some());
        }
        for usage in 0xe0..=0xe7 {
            assert!(hid_usage_to_key(usage).is_some());
        }
        assert_eq!(
            hid_usage_to_key(0xe4),
            Some(KeyMapping {
                virtual_key: 0xa3,
                extended: true
            })
        );
        assert!(hid_usage_to_key(0).is_none());
    }

    #[test]
    fn touch_frames_preserve_contact_ids_and_cancel_every_active_contact() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 1_920,
            bottom: 1_080,
        };
        let mut touches = [None; ladoflow_protocol::MAX_TOUCH_CONTACTS as usize];
        touches[3] = Some(ActiveTouch {
            point: ScreenPoint { x: 50, y: 60 },
            pressure: u16::MAX,
        });

        let begin = touch_frame(&touches, Some((3, TouchPhase::Begin)), bounds);
        assert_eq!(begin.len(), 1);
        assert_eq!(begin[0].pointerInfo.pointerId, 4);
        assert!(
            begin[0]
                .pointerInfo
                .pointerFlags
                .contains(POINTER_FLAG_DOWN)
        );
        assert_eq!(begin[0].pressure, 1_024);

        let cancel = touch_frame(&touches, None, bounds);
        assert!(
            cancel[0]
                .pointerInfo
                .pointerFlags
                .contains(POINTER_FLAG_UP | POINTER_FLAG_CANCELED)
        );
    }

    #[test]
    #[ignore = "moves and restores the real Windows pointer"]
    fn native_pointer_injection_reaches_the_selected_monitor() {
        let mut original = POINT::default();
        unsafe { GetCursorPos(&raw mut original) }.expect("read original pointer position");
        let _restore = CursorRestore(original);
        let controller =
            NativeInputController::new(None, 1_280, 720).expect("create input controller");
        let expected = controller
            .mapper
            .screen_point(640, 360)
            .expect("map center point");

        controller
            .pointer_move(640, 360)
            .expect("inject pointer move");
        thread::sleep(Duration::from_millis(20));
        let mut actual = POINT::default();
        unsafe { GetCursorPos(&raw mut actual) }.expect("read injected pointer position");

        assert!((actual.x - expected.x).abs() <= 1);
        assert!((actual.y - expected.y).abs() <= 1);
    }
}
