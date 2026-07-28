# Roadmap

Status is intentionally conservative. `[x]` means implemented and tested.

- [ ] Core multiplexer
  - [x] Pane tree, splits, focus, close, and layout
  - [x] Session daemon and authenticated local create/list IPC
  - [x] Single-client detach and reattach
- [ ] PTY engine
  - [x] Spawn a fixed local shell with PTY input/output
  - [x] Dynamic resize and raw input mode
  - [x] Explicit child shutdown and reaping for attached panes
  - [ ] Full signal policy
- [ ] Persistent session manager (live daemon ownership works; restart restore remains)
- [x] Interactive attached split-pane TUI
- [x] Plain-text terminal parser and renderer
- [x] Strict local configuration, ten built-in themes, and bounded idle scenes
- [ ] Scrollback and copy mode
  - [x] Bounded client-local viewport navigation and literal search primitives
  - [ ] Daemon history snapshot protocol, selection, and clipboard integration
- [ ] Mouse support
  - [x] File selection, directory activation, and wheel navigation in the TUI
  - [ ] Pane focus, text selection, and native desktop drag-and-drop
- [ ] Native file explorer
  - [x] Root-confined read-only TUI explorer
  - [ ] Preview and confirmed filesystem mutations
- [ ] File preview
- [ ] Confirmed file copy, move, and delete
- [ ] Drag and drop
- [ ] SSH
- [ ] SFTP
- [ ] Workspace manager
- [ ] Git integration
- [ ] Build dashboard
- [ ] Capability-scoped plugin system
- [ ] Cross-platform packaging
- [ ] Reproducible, signed releases
