# TRUEOS Remote Desktop

TRUEOS-RDP is a deliberately small remote-display client:

- a normal decorated, resizable OS window;
- one aspect-fitted 1920x1088 video surface with black letterboxing;
- the existing byte-for-byte THID UDP keyboard/tablet sender;
- the proven TME1 UDP H.264 receiver, feeding a live decoder instead of a file.

The former TCP scene/resource interpreter and its custom startup graphics are
not part of the viewer anymore.

## Run

The host needs `ffmpeg` in `PATH`.

```sh
cargo run --release -- 192.168.178.94
```

The default host is `192.168.178.94`, so the address may be omitted. Legacy
`HOST:100` arguments remain accepted; the client derives the fixed service
ports from the host.

Input-only mode is unchanged:

```sh
cargo run --release -- 192.168.178.94 --no-view
```

## Ports

| Port | Transport | Direction | Purpose |
|---|---|---|---|
| 100 | UDP | client to TRUEOS | Existing THID keyboard/tablet input |
| 9650 | UDP | both for subscription, then TRUEOS to client | TME1 H.264 media |
| 100 | TCP | reserved for the next control-plane baseline | Session negotiation and health only |

The current media handshake unicasts `TME1GET1` to the selected host. The
client keeps one bound media socket alive, validates/reassembles every access
unit, and immediately re-subscribes when TRUEOS ends its bounded 100-frame
session. Nothing is saved unless a separate recording tool is used.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the control-plane boundary.
