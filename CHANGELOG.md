# Changelog

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

### Fixed

- PTY echo is flushed after every read so typed characters remain immediately
  visible even before Enter is pressed.
- Architecture, dependency, security, and roadmap documentation.
