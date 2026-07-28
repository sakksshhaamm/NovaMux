# Architecture

NovaMux is a modular Cargo workspace.

## Components

- `novamux-core`: platform-independent session state, pane trees, focus, and
  layout. It also owns the byte-preserving `Ctrl-B` input router so shortcut
  behavior can be tested without a terminal or process. Its dependency-free
  session protocol codec defines bounded create, list, attach, detach, ping,
  success, and structured-error messages. It performs no I/O and executes no
  commands.
- `novamux-terminal`: bounded VT-compatible parsing and screen state for each
  live pane. PTY reader workers feed bytes into this layer; renderers never
  interpret raw process output directly.
- `novamux`: the user-facing executable and live PTY lifecycle layer. Each
  `LivePty` owns exactly one child, PTY master, input writer, output worker, and
  synchronized bounded terminal buffer. The `start` command reconciles these
  resources with the pane tree and renders them in an attached alternate-screen
  event loop. Its separate local-transport module binds a private Unix-domain
  endpoint and authenticates peers before handing a byte stream to bounded
  protocol dispatch. Its daemon owns a `Session` and `PaneRegistry<LivePty>` for
  every validated name. A thin attached client exchanges bounded input,
  resize, command, and screen-snapshot frames with that daemon. `demo` renders
  a fixed layout preview, while `shell`
  remains the single-PTY test interface.
- `novamux::config`: a dependency-free, bounded parser for optional local
  client configuration. It selects a built-in theme and validated color
  overrides before entering the TUI. Values never enter the daemon protocol.

Future SSH/SFTP, filesystem, UI, and plugin components will
use separate modules or crates with narrow interfaces.

## Session protocol

The local session protocol uses a fixed magic value, explicit protocol version,
message type, and big-endian payload length. A complete frame is capped at 8
KiB. Session names are revalidated while decoding, session lists are capped at
64 entries, and peer-provided error descriptions are capped at 256 bytes.
Request and response type namespaces are decoded separately.
Attached input batches are capped at 4 KiB, snapshots at 32 panes and one 8 KiB
frame, and screen text is truncated at UTF-8 boundaries. Only one connection
may attach to a named session; disconnect releases that reservation.

The codec does not establish trust. The local transport establishes a
same-effective-user boundary using a private runtime directory and
kernel-supplied Unix peer credentials. Protocol framing and session ownership
remain independent from endpoint ownership.

## Shared-VM model

The intended deployment is one unprivileged NovaMux daemon per operating-system
user. Runtime state will live in a user-owned directory with restrictive
permissions. No cross-user attach operation will be provided by default.
Connecting to the VM remains the responsibility of the organisation's existing
SSH service; NovaMux will attach to that user's local session after login.

## Invariants

- A live session always contains at least one pane.
- Exactly one pane is focused.
- Pane identifiers are unique within a session and never reused.
- Runtime resources reconcile atomically with pane IDs; a failed PTY spawn
  cannot leave a partially registered pane set.
- A live PTY launches only NovaMux's fixed platform shell path, validates its
  working directory, and reaps its child during explicit or automatic cleanup.
- All session names pass a portable allow-list before reaching storage or IPC.
- Attached clients never own daemon PTYs; detach and disconnect preserve them.
- Core layout calculations are deterministic and use saturating arithmetic at
  terminal coordinate boundaries.
- Historical viewport reads clone bounded screen state, while each TUI client
  owns its copy-mode offset. Navigation therefore cannot move the daemon's live
  terminal viewport or pause its PTYs.
- Themes are presentation-only client state. Daemon session ownership and IPC
  authentication are independent of configuration.
- Optional accent animation derives one of four deterministic frames from
  monotonic elapsed time. No animation state grows over time, and only the
  focused border and status accent change.
- Optional screensavers are local-client state derived from a monotonic
  idle deadline. Their deterministic frames are bounded by terminal dimensions,
  rendered at exactly eight frames per second while active, and never enter the
  session protocol or daemon. The renderer calculates Unicode display-cell
  width before centering or clipping emoji scenes.
