# Roadmap

Status is intentionally conservative. `[x]` means implemented and tested.

- [ ] Core multiplexer
  - [x] Pane tree, splits, focus, close, and layout
  - [x] Session daemon and authenticated local create/list IPC
  - [ ] Detach and reattach
- [ ] PTY engine
  - [x] Spawn a fixed local shell with PTY input/output
  - [x] Dynamic resize and raw input mode
  - [x] Explicit child shutdown and reaping for attached panes
  - [ ] Full signal policy
- [ ] Persistent session manager
- [x] Interactive attached split-pane TUI
- [x] Plain-text terminal parser and renderer
- [ ] Mouse support
- [ ] Native file explorer
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
