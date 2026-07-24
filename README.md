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

## Portable Linux bundle

The normal native build adopts the glibc version of the build machine. To
produce an x86_64 bundle that runs on glibc 2.35 or newer:

```sh
./tools/build-portable-linux.sh
```

The script downloads pinned build-only copies of Zig and `cargo-zigbuild` into
the ignored `.codex_tmp` directory, verifies the Zig archive checksum, builds
against the older ABI, checks the executable's imported glibc symbol versions,
and writes:

```text
dist/trueos-rdp-linux-x86_64-glibc2.35.tar.xz
```

The bundle does not ship glibc, Vulkan, or a GPU driver. It uses the
destination machine's graphics stack, which is required for reliable Intel
media/graphics interoperation. In view mode, `ffmpeg` must be installed on the
destination and available in `PATH`; the included launcher reports a direct
installation hint if it is missing. Input-only `--no-view` mode does not
require `ffmpeg`.

## macOS bundle

The viewer also compiles for both supported macOS Rust targets. On a Mac with
Xcode Command Line Tools installed, build a universal Apple Silicon + Intel
application:

```sh
./tools/build-macos.sh
```

This writes:

```text
dist/TRUEOS-RDP.app
dist/trueos-rdp-macos-universal.zip
```

The application targets macOS 11 or newer and is ad-hoc signed by default.
Set `TRUEOS_RDP_CODESIGN_IDENTITY` to an Apple Developer ID identity when
producing a distributable signed build. Notarization remains a separate
release step.

Both macOS architectures can also be cross-compiled with `cargo-zigbuild` and
a user-supplied macOS SDK. `tools/package-macos-cross.sh` turns its universal
Mach-O output into an explicitly unsigned test archive; sign that application
on a Mac before opening it. The native build remains the release path.

View mode expects `ffmpeg` from Apple Silicon Homebrew (`/opt/homebrew/bin`),
Intel Homebrew (`/usr/local/bin`), or the inherited `PATH`. Install it with:

```sh
brew install ffmpeg
```

The default host works when opening the application normally. To provide a
different address:

```sh
open dist/TRUEOS-RDP.app --args 192.168.178.94
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
