# Ubuntu ViewOnly server

The Rust server streams one Ubuntu monitor or window to the `rdp` Blueprint.
On GNOME Wayland it uses the desktop sharing portal, PipeWire, and GStreamer.
The X11 capture path uses FFmpeg. The viewer feeds baseline H.264 from UDP
43402 into TRUEOS's native iGPU decoder and UI4 video frame. The protocol
carries video only.

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

## Share a Wayland monitor or window on localhost

Install the portal capture dependencies inside Ubuntu.

```sh
sudo apt install python3-gi gir1.2-gstreamer-1.0 gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-plugins-ugly
```

Run the daemon in the logged-in GNOME session and choose the monitor or window
in the sharing picker.

```sh
./target/release/trueos-rdp-server --portal
```

This mode binds to `127.0.0.1:43402` by default. Use the Blueprint's Linux
receive probe with `--probe 5 127.0.0.1:43402` to verify the stream on the same
host. Use GNOME's sharing indicator or Ctrl+C in the daemon terminal to stop
capture.

## Share a Wayland monitor or window with the TRUEOS rig

On the current bring-up LAN the Ubuntu host is `192.168.178.111` and the
TRUEOS rig is `192.168.178.94`.

```sh
./target/release/trueos-rdp-server --portal --bind 192.168.178.111:43402 --peer 192.168.178.94
```

Approve the source in the picker, then launch `rdp` on the rig. A LAN portal
listener requires an explicit bind address and an allowed viewer IP. The
source-IP filter limits subscription admission on this trusted LAN. UDP
video is unauthenticated and unencrypted.

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

## X11 capture limits

- With `--window`, use Xorg or select an XWayland window.
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
