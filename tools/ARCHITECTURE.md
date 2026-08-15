# TRUEOS-RDP Architecture

## Baseline

The client has five independent paths:

1. **Input data plane** — the existing THID packets go to UDP port 100. Their
   magic, version, device ID, sequence handling, keyboard report, tablet
   report, and startup probe are unchanged.
2. **Media data plane** — `TME1GET1` is unicast to UDP port 9650. TME1
   datagrams are CRC-checked and access units are reassembled in bounded
   memory. Complete Annex-B access units feed one long-lived low-latency
   decoder. The newest decoded RGBA frame replaces a persistent GPU texture.
3. **Presentation** — after the first decoded frame, the decorated winit window
   bypasses the local adapter command interpreter. The receiver hands an owned
   latest image to a dedicated GPU presenter, which updates one retained video
   texture and composites it, the local pointer, and telemetry in one pass.
   Resizing and native maximize preserve the 2560x1440 raster by scaling it
   uniformly and clearing unused space to black. The displayed rectangle is
   also used to map pointer coordinates back to the remote normalized input
   range. A local 4 px white cursor and an 80 px maximum, time-faded trail are
   composed after the video texture so pointer feedback is never dependent on
   remote-frame latency.
4. **Startup UI** — until the first decoded video frame arrives, the local
   wallpaper-backed connection panel edits a shared host target. IP/HOST tab
   changes retarget the TME1 subscriber and THID sender without changing either
   wire format. Autoconnect retries continue in the background.
5. **Telemetry** — a disposable 4 Hz worker samples bounded media counters and
   stamps an opaque 2x `microfont` texture. It cycles headers, four-digit values,
   and units at the requested cadence. A click in its bottom-right bounds is
   consumed locally, stops and joins the worker, drops the retained GPU texture,
   and disables the panel for the remainder of the process. `ping` is a
   best-effort mean UDP arrival interval: TME1 has no echo or sender timestamp
   from which a real round-trip time could be calculated.

There is no remote scene graph, resource snapshot, remote shader, resize
recomposition request, or application-drawn window chrome. The connection
panel is entirely local and has no relationship to the retired TCP scene
protocol.

## Current UDP session latch

The selected IP address or hostname makes broadcast discovery unnecessary. The
client:

1. binds UDP port 9650 once;
2. unicasts `TME1GET1` every 500 ms until a valid session arrives;
3. pins the expected server endpoint and session ID;
4. drains the bounded 100-frame session;
5. keeps the same socket and decoder alive, clears the pin, and immediately
   requests the next session.

Keeping the socket alive eliminates the rollover hole created by launching a
new recording process after each bounded session. Transient UDP receive errors
also return to this subscription loop instead of terminating the client media
worker.

## Small TCP control plane

TCP should replace only the UDP subscription/liveness handshake. It should not
revive the former TRDP scene protocol and should not carry video or HID data.

The first useful protocol needs only:

- client hello: protocol version and client UDP media port;
- stream request: start or stop, requested raster and cadence;
- server acceptance: session identifier and actual raster/cadence;
- ping/pong or an equivalent connection-liveness rule;
- explicit rejection with a small reason code.

The TCP connection owns the lifetime of its media subscription. On disconnect,
TRUEOS stops that subscriber. TME1 remains the UDP media wire format and THID
remains the independent UDP input format.

No TCP bytes are sent by the current client because the live kernel still
exposes the old scene protocol on that port. The control protocol should be
added to both sides together, behind a new magic/version, so an old endpoint
cannot misinterpret it.
