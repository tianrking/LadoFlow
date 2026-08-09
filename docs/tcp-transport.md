# LDFL TCP transport boundary

LadoFlow uses one TCP byte stream as the driver-independent wired fallback for
Android USB tethering and, later, as the data plane for trusted LAN sessions.
It does not change LDFL framing: the stream is a concatenation of complete LDFL
frames, and channel identity is derived from each frame's message type.

The shared `TcpPacketTransport` now accepts an already connected and
authenticated `TcpStream`. It enables `TCP_NODELAY`, moves socket I/O to a
cancellable nonblocking worker, caps each read/write turn at 64 KiB, and bridges
the socket to the same bounded control/media queues used by USB. The worker:

- restores global LDFL sequence order before writing control and media;
- incrementally decodes arbitrary TCP segmentation and coalescing;
- stops reading while an inbound queue is full so TCP applies backpressure;
- rejects malformed, trailing, wrong-channel, and duplicate-sequence frames;
- reports byte/frame counters and the terminal error;
- closes promptly without waiting on a blocking network call.

Unit tests cover split/coalesced framing, bidirectional control/media delivery,
counter convergence, clean shutdown, and malformed-stream disconnection over
real loopback TCP sockets. This proves the local transport primitive, not a
Windows-to-Android connection.

## Connection ownership

The Android display will listen; the desktop host will connect. That direction
fits USB tethering because Android is normally the tether gateway and avoids
opening a Windows listener or adding an inbound firewall rule. Discovery must
only consider an explicitly selected address or a route proven to belong to a
USB-tether interface; LadoFlow must not scan arbitrary LAN gateways in the
background.

Discovery, consent, and authentication remain outside `TcpPacketTransport`.
Before handing the socket to the worker, the platform composition layer must:

1. obtain explicit user intent on both devices;
2. connect to the agreed endpoint with a bounded timeout;
3. authenticate a versioned pairing preface and bind it to both fresh nonces;
4. reject reflection, role mismatch, replay, and trailing preface bytes;
5. only then begin the existing LDFL Hello/Capabilities/DisplayConfig exchange.

The tether link can be local and isolated, but that is not an authentication
guarantee. A raw TCP socket is therefore never presented as a release-ready
session. LAN support must add encrypted transport after pairing; it must not
silently reuse an unauthenticated tether-only assumption.

## Remaining product work

- fix the pairing-preface bytes, port, token representation, and expiry;
- implement the Android listener as a foreground, user-visible display action;
- implement bounded Windows USB-tether route discovery plus manual address entry;
- integrate TCP selection, status, cancellation, and reconnect into the desktop UI;
- prove Windows-to-Android LDFL negotiation over a physical tether cable;
- record sustained bitrate, frame pacing, latency, cable removal, and recovery;
- add authenticated encryption before general LAN discovery is enabled.
