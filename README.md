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
- `novamux attach NAME` with live input, splits, focus, resize, and detach.
- bounded local scrollback navigation entered with `Ctrl-B [`; selection and
  system clipboard copying are not implemented yet.
- strict offline local configuration with ten curated themes (including
  Sakura, Cyberpunk, Ocean, Forest, Nord, and Sunset), validated named and
  24-bit colors, optional subtle accents, and client-only Panda and cat idle
  scenes.
- a root-confined, read-only file explorer with keyboard and mouse navigation.

The daemon keeps shell processes alive independently of the creating client,
including while its single attached client is disconnected. NovaMux does not
yet restore sessions after daemon or machine restart, support multiple clients
on one session, provide SSH/SFTP, or offer the file explorer. See
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
cargo run -- attach work
```

The PTY shell uses raw keyboard input and propagates host-terminal size changes
to the child PTY. Type `exit` or press Control-D to close it. NovaMux displays
entry and exit messages and preserves the directory from which it was launched.

`novamux start` opens the alternate-screen multiplexer. Its controls are
`Ctrl-B %` for a left/right split, `Ctrl-B "` for a top/bottom split,
`Ctrl-B o` to focus the next pane, `Ctrl-B x` to close the focused pane, and
`Ctrl-B q` to quit. `Ctrl-B [` enters copy mode; arrow keys or `j`/`k` move by
line, Page Up/Page Down move by page, and Escape or `q` exits. The final pane
cannot be closed.

`Ctrl-B f` opens the read-only file explorer rooted at the directory from which
that client was launched. Click once or use the arrow keys to select; double
click or press Enter to open a directory; use the mouse wheel or Page Up/Page
Down to scroll; Left/Backspace or right-click goes to the parent; Escape closes
the explorer. Parent navigation cannot cross the original canonical root.
Symlinks are labeled `LINK` and are not followed. This checkpoint cannot
preview, copy, move, delete, drag, execute, or otherwise modify files.

For a named daemon session, run `novamux attach NAME`. `Ctrl-B d` detaches
without stopping its shells; running `attach` again restores the latest bounded
screen state. An unexpected client disconnect is also treated as a detach.
For `attach`, the explorer remains client-local: it browses the filesystem of
the machine where the `attach` command runs, not daemon state or a separate
desktop. When NovaMux is run after SSH login, that machine is the SSH host.

End users will not need Rust once packaging is implemented.

## Configuration

The optional configuration file changes local `start` and `attach` colors. It
uses a strict declarative format and safe defaults when absent. See
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) and the
[example configuration](examples/config). It never executes commands, loads
plugins or includes, accesses the network, or expands variables.

## License

Apache-2.0.
