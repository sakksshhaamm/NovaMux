# NovaMux

NovaMux is an original, open-source terminal workspace and multiplexer written
in Rust. It is designed for personal sessions on shared development machines:
each operating-system user will own isolated local sessions that can be opened
after connecting over SSH.

The project is offline-first and has no telemetry, cloud service, account,
advertising, silent updater, or root requirement.

## Current status

Iteration 1 provides the dependency-free core pane tree:

- validated portable session names;
- horizontal and vertical pane splits;
- deterministic focus and close behavior;
- terminal-cell layout calculation;
- a CLI screen preview;
- an experimental real local PTY shell for macOS testing.

NovaMux does **not yet** persist or reattach sessions, render multiple live
shell panes, provide SSH/SFTP, or offer the file explorer. See
[ROADMAP.md](ROADMAP.md) for the honest implementation status.

## Build and test

Rust 1.85 or newer is required for contributors:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
cargo run -- demo my_session
cargo run -- shell
```

The PTY shell currently uses the terminal's normal line input mode. Type `exit`
or press Control-D to close it. Full-screen programs and per-keystroke shortcuts
will be enabled by the future raw-mode TUI layer.

End users will not need Rust once packaging is implemented.

## License

Apache-2.0.
