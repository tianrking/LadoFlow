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
| serial | Stable local host identifier, optional |

The Android manifest intentionally filters only manufacturer and model so a
host upgrade does not make an existing app undiscoverable.

## Byte-stream contract

- Both directions carry the unchanged LDFL v1 stream from
  `docs/protocol.md`; USB adds no private frame header.
- One LDFL frame may span multiple USB writes, and one USB write may contain
  multiple LDFL frames. The Android incremental decoder accepts both cases.
- Host bulk writes must be no larger than 64 KiB. Android reads each accessory
  transfer into a 64 KiB buffer because the framework warns that unread bytes
  from a partially consumed accessory transfer can be discarded.
- Android sends control/input/telemetry frames only. Video is host-to-Android.
- Control queues are bounded and ordered. Media overflow discards the broken
  delta chain and waits for the next frame marked `KEYFRAME`; it never feeds a
  known-incomplete chain to the decoder.

## Lifecycle implemented on Android

1. An AOA attach intent or foreground rescan finds a matching accessory.
2. Existing permission is reused; otherwise Android shows its system USB
   permission dialog through an app-scoped mutable `PendingIntent`.
3. `UsbManager.openAccessory` yields one duplex file descriptor. Android
   duplicates that descriptor before creating independent auto-closing input
   and output streams.
4. A reader coroutine incrementally parses LDFL frames. A writer coroutine
   serializes reverse control/input frames. All queues and decoder buffers are
   finite.
5. Detach closes both streams immediately. Transient I/O failures retry six
   times with 250 ms to 5 s bounded exponential backoff.
6. Moving the app to the background closes the active stream; returning to the
   foreground rescans and reopens it. A user disconnect remains paused until
   retry is requested.

## Evidence boundary

JVM tests cover split/coalesced reads, writer bytes, queue overflow/keyframe
recovery, identity matching, and reconnect timing. Android lint and APK
assembly cover framework API integration. **未实机验证 / Not verified on a
physical Android device.** A real AOA host and Android device are still needed
to validate permission UI, endpoint transfer sizing, detach timing, and
sustained throughput.
