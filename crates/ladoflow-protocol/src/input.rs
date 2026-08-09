use std::ops::BitOr;

use crate::{MessageType, ProtocolError, WirePayload};

const INPUT_HEADER_LEN: usize = 9;
const POINTER_MOVE_LEN: usize = INPUT_HEADER_LEN + 4;
const POINTER_BUTTON_LEN: usize = INPUT_HEADER_LEN + 2;
const WHEEL_LEN: usize = INPUT_HEADER_LEN + 4;
const KEY_LEN: usize = INPUT_HEADER_LEN + 5;
const TOUCH_LEN: usize = INPUT_HEADER_LEN + 8;
const FOCUS_LEN: usize = INPUT_HEADER_LEN + 1;

/// Maximum number of independently tracked touch contacts in version one.
pub const MAX_TOUCH_CONTACTS: u8 = 16;

/// Pressed or released state for a pointer button or keyboard key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ButtonState {
    /// Button or key is no longer held.
    Released = 0,
    /// Button or key is held.
    Pressed = 1,
}

impl TryFrom<u8> for ButtonState {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Released),
            1 => Ok(Self::Pressed),
            _ => Err(ProtocolError::InvalidPayload("unknown button state")),
        }
    }
}

/// Pointer buttons supported by the version-one input subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PointerButton {
    /// Primary, normally left, pointer button.
    Primary = 1,
    /// Secondary, normally right, pointer button.
    Secondary = 2,
    /// Middle pointer button.
    Middle = 3,
    /// Browser or navigation back button.
    Back = 4,
    /// Browser or navigation forward button.
    Forward = 5,
}

impl TryFrom<u8> for PointerButton {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Primary),
            2 => Ok(Self::Secondary),
            3 => Ok(Self::Middle),
            4 => Ok(Self::Back),
            5 => Ok(Self::Forward),
            _ => Err(ProtocolError::InvalidPayload("unknown pointer button")),
        }
    }
}

/// Lifecycle phase for one direct-touch contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TouchPhase {
    /// A new contact began.
    Begin = 1,
    /// An existing contact moved or changed pressure.
    Move = 2,
    /// A contact ended normally.
    End = 3,
    /// A contact was cancelled and must not generate a click.
    Cancel = 4,
}

impl TryFrom<u8> for TouchPhase {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Begin),
            2 => Ok(Self::Move),
            3 => Ok(Self::End),
            4 => Ok(Self::Cancel),
            _ => Err(ProtocolError::InvalidPayload("unknown touch phase")),
        }
    }
}

/// Snapshot of keyboard modifiers accompanying a key transition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KeyModifiers(u16);

impl KeyModifiers {
    /// Shift modifier.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control modifier.
    pub const CONTROL: Self = Self(1 << 1);
    /// Alt/Option modifier.
    pub const ALT: Self = Self(1 << 2);
    /// Meta/Command/Windows modifier.
    pub const META: Self = Self(1 << 3);
    /// Caps Lock is active.
    pub const CAPS_LOCK: Self = Self(1 << 4);
    /// Num Lock is active.
    pub const NUM_LOCK: Self = Self(1 << 5);

    const KNOWN_MASK: u16 = Self::SHIFT.0
        | Self::CONTROL.0
        | Self::ALT.0
        | Self::META.0
        | Self::CAPS_LOCK.0
        | Self::NUM_LOCK.0;

    /// Decode a modifier mask after rejecting unknown version-one bits.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when an unknown bit is set.
    pub fn from_bits(bits: u16) -> Result<Self, ProtocolError> {
        if bits & !Self::KNOWN_MASK == 0 {
            Ok(Self(bits))
        } else {
            Err(ProtocolError::InvalidPayload("unknown keyboard modifier"))
        }
    }

    /// Numeric modifier mask used on the wire.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every modifier in `other` is present.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for KeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Practical, bounded input event variants supported by protocol version one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEventKind {
    /// Move the pointer to an absolute pixel coordinate in the configured display.
    PointerMove {
        /// Horizontal pixel coordinate.
        x: u16,
        /// Vertical pixel coordinate.
        y: u16,
    },
    /// Change one pointer button.
    PointerButton {
        /// Button that changed.
        button: PointerButton,
        /// New button state.
        state: ButtonState,
    },
    /// Scroll by signed horizontal and vertical wheel units.
    Wheel {
        /// Horizontal wheel delta.
        delta_x: i16,
        /// Vertical wheel delta.
        delta_y: i16,
    },
    /// Change one USB HID keyboard usage.
    Key {
        /// Non-zero USB HID usage identifier.
        usage: u16,
        /// New key state.
        state: ButtonState,
        /// Complete modifier snapshot after this transition.
        modifiers: KeyModifiers,
    },
    /// Update one direct-touch contact.
    Touch {
        /// Contact slot in the range `0..MAX_TOUCH_CONTACTS`.
        contact_id: u8,
        /// Contact lifecycle phase.
        phase: TouchPhase,
        /// Horizontal pixel coordinate.
        x: u16,
        /// Vertical pixel coordinate.
        y: u16,
        /// Normalized pressure from zero through `u16::MAX`.
        pressure: u16,
    },
    /// Notify the host that the remote display gained or lost input focus.
    Focus {
        /// Whether the remote display now owns input focus.
        focused: bool,
    },
}

impl InputEventKind {
    const fn wire_kind(self) -> u8 {
        match self {
            Self::PointerMove { .. } => 1,
            Self::PointerButton { .. } => 2,
            Self::Wheel { .. } => 3,
            Self::Key { .. } => 4,
            Self::Touch { .. } => 5,
            Self::Focus { .. } => 6,
        }
    }

    const fn encoded_len(self) -> usize {
        match self {
            Self::PointerMove { .. } => POINTER_MOVE_LEN,
            Self::PointerButton { .. } => POINTER_BUTTON_LEN,
            Self::Wheel { .. } => WHEEL_LEN,
            Self::Key { .. } => KEY_LEN,
            Self::Touch { .. } => TOUCH_LEN,
            Self::Focus { .. } => FOCUS_LEN,
        }
    }

    fn validate(self) -> Result<(), ProtocolError> {
        match self {
            Self::Key { usage: 0, .. } => Err(ProtocolError::InvalidPayload(
                "keyboard usage must be non-zero",
            )),
            Self::Touch { contact_id, .. } if contact_id >= MAX_TOUCH_CONTACTS => Err(
                ProtocolError::InvalidPayload("touch contact identifier is out of range"),
            ),
            _ => Ok(()),
        }
    }
}

/// Timestamped input payload. The enclosing frame sequence is its event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputEvent {
    timestamp_micros: u64,
    kind: InputEventKind,
}

impl InputEvent {
    /// Construct a validated input event.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] for a reserved keyboard usage
    /// or an out-of-range touch contact identifier.
    pub fn new(timestamp_micros: u64, kind: InputEventKind) -> Result<Self, ProtocolError> {
        kind.validate()?;
        Ok(Self {
            timestamp_micros,
            kind,
        })
    }

    /// Event creation time in the sender's monotonic clock domain.
    #[must_use]
    pub const fn timestamp_micros(self) -> u64 {
        self.timestamp_micros
    }

    /// Input transition carried by this payload.
    #[must_use]
    pub const fn kind(self) -> InputEventKind {
        self.kind
    }
}

impl WirePayload for InputEvent {
    const KIND: MessageType = MessageType::Input;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.kind.validate()?;
        let mut payload = Vec::with_capacity(self.kind.encoded_len());
        payload.extend_from_slice(&self.timestamp_micros.to_be_bytes());
        payload.push(self.kind.wire_kind());

        match self.kind {
            InputEventKind::PointerMove { x, y } => {
                payload.extend_from_slice(&x.to_be_bytes());
                payload.extend_from_slice(&y.to_be_bytes());
            }
            InputEventKind::PointerButton { button, state } => {
                payload.push(button as u8);
                payload.push(state as u8);
            }
            InputEventKind::Wheel { delta_x, delta_y } => {
                payload.extend_from_slice(&delta_x.to_be_bytes());
                payload.extend_from_slice(&delta_y.to_be_bytes());
            }
            InputEventKind::Key {
                usage,
                state,
                modifiers,
            } => {
                payload.extend_from_slice(&usage.to_be_bytes());
                payload.push(state as u8);
                payload.extend_from_slice(&modifiers.bits().to_be_bytes());
            }
            InputEventKind::Touch {
                contact_id,
                phase,
                x,
                y,
                pressure,
            } => {
                payload.push(contact_id);
                payload.push(phase as u8);
                payload.extend_from_slice(&x.to_be_bytes());
                payload.extend_from_slice(&y.to_be_bytes());
                payload.extend_from_slice(&pressure.to_be_bytes());
            }
            InputEventKind::Focus { focused } => payload.push(u8::from(focused)),
        }

        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() < INPUT_HEADER_LEN {
            return Err(ProtocolError::InvalidPayload("input payload is truncated"));
        }

        let timestamp_micros = read_u64(payload, 0);
        let kind = match payload[8] {
            1 => {
                require_len(payload, POINTER_MOVE_LEN)?;
                InputEventKind::PointerMove {
                    x: read_u16(payload, 9),
                    y: read_u16(payload, 11),
                }
            }
            2 => {
                require_len(payload, POINTER_BUTTON_LEN)?;
                InputEventKind::PointerButton {
                    button: PointerButton::try_from(payload[9])?,
                    state: ButtonState::try_from(payload[10])?,
                }
            }
            3 => {
                require_len(payload, WHEEL_LEN)?;
                InputEventKind::Wheel {
                    delta_x: read_i16(payload, 9),
                    delta_y: read_i16(payload, 11),
                }
            }
            4 => {
                require_len(payload, KEY_LEN)?;
                InputEventKind::Key {
                    usage: read_u16(payload, 9),
                    state: ButtonState::try_from(payload[11])?,
                    modifiers: KeyModifiers::from_bits(read_u16(payload, 12))?,
                }
            }
            5 => {
                require_len(payload, TOUCH_LEN)?;
                InputEventKind::Touch {
                    contact_id: payload[9],
                    phase: TouchPhase::try_from(payload[10])?,
                    x: read_u16(payload, 11),
                    y: read_u16(payload, 13),
                    pressure: read_u16(payload, 15),
                }
            }
            6 => {
                require_len(payload, FOCUS_LEN)?;
                InputEventKind::Focus {
                    focused: decode_bool(payload[9])?,
                }
            }
            _ => return Err(ProtocolError::InvalidPayload("unknown input event kind")),
        };

        Self::new(timestamp_micros, kind)
    }
}

fn require_len(payload: &[u8], expected: usize) -> Result<(), ProtocolError> {
    if payload.len() == expected {
        Ok(())
    } else {
        Err(ProtocolError::InvalidPayload(
            "input event length does not match its kind",
        ))
    }
}

const fn decode_bool(value: u8) -> Result<bool, ProtocolError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::InvalidPayload(
            "boolean input field must be zero or one",
        )),
    }
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
