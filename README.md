# TRUEOS RDP over SSH

For the opposite direction—one Ubuntu window streamed into a native TRUEOS
UI4 video frame—see the new [view-only Ubuntu server](server/README.md) and
the `rdp` Blueprint in TRUEOS-Blueprints. The server is an independent Rust
package under `server/`, using UDP 43402.

The desktop client opens one OpenSSH subsystem session, `trueos-rdp`, on port
22. Input and video share that authenticated, encrypted session in both View and
NoView mode. `--no-view` omits video; it does not select a different input or login
path. There is no UDP input or unauthenticated video subscription fallback.

Use `trueos-rdp t4ce@192.168.178.94` to select the SSH account explicitly.
Otherwise the client uses `$USER`/`$USERNAME` (or `t4ce` when neither is set).
OpenSSH uses the existing SSH config, agent and keys. Host-key verification is
required: establish the trusted rig host key with ordinary SSH first if needed.
The kernel controls account proof, including its current development policy;
the client adds no admission bypass and stores no separate RDP credential.
OpenSSH errors are preserved in the input log, including when StartCenter hides
stderr. When launched from a terminal, existing key/Cry prompts use OpenSSH's
normal terminal/askpass behavior.

On Ubuntu, install the runtime dependencies before launching:

```sh
sudo apt install openssh-client ffmpeg libxkbcommon-x11-0
```

FFmpeg decodes video; `libxkbcommon-x11-0` is required by the X11 window backend,
including in NoView mode.

StartCenter launches `target/release/trueos-rdp -NoView`.

Ctrl+Space toggles local center-snap. Space press, repeat, and release are
consumed by the shortcut; the combo is not forwarded to TRUEOS. Ctrl still
works normally with other keys.

The default capture mode hides the local pointer, shows a centered marker in
the input-only window, and sends differences between successive window cursor
positions using the existing relative tablet protocol. It does not grab or
warp the compositor pointer. This supports absolute input from nested RDP.
The first position after capture/entry is only a baseline, avoiding a jump.
Motion is limited to positions delivered inside the window; this is not an
unlimited relative mouse lock. Ctrl+Space releases the visual snap.

In the laptop → Remmina → Ubuntu → TRUEOS chain, the laptop pointer remains
owned by the outer Remmina session. Inner-window confinement does not capture
that pointer or create unlimited relative motion. Native Wayland confinement
was tested: packets continued, but it did not solve the outer-pointer limit,
so that experiment was removed. True relative capture needs support at the
outer client and along the input transport.

For a physical mouse with genuine relative input, launch with `--raw-capture`
to use compositor capture and winit raw mouse deltas. Do not infer relative
input just from `DeviceEvent::MouseMotion`: winit 0.30.13's X11 backend assumes
XInput raw valuators are relative, including the absolute positions observed
in GNOME/Remmina. In live testing, the grab also stopped further motion.

Input diagnostics are written to `$XDG_STATE_HOME/trueos-rdp/input.log`, or
`~/.local/state/trueos-rdp/input.log`. This works even though StartCenter
discards stderr. Each record includes the process ID. Inspect them with:

```sh
tail -f ~/.local/state/trueos-rdp/input.log
```

`Ctrl+Space consumed locally` confirms the shortcut. `input capture=true`
counts incoming raw and cursor events once per second, including zero-event
intervals. `SSH RDP ready` confirms subsystem admission; `SSH input not sent` records
closed, unready or congested sessions. No input is replayed into a new session.

Build and verify with `cargo test` and `cargo build --release`, then relaunch
the RDP tool from StartCenter to load the rebuilt executable.

To measure the SSH receiver and ffmpeg decoder without a graphical window:

```sh
cargo run --release --example media-probe -- 192.168.178.94 40
```

The probe opens the same authenticated SSH subsystem and prints cumulative
access units, decoded frames, datagrams, and receiver anomalies once per second.
The rate counts encoded media payload in Mbps, excluding SSH framing. Anomalies include
gaps, reordered or duplicate datagrams, and reassembly skips; they are not an
exact packet-loss count. Run a single viewer at a time when comparing active samples. Media records
are bounded independently of the input queue so a slow decoder cannot grow
unlimited session buffering.

On older Intel Haswell GPUs (for example HD Graphics 4600), Mesa may report
Vulkan conformance version `0.0.0.0`. wgpu hides these adapters by default,
even when `vulkaninfo` detects them. To test the viewer with such an adapter,
build the updated executable and launch it directly from the repository:

```sh
WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1 ./target/release/trueos-rdp
```

This opts into an incompletely supported driver; rendering may still fail or
show artifacts. If startup fails, capture the terminal's
`adapterlibgfx renderer startup error` message.

The Linux build includes Vulkan and native OpenGL/GLES. Both are considered
by default, so a GPU hidden by Vulkan can still be selected through OpenGL.
The OpenGL path uses presentation-only limits and draws uploaded frames into
the window rather than requiring direct swapchain texture uploads. To force
OpenGL when diagnosing driver problems:

```sh
WGPU_BACKEND=gl ./target/release/trueos-rdp
```

The terminal's `adapterlibgfx desktop selected` line identifies the selected
backend and GPU. OpenGL uses the system EGL/GL driver; no browser is bundled.
