# Ubuntu window server

This is the first view-only Ubuntu → TRUEOS stream. A Rust server captures one
chosen X11 window through FFmpeg, sends low-latency baseline H.264 over UDP
43402, and the `rdp` Blueprint feeds TRUEOS's native iGPU video decoder/UI4
video frame. It has no keyboard, mouse, clipboard, audio, or remote-control
protocol. This is a TRUEOS video transport, not Microsoft RDP compatibility.

## Build and local verification

On Ubuntu, install Rust and the capture/runtime packages:

```sh
sudo apt install ffmpeg x11-utils
cd /path/to/TRUEOS-RDP/server
cargo test --workspace
cargo build --release
python3 tools/smoke.py
```

The smoke check uses a synthetic pattern and loopback UDP. It validates packet
CRC/reassembly, decodes received pictures with FFprobe, and checks that a second
subscriber cannot replace an active one. It needs no display or bare-metal rig.
The independent Cargo workspace leaves the existing desktop RDP viewer alone.

## Choose an Ubuntu window and stream it

Use an **Ubuntu on Xorg** login for the first test. In a terminal inside that
desktop session, run `xwininfo`, click the desired window, and copy its window
ID. Then run, for example:

```sh
./target/release/trueos-rdp-server --window 0x04600007
```

The default output is 1280×720 at 30 fps, padded to preserve the source aspect
ratio. Choose another fixed even size/rate with `--width`, `--height`, `--fps`.
Encoding runs on the Ubuntu CPU using libx264; decoding/display on TRUEOS uses
the existing media engine. To test the network without window capture:

```sh
./target/release/trueos-rdp-server --test-source
```

Open the `rdp` Blueprint in TRUEOS, enter the Ubuntu machine's IP address, and
connect. Its default port is 43402. The server binds `0.0.0.0:43402`; override
with `--bind 192.168.1.20:43402` or an IPv6 socket address if needed. Keepalives
maintain the single subscriber; after three seconds without one the slot is
released. A viewer joins on an IDR with SPS/PPS, within about one keyframe
interval (one second). After packet loss the client waits for another IDR.

If Ubuntu's firewall is enabled, permit UDP 43402 from the TRUEOS machine on
your LAN. The transport is unauthenticated and unencrypted: use a trusted LAN
or an existing VPN and avoid forwarding the UDP port onto the Internet.

## Capture limits for this first version

- Native Wayland window capture is not implemented. XWayland windows can work,
  but an Xorg session is the straightforward first target. No PipeWire portal
  integration is included yet.
- X11 capture follows the chosen window's pixels; visibility/occlusion depends
  on the X server/compositor. Keep the window visible and unminimized.
- The source window size is fixed when FFmpeg opens it. Restart the server
  after resizing or reopening the source window. X11 window IDs change when
  windows are recreated; closing the source ends the FFmpeg process/server.
- Only one viewer is supported. Restarting/reconnecting establishes a fresh
  stream session; frame output stays at the configured size.

## Optional user service

The supplied unit is a template for a desktop **user** service, after a manual
test works. It is not installed or enabled by this change. Install the release
binary at `~/.local/bin/trueos-rdp-server` and the unit from
`ubuntu/trueos-rdp-server.service` under `~/.config/systemd/user/`. Set
`~/.config/trueos-rdp-server/environment` to the actual desktop display/window:

```ini
DISPLAY=:0
TRUEOS_RDP_WINDOW=0x04600007
```

Import `XAUTHORITY` into the systemd user manager if your Xorg session requires
it (`systemctl --user import-environment DISPLAY XAUTHORITY`), then reload/start
the user service. Its output appears in
`journalctl --user -u trueos-rdp-server -f`. Update the window ID and restart the
service when the application creates a new window. This service requires the
logged-in desktop session and is not a headless login server.

## Wire format

Clients send the exact eight-byte `TME1GET1` subscription at least once per
second to the server port. The source UDP address is the return endpoint.
Keepalives from the active endpoint receive `TME1INFO` metadata (20 bytes):
8-byte magic; big-endian session `u32`, width `u16`, height `u16`, fps `u16`, and
reserved `u16` zero. Video uses the existing TME1 v1 shape: 32-byte header,
CRC32 of the payload, fragmented Annex-B access units, maximum UDP payload
1200 bytes. The `protocol` crate shares the parser and metadata with the
Blueprint. SPS/PPS are repeated at every one-second IDR. All allocation and
fragment counts are bounded by the transport's maximum access-unit size.
