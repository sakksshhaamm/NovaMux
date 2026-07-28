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

## Session IPC protocol

The platform-independent protocol codec accepts at most 8 KiB per complete
frame and rejects truncated frames, trailing bytes, unsupported versions,
unknown message and error codes, invalid UTF-8, invalid session names, session
lists beyond 64 entries, and error text beyond 256 bytes. It has no deserializer
dependency and performs no I/O.

These checks limit parsing and allocation exposure. On macOS and Linux, the
local transport authenticates every accepted connection using kernel-supplied
peer credentials before returning it to protocol code. It never listens on a
network socket.

The endpoint lives in a UID-qualified runtime directory. The directory must be
absolute, owned by the effective user, be a real directory rather than a
symlink, and grant no group or other access; newly created directories use mode
`0700`. The socket path is limited to the conservative macOS maximum and is
changed to mode `0600` immediately after binding.

Endpoint recovery is fail-closed. NovaMux first attempts a connection. A
successful connection means a daemon is live. Only after a failed connection
will it inspect the endpoint without following symlinks, and it removes only a
Unix socket owned by the current effective user. Regular files, symlinks,
foreign-owned objects, and insecure runtime directories are never replaced.
Accepted streams receive fixed read and write timeouts.

Windows exposes an explicit unsupported transport rather than an
unauthenticated fallback. It needs equivalent per-user ACLs and kernel peer
identity before detach/attach can be enabled. Session dispatch is not part of
this checkpoint.

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
