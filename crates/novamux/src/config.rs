//! Strict, local-only user configuration.

use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: usize = 16 * 1024;
const MAX_LINES: usize = 64;
const MAX_LINE_BYTES: usize = 512;

/// A terminal color accepted by `NovaMux` configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    DarkGrey,
    Grey,
    Rgb(u8, u8, u8),
}

/// Colors used by the local TUI renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    pub pane_border: Color,
    pub focused_border: Color,
    pub status: Color,
    pub status_background: Color,
    pub foreground: Color,
    pub background: Color,
}

impl Theme {
    /// Built-in `NovaMux` default.
    #[must_use]
    pub const fn default_theme() -> Self {
        Self {
            pane_border: Color::DarkGrey,
            focused_border: Color::Cyan,
            status: Color::Black,
            status_background: Color::Cyan,
            foreground: Color::White,
            background: Color::Black,
        }
    }

    /// Built-in high-contrast theme.
    #[must_use]
    pub const fn high_contrast() -> Self {
        Self {
            pane_border: Color::White,
            focused_border: Color::Yellow,
            status: Color::Black,
            status_background: Color::Yellow,
            foreground: Color::White,
            background: Color::Black,
        }
    }

    /// Built-in rose and purple theme.
    #[must_use]
    pub const fn sakura() -> Self {
        Self {
            pane_border: Color::Rgb(118, 81, 138),
            focused_border: Color::Rgb(255, 92, 168),
            status: Color::Rgb(255, 238, 248),
            status_background: Color::Rgb(125, 45, 118),
            foreground: Color::Rgb(248, 232, 244),
            background: Color::Rgb(24, 15, 30),
        }
    }

    /// Built-in violet night theme.
    #[must_use]
    pub const fn dracula() -> Self {
        Self::rgb(
            [68, 71, 90],
            [189, 147, 249],
            [40, 42, 54],
            [255, 121, 198],
            [248, 248, 242],
        )
    }

    /// Built-in neon city theme.
    #[must_use]
    pub const fn cyberpunk() -> Self {
        Self::rgb(
            [61, 43, 88],
            [0, 255, 224],
            [18, 10, 36],
            [255, 35, 149],
            [246, 241, 255],
        )
    }

    /// Built-in deep ocean theme.
    #[must_use]
    pub const fn ocean() -> Self {
        Self::rgb(
            [35, 78, 112],
            [77, 220, 255],
            [8, 27, 42],
            [20, 115, 145],
            [221, 247, 255],
        )
    }

    /// Built-in evergreen theme.
    #[must_use]
    pub const fn forest() -> Self {
        Self::rgb(
            [54, 91, 67],
            [144, 238, 144],
            [13, 31, 20],
            [45, 103, 63],
            [230, 247, 232],
        )
    }

    /// Built-in arctic theme.
    #[must_use]
    pub const fn nord() -> Self {
        Self::rgb(
            [76, 86, 106],
            [136, 192, 208],
            [46, 52, 64],
            [94, 129, 172],
            [236, 239, 244],
        )
    }

    /// Built-in warm, low-glare dark theme.
    #[must_use]
    pub const fn solarized_dark() -> Self {
        Self::rgb(
            [88, 110, 117],
            [181, 137, 0],
            [0, 43, 54],
            [7, 54, 66],
            [147, 161, 161],
        )
    }

    /// Built-in warm sunset theme.
    #[must_use]
    pub const fn sunset() -> Self {
        Self::rgb(
            [120, 68, 92],
            [255, 180, 84],
            [35, 20, 42],
            [171, 58, 91],
            [255, 235, 208],
        )
    }

    const fn rgb(
        pane: [u8; 3],
        focused: [u8; 3],
        background: [u8; 3],
        status_background: [u8; 3],
        foreground: [u8; 3],
    ) -> Self {
        Self {
            pane_border: Color::Rgb(pane[0], pane[1], pane[2]),
            focused_border: Color::Rgb(focused[0], focused[1], focused[2]),
            status: Color::Rgb(foreground[0], foreground[1], foreground[2]),
            status_background: Color::Rgb(
                status_background[0],
                status_background[1],
                status_background[2],
            ),
            foreground: Color::Rgb(foreground[0], foreground[1], foreground[2]),
            background: Color::Rgb(background[0], background[1], background[2]),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

/// Optional, bounded local animation mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Animation {
    #[default]
    Off,
    Subtle,
}

/// Optional client-local idle artwork.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screensaver {
    #[default]
    Off,
    PandaClimb,
    CatPlay,
}

/// Complete local-client configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub theme: Theme,
    pub animation: Animation,
    pub screensaver: Screensaver,
    pub idle_seconds: u16,
}

impl Config {
    const DEFAULT_IDLE_SECONDS: u16 = 300;
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            animation: Animation::default(),
            screensaver: Screensaver::default(),
            idle_seconds: Self::DEFAULT_IDLE_SECONDS,
        }
    }
}

/// Configuration read or validation error.
#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Invalid { line: usize, message: String },
    TooLarge,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "cannot read configuration: {error}"),
            Self::Invalid { line, message } => {
                write!(formatter, "configuration line {line}: {message}")
            }
            Self::TooLarge => write!(
                formatter,
                "configuration exceeds the {MAX_CONFIG_BYTES}-byte limit"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Returns the platform's conventional `NovaMux` configuration path.
///
/// This function only locates a file. Configuration content never expands
/// environment variables.
#[must_use]
pub fn platform_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("NovaMux")
                    .join("config")
            })
    }
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
            .map(|root| root.join("NovaMux").join("config"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
        {
            Some(root.join("novamux").join("config"))
        } else {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config").join("novamux").join("config"))
        }
    }
}

/// Loads the platform configuration, using safe defaults when no file exists.
///
/// # Errors
///
/// Returns a bounded I/O or validation error for an existing invalid file.
pub fn load() -> Result<Config, ConfigError> {
    match platform_path() {
        Some(path) => load_path(&path),
        None => Ok(Config::default()),
    }
}

/// Loads an explicit path. This is also the deterministic test entry point.
///
/// # Errors
///
/// Returns a bounded I/O or validation error for an existing invalid file.
pub fn load_path(path: &Path) -> Result<Config, ConfigError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ConfigError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path is not a regular file",
        )));
    }
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(ConfigError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let source = String::from_utf8(bytes).map_err(|_| ConfigError::Invalid {
        line: 1,
        message: "configuration must be valid UTF-8".to_owned(),
    })?;
    parse(&source)
}

/// Parses configuration without performing I/O.
///
/// # Errors
///
/// Returns an exact line error when input violates the strict grammar or
/// resource bounds.
pub fn parse(source: &str) -> Result<Config, ConfigError> {
    if source.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    if source.lines().count() > MAX_LINES {
        return invalid(
            MAX_LINES + 1,
            format!("at most {MAX_LINES} lines are allowed"),
        );
    }

    let mut config = Config::default();
    let mut seen = Vec::<&str>::new();
    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        if raw.len() > MAX_LINE_BYTES {
            return invalid(
                line_number,
                format!("line exceeds the {MAX_LINE_BYTES}-byte limit"),
            );
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return invalid(line_number, "expected key = value");
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return invalid(line_number, "key and value must not be empty");
        }
        if seen.contains(&key) {
            return invalid(line_number, format!("duplicate key '{key}'"));
        }
        seen.push(key);
        match key {
            "theme" => {
                config.theme = match value {
                    "default" => Theme::default_theme(),
                    "high-contrast" => Theme::high_contrast(),
                    "sakura" => Theme::sakura(),
                    "dracula" => Theme::dracula(),
                    "cyberpunk" => Theme::cyberpunk(),
                    "ocean" => Theme::ocean(),
                    "forest" => Theme::forest(),
                    "nord" => Theme::nord(),
                    "solarized-dark" => Theme::solarized_dark(),
                    "sunset" => Theme::sunset(),
                    _ => {
                        return invalid(line_number, "unknown built-in theme");
                    }
                };
            }
            "animation" => {
                config.animation = match value {
                    "off" => Animation::Off,
                    "subtle" => Animation::Subtle,
                    _ => return invalid(line_number, "animation must be 'off' or 'subtle'"),
                };
            }
            "screensaver" => {
                config.screensaver = match value {
                    "off" => Screensaver::Off,
                    "panda-climb" => Screensaver::PandaClimb,
                    "cat-play" => Screensaver::CatPlay,
                    _ => {
                        return invalid(
                            line_number,
                            "screensaver must be 'off', 'panda-climb', or 'cat-play'",
                        );
                    }
                };
            }
            "idle_seconds" => {
                config.idle_seconds = value.parse::<u16>().map_err(|_| ConfigError::Invalid {
                    line: line_number,
                    message: "idle_seconds must be an integer from 10 to 3600".to_owned(),
                })?;
                if !(10..=3600).contains(&config.idle_seconds) {
                    return invalid(line_number, "idle_seconds must be from 10 to 3600");
                }
            }
            "pane_border" => config.theme.pane_border = parse_color(value, line_number)?,
            "focused_border" => config.theme.focused_border = parse_color(value, line_number)?,
            "status" => config.theme.status = parse_color(value, line_number)?,
            "status_background" => {
                config.theme.status_background = parse_color(value, line_number)?;
            }
            "foreground" => config.theme.foreground = parse_color(value, line_number)?,
            "background" => config.theme.background = parse_color(value, line_number)?,
            _ => return invalid(line_number, format!("unknown key '{key}'")),
        }
    }
    Ok(config)
}

fn parse_color(value: &str, line: usize) -> Result<Color, ConfigError> {
    let named = match value {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "dark-grey" => Some(Color::DarkGrey),
        "grey" => Some(Color::Grey),
        _ => None,
    };
    if let Some(color) = named {
        return Ok(color);
    }
    if value.len() == 7 && value.starts_with('#') {
        let component = |range| u8::from_str_radix(&value[range], 16).ok();
        if let (Some(red), Some(green), Some(blue)) =
            (component(1..3), component(3..5), component(5..7))
        {
            return Ok(Color::Rgb(red, green, blue));
        }
    }
    invalid(
        line,
        format!("invalid color '{value}'; use a supported name or #RRGGBB"),
    )
}

fn invalid<T>(line: usize, message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid {
        line,
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_uses_safe_defaults() {
        let path = env::temp_dir().join(format!("novamux-missing-config-{}", std::process::id()));
        assert_eq!(load_path(&path).unwrap(), Config::default());
    }

    #[test]
    fn explicit_path_loads_the_requested_file() {
        let path = env::temp_dir().join(format!(
            "novamux-config-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, "theme = sakura\nanimation = subtle\n").unwrap();
        let loaded = load_path(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(loaded.theme, Theme::sakura());
        assert_eq!(loaded.animation, Animation::Subtle);
    }

    #[test]
    fn built_in_theme_can_be_overridden_by_valid_rgb() {
        let config =
            parse("theme = high-contrast\nfocused_border = #12AbF0\nbackground = dark-grey\n")
                .unwrap();
        assert_eq!(config.theme.status_background, Color::Yellow);
        assert_eq!(config.theme.focused_border, Color::Rgb(0x12, 0xab, 0xf0));
        assert_eq!(config.theme.background, Color::DarkGrey);
    }

    #[test]
    fn sakura_and_animation_are_explicit_and_deterministic() {
        let config = parse("theme = sakura\nanimation = subtle\n").unwrap();
        assert_eq!(config.theme, Theme::sakura());
        assert_eq!(config.animation, Animation::Subtle);
        assert_eq!(parse("").unwrap().animation, Animation::Off);
        assert!(parse("animation = fast\n").is_err());
    }

    #[test]
    fn every_documented_theme_name_parses() {
        for name in [
            "default",
            "high-contrast",
            "sakura",
            "dracula",
            "cyberpunk",
            "ocean",
            "forest",
            "nord",
            "solarized-dark",
            "sunset",
        ] {
            assert!(parse(&format!("theme = {name}\n")).is_ok(), "{name}");
        }
    }

    #[test]
    fn screensaver_and_idle_bounds_are_strict() {
        let config = parse("screensaver = panda-climb\nidle_seconds = 10\n").expect("valid config");
        assert_eq!(config.screensaver, Screensaver::PandaClimb);
        assert_eq!(config.idle_seconds, 10);
        assert_eq!(parse("").unwrap().screensaver, Screensaver::Off);
        assert_eq!(parse("").unwrap().idle_seconds, 300);
        for source in [
            "idle_seconds = 9",
            "idle_seconds = 3601",
            "idle_seconds = forever",
            "screensaver = command",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_unknown_and_duplicate_keys_with_lines() {
        let unknown = parse("theme = default\nlaunch = shell\n").unwrap_err();
        assert!(unknown.to_string().contains("line 2"));
        assert!(unknown.to_string().contains("unknown key"));
        let duplicate = parse("status = red\nstatus = blue\n").unwrap_err();
        assert!(duplicate.to_string().contains("duplicate key"));
    }

    #[test]
    fn rejects_expansion_include_and_malformed_values() {
        for source in [
            "include = /tmp/other",
            "foreground = $COLOR",
            "foreground = #12345g",
            "theme = $(touch /tmp/x)",
            "theme",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_resource_exhaustion_inputs() {
        assert!(matches!(
            parse(&"x".repeat(MAX_CONFIG_BYTES + 1)),
            Err(ConfigError::TooLarge)
        ));
        assert!(parse(&format!("theme = {}", "x".repeat(MAX_LINE_BYTES))).is_err());
        assert!(parse(&"theme = default\n".repeat(MAX_LINES + 1)).is_err());
    }

    #[test]
    fn comments_cannot_smuggle_settings() {
        let config = parse("# theme = high-contrast\n\n theme = default\n").unwrap();
        assert_eq!(config, Config::default());
    }
}
