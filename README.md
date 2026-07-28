# NovaMux

NovaMux is an original, open-source terminal workspace and multiplexer written
in Rust. It is designed for personal sessions on shared development machines:
each operating-system user will own isolated local sessions that can be opened
after connecting over SSH.

The project is offline-first and has no telemetry, cloud service, account,
advertising, silent updater, or root requirement.

## Current status

The current development build provides:

- validated portable session names;
- horizontal and vertical pane splits;
- deterministic focus and close behavior;
- terminal-cell layout calculation;
- a CLI screen preview;
- an experimental real local PTY shell;
- an attached split-pane TUI with one independent PTY per pane.
- a private per-user daemon that owns named detached shell sessions;
- `novamux new NAME` and `novamux list` daemon controls.

The daemon keeps shell processes alive independently of the creating client,
but screen attachment and input streaming are **not yet implemented**.
NovaMux does not yet restore sessions after daemon or machine restart, render
multiple live clients, provide SSH/SFTP, or offer the file explorer. See
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
cargo run -- start
cargo run -- new work
cargo run -- list
```

The PTY shell uses raw keyboard input and propagates host-terminal size changes
to the child PTY. Type `exit` or press Control-D to close it. NovaMux displays
entry and exit messages and preserves the directory from which it was launched.

`novamux start` opens the alternate-screen multiplexer. Its controls are
`Ctrl-B %` for a left/right split, `Ctrl-B "` for a top/bottom split,
`Ctrl-B o` to focus the next pane, `Ctrl-B x` to close the focused pane, and
`Ctrl-B q` to quit. The final pane cannot be closed. This is an attached local
client; connecting that TUI to daemon-owned sessions is not implemented yet.

End users will not need Rust once packaging is implemented.

## License

Apache-2.0.
