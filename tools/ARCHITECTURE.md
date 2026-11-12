# TRUEOS-RDP Architecture

## Baseline

The client has six independent paths:

1. **Input data plane** — existing THID records are framed inside the
   authenticated `trueos-rdp` SSH subsystem. Keyboard, absolute/relative tablet
   motion, buttons, wheel and pointer-mode control retain their wire format.
   Input is admitted only after the server's Ready record, and pending input
   is discarded on session changes.
2. **Media data plane** — TME1 records share the same encrypted SSH connection.
   Their CRC and bounded access-unit reassembly remain unchanged. Complete
   Annex-B access units feed one long-lived low-latency decoder. The newest
   decoded RGBA frame replaces a persistent GPU texture. NoView opens the same
   subsystem with viewing disabled; input authentication remains identical.
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
   changes terminate the old SSH session and reconnect both paths together
   without changing the inner wire formats. Autoconnect retries continue in the background.
5. **Telemetry** — a disposable 4 Hz worker samples bounded media counters and
   stamps an opaque 2x `microfont` texture. It cycles headers, four-digit values,
   and units at the requested cadence. A click in its bottom-right bounds is
   consumed locally, stops and joins the worker, drops the retained GPU texture,
   and disables the panel for the remainder of the process. `ping` is a
   best-effort mean media-record arrival interval: TME1 has no echo or sender timestamp
   from which a real round-trip time could be calculated.
6. **Drop uploads** — native winit file-drop events collect into one job per
   drop (a 200 ms quiet period ends the event burst). Each job captures the
   selected host, so editing the connection target cannot redirect queued
   files. Up to eight active, queued, and failed jobs are retained in memory.
   One background worker at a time packs the selected files and folders into
   a standard tar archive and streams it through LZ4 frame compression into
   an anonymous temporary file. This keeps client memory bounded and provides
   the Content-Length required by the current TRUEOSFS HTTP receiver.

   The worker discovers raw root IDs from GET / and uses the first advertised
   (primary) mount. POST /mkdir/<root>?name=RdpUpload creates the inbox;
   POST /up/<root>/RdpUpload?name=<archive>&content=blob stores the compressed
   bytes without inference or decompression. No kernel change is required.
   Timestamp, process ID, and sequence suffixes distinguish archive names;
   duplicate top-level item names inside a drop receive numeric suffixes.
   Folder hierarchy and empty directories are preserved. Symlinks are archived
   as links and never followed; unsupported devices/pipes fail the job.

   The existing 2x microfont draws packing/upload progress at bottom left in
   both the startup and connected render paths. Completion/errors disappear
   after six seconds; the idle window has no upload label. A failure does not
   stop subsequent jobs. F8 retries retained failures on their original hosts
   (repacking the original paths); it is consumed locally only when retrying.
   The queue is session-local. Temporary archives are removed after each
   attempt. The client enforces the service's 2 GiB compressed-upload limit,
   and reports success only after a complete successful HTTP response.

   Linux uses the existing default X11/XWayland window backend for native
   drops; this winit release does not implement drop events in its opt-in
   native Wayland backend. Windows and macOS use their native drop events.

   Validation: `cargo test --offline` round-trips single files, folders, mixed
   drops, duplicate names, empty folders, and symlinks through tar/LZ4; checks
   queue serialization, failure continuation/retry, captured destinations and
   notice expiry; and exercises discovery/mkdir/Blob POST against a localhost
   mock TRUEOSFS service, including storage and truncated-response errors.

There is no remote scene graph, resource snapshot, remote shader, resize
recomposition request, or application-drawn window chrome. The connection
panel is entirely local and has no relationship to the retired TCP scene
protocol.

## SSH session and framing

The client launches the system OpenSSH executable with `-T -s USER@HOST
trueos-rdp`, using its existing config, keys, agent and terminal/askpass support.
Host-key verification is required. There is no independent RDP key database or
cryptography implementation. The kernel SSH authentication policy also governs
this subsystem, including the existing development account-proof policy.

Records have a one-byte type and a four-byte little-endian payload length,
limited to 65,536 bytes. The client sends Hello (`1`, payload `[1, view]`) and
Input (`2`, payload one complete THID record). The server sends Ready (`0x81`,
width/height as little-endian u32), Control (`0x82`, existing eight-byte THIC
record), and Video (`0x83`, one existing TME1 record of at most 1200 bytes).
Unexpected records, oversized lengths, EOF and transport failure close the
session; there is no fallback to unauthenticated UDP.

One shared transport owns input and viewing. Input has a bounded 256-record
queue. Media has an 8192-record queue, enough for a maximum 4096-fragment IDR
plus short decoder scheduling delays. Full media queues reconnect for a fresh
IDR rather than grow indefinitely or block the input producer. Old reader
threads are joined before a replacement session may publish a new input route.

The client keeps existing bounded TME1 reassembly and decoder behavior. Each new
stream session ID starts a fresh reassembler, including at bounded encoder
rollover. No UDP subscription is needed. The headless media probe uses the same
SSH transport and decoder as the desktop client.

Drop uploads remain a separate public TRUEOSFS HTTP feature with the filesystem
service's own access checks; they confer no input or viewing authority.
