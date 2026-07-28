//! Bounded terminal emulation for live `NovaMux` panes.

/// Default number of historical lines retained per pane.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// Dimensions of one terminal pane in character cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    /// Number of visible rows.
    pub rows: u16,
    /// Number of visible columns.
    pub cols: u16,
}

impl TerminalSize {
    /// Creates a non-empty terminal size.
    #[must_use]
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: if rows == 0 { 1 } else { rows },
            cols: if cols == 0 { 1 } else { cols },
        }
    }
}

/// Parsed terminal state for one PTY.
///
/// Input is interpreted as a terminal byte stream rather than decoded as
/// untrusted text. Scrollback is bounded at construction to prevent an active
/// process from causing unbounded memory growth.
pub struct TerminalBuffer {
    parser: vt100::Parser,
    size: TerminalSize,
}

impl TerminalBuffer {
    /// Creates an empty terminal with the default bounded scrollback.
    #[must_use]
    pub fn new(size: TerminalSize) -> Self {
        Self::with_scrollback(size, DEFAULT_SCROLLBACK_LINES)
    }

    /// Creates an empty terminal with an explicit scrollback limit.
    #[must_use]
    pub fn with_scrollback(size: TerminalSize, scrollback_lines: usize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, scrollback_lines),
            size,
        }
    }

    /// Processes bytes read from a pane's PTY.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Resizes the visible terminal without discarding its parsed contents.
    pub fn resize(&mut self, size: TerminalSize) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
        self.size = size;
    }

    /// Returns the current dimensions.
    #[must_use]
    pub const fn size(&self) -> TerminalSize {
        self.size
    }

    /// Returns visible text without terminal control sequences.
    #[must_use]
    pub fn visible_text(&self) -> String {
        self.parser.screen().contents()
    }

    /// Returns terminal-formatted bytes suitable for redrawing this pane.
    #[must_use]
    pub fn formatted_screen(&self) -> Vec<u8> {
        self.parser.screen().contents_formatted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_zero_sized_terminals() {
        assert_eq!(TerminalSize::new(0, 0), TerminalSize { rows: 1, cols: 1 });
    }

    #[test]
    fn parses_ansi_without_exposing_control_sequences_as_text() {
        let mut buffer = TerminalBuffer::new(TerminalSize::new(4, 20));
        buffer.process(b"plain \x1b[31mred\x1b[0m");
        assert_eq!(buffer.visible_text(), "plain red");
    }

    #[test]
    fn handles_split_escape_sequences_across_reads() {
        let mut buffer = TerminalBuffer::new(TerminalSize::new(4, 20));
        buffer.process(b"before\x1b[");
        buffer.process(b"2J\x1b[Hafter");
        assert_eq!(buffer.visible_text(), "after");
    }

    #[test]
    fn resize_preserves_existing_content() {
        let mut buffer = TerminalBuffer::new(TerminalSize::new(2, 10));
        buffer.process(b"hello");
        buffer.resize(TerminalSize::new(4, 20));
        assert_eq!(buffer.size(), TerminalSize::new(4, 20));
        assert!(buffer.visible_text().contains("hello"));
    }
}
