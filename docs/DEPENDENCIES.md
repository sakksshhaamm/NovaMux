# Dependency review

## Runtime dependencies

### `portable-pty` 0.9.0

- **Required for:** opening and controlling native pseudo-terminals on macOS,
  Linux, and Windows through one narrow interface.
- **Security:** PTY setup necessarily calls operating-system APIs and contains
  platform-specific unsafe code. NovaMux keeps that code outside its own
  workspace and never accepts a user-supplied executable path.
- **Maintenance:** maintained as part of the established WezTerm project; the
  selected release was published in February 2025.
- **Possible replacement:** small audited platform crates maintained by
  NovaMux, or direct standard-library PTY support if Rust gains it.

### `crossterm` 0.29.0

- **Required for:** portable raw-mode setup and terminal-size discovery.
- **Security:** configured without default features, excluding its event,
  clipboard, and Windows feature sets; NovaMux uses only terminal state APIs.
- **Maintenance:** actively maintained and widely used by Rust TUI projects.
- **Possible replacement:** audited platform-specific terminal mode adapters.

### `vt100` 0.16.2

- **Required for:** safe VT-compatible parsing, bounded scrollback, and screen
  state needed to render multiple independent PTYs.
- **Security:** processes can emit hostile terminal sequences; parsing them into
  a screen model prevents raw output from being composed directly by NovaMux.
- **Maintenance:** current release with complete documented public APIs.
- **Possible replacement:** a NovaMux terminal-state machine built on `vte`,
  after comprehensive compatibility and fuzz testing exists.

The platform-independent `novamux-core` crate remains standard-library-only.
After dependencies are fetched, builds work offline.

Every future external crate must document:

- the exact capability it provides;
- its security and unsafe-code exposure;
- maintenance activity;
- a practical replacement or removal path.
