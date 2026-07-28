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

The platform-independent `novamux-core` crate remains standard-library-only.
After dependencies are fetched, builds work offline.

Every future external crate must document:

- the exact capability it provides;
- its security and unsafe-code exposure;
- maintenance activity;
- a practical replacement or removal path.
