# Changelog

## Unreleased

- Added strict, bounded local configuration with actionable line errors,
  safe no-file defaults, default, high-contrast, and Sakura themes, and
  validated named or `#RRGGBB` colors for pane borders, status bar, foreground,
  and background.
- Added an opt-in, reduced-motion-friendly subtle accent animation with four
  bounded deterministic frames at 8 Hz and automatic `TERM=dumb` suppression.
- Applied themes only to local `start` and `attach` clients without extending
  daemon or protocol trust.
- Added client-local copy-mode state entered with `Ctrl-B [`, safe line/page
  navigation for local panes, and an explicit current-viewport mode for remote
  attachments pending a history snapshot protocol.
- Added non-mutating bounded scrollback viewport, literal search, and text
  extraction primitives with strict query validation.
- Added a dependency-free, size-bounded session IPC protocol codec for the
  upcoming create/list/attach/detach daemon milestone.
- Added a private macOS/Linux Unix-domain endpoint with restrictive runtime
  permissions, safe stale-socket recovery, path limits, I/O timeouts, and
  fail-closed kernel peer authentication.
- Added an explicit unsupported session transport on Windows pending an
  equivalently secure named-pipe implementation.
- Added a per-user session daemon that owns named shell PTYs after the creating
  client exits, with bounded `Ping`, `Create`, and `List` request handling.
- Added `novamux new NAME` and `novamux list`, including fixed-current-executable
  daemon autostart and a strict 64-session limit.
- Added `novamux attach NAME`, bounded daemon screen snapshots, focused-pane
  input and resize forwarding, split/focus/close commands, and `Ctrl-B d`
  detach with daemon-owned PTY survival.
- Enforced one attached client per session and made unexpected disconnects
  release attachment state safely.

## 0.1.0 - 2026-07-28

### Added

- Dependency-free Rust workspace.
- Validated session names.
- Core pane split, focus, close, and layout engine.
- CLI screen preview.
- Experimental cross-platform PTY shell command with a fixed executable.
- Visible PTY entry/exit messages and preservation of the launch directory.
- Raw keyboard input and live terminal resize propagation.
- Byte-preserving `Ctrl-B` command routing for live-pane controls.
- Bounded VT-compatible terminal buffers for independent live panes.
- Atomic pane-to-runtime ownership reconciliation with failed-spawn rollback.
- Per-pane live PTY runtime ownership, input, resize, exit polling, cleanup,
  and synchronized output parsing into bounded terminal buffers.
- Attached alternate-screen `start` interface with live horizontal and vertical
  splits, focus switching, guarded pane closing, and clean quit.
- Plain pane borders, focused-pane indication, and an always-visible shortcut
  reference.

### Fixed

- PTY echo is flushed after every read so typed characters remain immediately
  visible even before Enter is pressed.
- Architecture, dependency, security, and roadmap documentation.
