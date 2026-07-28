# Security

## Current threat surface

Iteration 1 is an in-memory state engine plus a read-only screen preview. It
does not access the filesystem, network, PTYs, credentials, or subprocesses.
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
