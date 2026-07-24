# TRUEOS-RDP Architecture

## Baseline

The client has three independent paths:

1. **Input data plane** — the existing THID packets go to UDP port 100. Their
   magic, version, device ID, sequence handling, keyboard report, tablet
   report, and startup probe are unchanged.
2. **Media data plane** — `TME1GET1` is unicast to UDP port 9650. TME1
   datagrams are CRC-checked and access units are reassembled in bounded
   memory. Complete Annex-B access units feed one long-lived low-latency
   decoder. The newest decoded RGBA frame replaces a persistent GPU texture.
3. **Presentation** — a decorated winit window draws only that texture.
   Resizing and native maximize preserve the 1920x1088 raster by scaling it
   uniformly and clearing unused space to black. The displayed rectangle is
   also used to map pointer coordinates back to the remote normalized input
   range.

There is no remote scene graph, resource snapshot, remote shader, resize
recomposition request, custom connection panel, or application-drawn window
chrome.

## Current UDP session latch

The selected IP address makes broadcast discovery unnecessary. The client:

1. binds UDP port 9650 once;
2. unicasts `TME1GET1` every 500 ms until a valid session arrives;
3. pins the expected server endpoint and session ID;
4. drains the bounded 100-frame session;
5. keeps the same socket and decoder alive, clears the pin, and immediately
   requests the next session.

Keeping the socket alive eliminates the rollover hole created by launching a
new recording process after each bounded session.

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
