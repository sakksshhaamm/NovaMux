# Configuration and themes

NovaMux uses a single optional local configuration file:

- macOS: `~/Library/Application Support/NovaMux/config`
- Linux: `$XDG_CONFIG_HOME/novamux/config`, or `~/.config/novamux/config`
- Windows: `%APPDATA%\NovaMux\config`

If the file does not exist, NovaMux uses its built-in default theme. Copy
`examples/config` to the platform path to customize it. NovaMux does not create
or modify this file.

The format is deliberately small: one `key = value` setting per line. Empty
lines and lines beginning with `#` are ignored. Supported keys are `theme`,
`animation`, `pane_border`, `focused_border`, `status`, `status_background`,
`foreground`, and `background`.

`theme` is `default`, `high-contrast`, or `sakura`. Sakura is a dark
rose-and-purple theme designed for clear focus without reducing text contrast.
Colors may be `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
`white`, `dark-grey`, `grey`, or exact `#RRGGBB`. Settings after `theme`
override that built-in theme. A key may appear only once.

`animation` is `off` (the default) or `subtle`. Subtle mode gently pulses only
RGB focused-border and status accents on four deterministic frames at 8 Hz. It
does not move content, affect input, or change pane snapshots. It is disabled
automatically for `TERM=dumb`; set it to `off` at any time for accessibility,
reduced motion, or maximum stillness. Named terminal colors remain static.

Unknown keys, duplicate keys, invalid colors, and malformed lines stop startup
with the exact line number and an actionable message. Configuration is capped
at 16 KiB, 64 lines, and 512 bytes per line.

Configuration affects only the local `start` or `attach` display. It is not
sent to the daemon and cannot weaken session authentication. There are no
includes, shell commands, plugins, network references, substitutions, or
environment-variable expansion in configuration content.
