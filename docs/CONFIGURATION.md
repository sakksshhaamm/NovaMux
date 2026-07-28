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
`foreground`, `background`, `screensaver`, `scene_style`, and `idle_seconds`.

`theme` is `default`, `high-contrast`, `sakura`, `dracula`, `cyberpunk`,
`ocean`, `forest`, `nord`, `solarized-dark`, or `sunset`. The curated palettes
are original NovaMux definitions inspired by their named moods; NovaMux does
not include third-party theme code or assets.
Colors may be `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
`white`, `dark-grey`, `grey`, or exact `#RRGGBB`. Settings after `theme`
override that built-in theme. A key may appear only once.

`animation` is `off` (the default) or `subtle`. Subtle mode gently pulses only
RGB focused-border and status accents on four deterministic frames at 8 Hz. It
does not move content, affect input, or change pane snapshots. It is disabled
automatically for `TERM=dumb`; set it to `off` at any time for accessibility,
reduced motion, or maximum stillness. Named terminal colors remain static.

`screensaver` is `off` (the default), `panda-climb`, or `cat-play`.
`idle_seconds` is an integer from 10 through 3600 and defaults to 300. The
original scenes run at eight frames per second, use bounded deterministic
frames, and exist only in the local client. `scene_style` is `unicode` (the
default) or `ascii`. Unicode mode shows recognizable emoji artwork such as a
panda climbing bamboo and a cat chasing yarn. NovaMux automatically uses the
ASCII compatibility scene for `TERM=dumb` or an explicitly non-UTF-8 locale.
Any key or mouse event dismisses the scene and is consumed rather than
forwarded to the shell. Screensavers are disabled for non-interactive
terminals; small terminals show a compact one-line fallback.

Unknown keys, duplicate keys, invalid colors, and malformed lines stop startup
with the exact line number and an actionable message. Configuration is capped
at 16 KiB, 64 lines, and 512 bytes per line.

Configuration affects only the local `start` or `attach` display. It is not
sent to the daemon and cannot weaken session authentication. There are no
includes, shell commands, plugins, network references, substitutions, or
environment-variable expansion in configuration content.
