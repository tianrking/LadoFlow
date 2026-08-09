# Android Open Accessory handoff

The production USB direction is fixed: the Windows, macOS, or Linux computer
is the USB host; Android remains the USB device and exposes the app through
Android Open Accessory 2.0. ADB, developer options, and Android USB-host mode
are not part of the product connection path.

## Accessory identity required from the PC host

The host-side AOA negotiation must publish these exact strings so Android can
route the attach intent to LadoFlow:

| AOA field | Required value |
| --- | --- |
| manufacturer | `LadoFlow` |
| model | `LadoFlow Host` |
| description | User-visible host name, recommended |
| version | Host implementation version, recommended |
| URI | `https://github.com/tianrking/LadoFlow`, recommended |
| serial | Stable local host identifier, optional |

The Android manifest intentionally filters only manufacturer and model so a
host upgrade does not make an existing app undiscoverable.

## Byte-stream contract

- Both directions carry the unchanged LDFL v1 stream from
  `docs/protocol.md`; USB adds no private frame header.
- One LDFL frame may span multiple USB writes, and one USB write may contain
  multiple LDFL frames. The Android incremental decoder accepts both cases.
- Host bulk writes must be no larger than 64 KiB. Android reads into a 64 KiB
  application buffer because the framework warns that unread bytes from a
  partially consumed accessory transfer can be discarded. This is a stream
  chunk contract, not the USB endpoint's negotiated max-packet size.
- Android sends control/input/telemetry frames only. Video is host-to-Android.
- The receive path validates one strictly increasing sender sequence across
  control and media before dispatch. Its ordered queue is bounded to eight
  decoded frames and applies backpressure rather than changing wire order. EOF
  with a partial/trailing LDFL frame is a protocol failure.
- Android selects outbound protocol control (32), critical input (64), and
  coalescible input (32) from separate bounded payload queues. It assigns the
  next global sequence only after that priority decision, then hands the frame
  to one 64-frame FIFO USB writer. Priority can therefore never reorder frames
  that already have sequence numbers.
- The pre-Surface queue is bounded to three. The decoder has a separate single
  eight-access-unit bound spanning Handler admission, pending codec input, and
  input already queued into MediaCodec. Overflow drops the dependent delta
  chain and waits for a new LDFL frame marked `KEYFRAME`; it never feeds a
  known-incomplete chain to MediaCodec.

## Lifecycle implemented on Android

1. An AOA attach intent or foreground rescan finds a matching accessory.
2. Existing permission is reused; otherwise Android shows its system USB
   permission dialog through an app-scoped mutable `PendingIntent`.
3. `UsbManager.openAccessory` yields one duplex `ParcelFileDescriptor`. Android
   duplicates that descriptor before creating independent auto-closing input
   and output streams. Because Android is the accessory/device here, the app
   does not use the host-side `UsbDeviceConnection` or `UsbRequest` APIs.
4. A reader coroutine incrementally parses LDFL frames. A writer coroutine
   serializes reverse control/input frames. All queues and decoder buffers are
   finite.
5. Accessory `InputStream.read` is intentionally blocking and has no synthetic
   per-read timeout. Detach, backgrounding, user disconnect, or session failure
   closes both duplicated descriptors; descriptor close is the cancellation
   mechanism that releases a blocked read. Transient I/O failures retry six
   times with 250 ms to 5 s bounded exponential backoff.
6. Moving the app to the background closes the active stream; returning to the
   foreground rescans and reopens it. A user disconnect remains paused until
   retry is requested.
7. A physical detach closes the descriptors and publishes a distinct
   `Detached` state instead of pretending the app is merely waiting. A later
   attach in the same process opens a new descriptor and starts a fresh LDFL
   generation: both peers may begin their independent sender sequences at
   `Hello/0` again. No reconnect marker or private USB header is added.
8. The transport is owned by the application/process lifecycle, not an
   Activity. Activity recreation does not intentionally close the descriptor;
   process backgrounding still does.

## Evidence boundary

JVM tests cover split/coalesced reads, global control/media wire order,
duplicate/stale sequence rejection, priority-before-sequence assignment,
blocking-read close, queue overflow/keyframe recovery, identity matching,
detach state propagation, reconnect timing, and a fresh handshake/sequence
generation after in-process recovery. Android lint and instrumentation cover
framework and Activity lifecycle integration.

**未实机验证 / Not verified on a physical Android device.**

A real AOA host and Android device are still needed to validate permission UI,
endpoint transfer sizing, descriptor-close timing, and sustained throughput.
