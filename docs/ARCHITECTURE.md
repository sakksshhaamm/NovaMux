# Architecture

NovaMux is a modular Cargo workspace.

## Components

- `novamux-core`: platform-independent session state, pane trees, focus, and
  layout. It also owns the byte-preserving `Ctrl-B` input router so shortcut
  behavior can be tested without a terminal or process. It performs no I/O and
  executes no commands.
- `novamux-terminal`: bounded VT-compatible parsing and screen state for each
  live pane. PTY reader workers feed bytes into this layer; renderers never
  interpret raw process output directly.
- `novamux`: the user-facing executable. Its current `demo` command renders a
  fixed layout preview without starting a shell.

Future PTY, daemon/IPC, terminal emulation, SSH/SFTP, filesystem, UI, and
plugin components will be separate crates with narrow interfaces.

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
- All session names pass a portable allow-list before reaching storage or IPC.
- Core layout calculations are deterministic and use saturating arithmetic at
  terminal coordinate boundaries.
