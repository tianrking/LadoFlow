# Android input and rotation boundary

The decoder `SurfaceView` attaches an `AndroidInputController`. It translates
Android events into the existing LDFL v1 `InputPayload` layouts; no Android-only
wire events or fields are added.

## Event mapping

| Android input | LDFL v1 event | Delivery class |
| --- | --- | --- |
| direct touch down/up/cancel | touch begin/end/cancel | critical |
| direct touch move | touch move | coalescible |
| mouse hover/move | absolute pointer move | coalescible |
| mouse button press/release | pointer button | critical |
| mouse wheel | wheel | coalescible |
| physical keyboard down/up | USB HID keyboard usage | critical |
| Surface focus change | focus | critical |

Android pointer IDs are allocated into the 16 contact IDs permitted by LDFL
v1. Pressure is clamped to `0..1` and normalized over the protocol's full u16
range. Keyboard repeat-down events are suppressed because the host should own
normal key-repeat behavior after receiving one press and one release. Android
key codes without a USB HID keyboard-page mapping are ignored; text composition
and IME strings require a later protocol version.

The current Android input mask is `POINTER | TOUCH | KEYBOARD`. The session
intersects it with the Host mask before sending each event. The decoder
SurfaceView is focusable, requests focus on an active pointer/touch down, and
forwards physical key down/up through `AndroidInputController.onKeyEvent`.
Physical-device keyboard behavior remains an explicit validation item.

The session controller queues protocol control (32), critical input (64), and
coalescible input (32) before assigning the next sender sequence. Critical
events use a suspending bounded path. Coalescible move/wheel events use the
non-blocking path and may be dropped under backpressure. After selection, all
numbered frames enter the same 64-frame FIFO USB writer.

## Coordinates, resolution, and rotation

Coordinates are fit-center mapped from the Android view to coded pixels. Input
in letterbox bars is ignored on begin; an active contact moving outside is
clamped so its matching end/cancel can still be sent. Pure Kotlin tests cover
letterboxing, all four quarter turns, u16/contact bounds, and landscape-to-
portrait resolution changes.

`SurfaceHolder.surfaceChanged` updates the current local view dimensions. A
portrait/landscape Activity or multi-window resize keeps the same negotiated
coded viewport and Surface lease generation; the mapping recalculates from the
new view bounds. A stale Surface callback from the destroyed Activity is ignored
after a newer lease attaches.

LDFL v1 advertises `DYNAMIC_ROTATION` as a capability but defines no standalone
rotation payload. Therefore the current interoperable mode change is a new
`DisplayConfig` with the new coded width/height followed by fresh Annex-B
SPS/PPS and a frame marked `KEYFRAME`. Android cancels active contacts, resets
the viewport, recreates MediaCodec, and waits for that keyframe. A local
quarter-turn enum exists for Surface layouts that are already rotated, but it
is never serialized as an invented protocol extension.

## Evidence boundary

Coordinate/contact/HID mapping and the negotiated keyboard send/drop paths are
covered by local JVM tests. Android instrumentation dispatches key down/up
through an actual SurfaceView listener and rebuilds/resizes that Surface in
portrait and landscape layouts. Physical multi-touch, mouse, keyboard,
rotation, and remote host injection have not been exercised.

**未实机验证 / Not verified on a physical Android device.**
