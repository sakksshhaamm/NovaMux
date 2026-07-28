# Security

## Current threat surface

The core is an in-memory state engine. The `shell` command opens a local PTY and
launches one compile-time-selected system shell: `/bin/zsh` on macOS, `/bin/sh`
on other Unix systems, or `cmd.exe` on Windows. NovaMux does not accept a shell
path, command, or argument from the user. It does not elevate privileges.
User-supplied session names are allow-listed to ASCII letters, numbers, hyphens,
and underscores and are capped at 64 bytes.

Unsafe Rust is forbidden across the workspace.

## Product commitments

- no telemetry, analytics, advertisements, or mandatory accounts;
- no root privileges for normal use;
- no plaintext password storage;
- no silent updates;
- explicit confirmation before overwriting user files;
- canonical path and symlink-boundary checks before filesystem mutations;
- authenticated, user-scoped IPC with restrictive local permissions;
- dependency review, reproducible builds, and signed release artifacts.

Security-sensitive features will include adversarial tests before being marked
complete.
## Attached multiplexer

The `start` client launches only NovaMux's fixed platform shell executable and
forwards keystrokes to the currently focused PTY. Multiplexer commands are
recognized from a fixed `Ctrl-B` allow-list; they are never interpolated into a
shell command. Pane output is parsed into bounded terminal state and rendered
as plain text, so untrusted PTY escape sequences are not replayed into the host
terminal.

Pane working directories are canonicalized and must already exist. Closing a
pane or quitting explicitly terminates and reaps its child process. The
alternate screen and raw input mode are restored through a guard on every
normal Rust error path. Process aborts and operating-system termination signals
are not yet covered by a full signal policy.
