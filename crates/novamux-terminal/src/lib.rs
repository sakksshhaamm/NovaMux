//! Bounded terminal emulation for live `NovaMux` panes.

use std::fmt;

/// Default number of historical lines retained per pane.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
/// Maximum UTF-8 byte length accepted for an in-memory scrollback search.
pub const MAX_SEARCH_QUERY_BYTES: usize = 256;

/// Direction in which retained terminal history is searched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchDirection {
    /// Search from the current viewport toward older output.
    Older,
    /// Search from the current viewport toward newer output.
    Newer,
}

/// A validated scrollback search query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery(String);

impl SearchQuery {
    /// Validates a literal, case-sensitive search query.
    ///
    /// # Errors
    ///
    /// Empty queries, queries longer than [`MAX_SEARCH_QUERY_BYTES`], and
    /// terminal control characters are rejected.
    pub fn parse(value: &str) -> Result<Self, SearchQueryError> {
        if value.is_empty() {
            return Err(SearchQueryError::Empty);
        }
        if value.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(SearchQueryError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(SearchQueryError::ControlCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated literal query.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a scrollback search query was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchQueryError {
    /// The query was empty.
    Empty,
    /// The query exceeded the fixed byte limit.
    TooLong,
    /// The query contained a terminal control character.
    ControlCharacter,
}

impl fmt::Display for SearchQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("search query cannot be empty"),
            Self::TooLong => formatter.write_str("search query cannot exceed 256 UTF-8 bytes"),
            Self::ControlCharacter => {
                formatter.write_str("search query cannot contain control characters")
            }
        }
    }
}

impl std::error::Error for SearchQueryError {}

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

    /// Returns the largest valid viewport offset into retained history.
    #[must_use]
    pub fn max_scrollback_offset(&self) -> usize {
        let mut screen = self.parser.screen().clone();
        screen.set_scrollback(usize::MAX);
        screen.scrollback()
    }

    /// Clamps a requested viewport offset to the retained history.
    #[must_use]
    pub fn clamp_scrollback_offset(&self, offset: usize) -> usize {
        offset.min(self.max_scrollback_offset())
    }

    /// Returns plain text for a historical viewport without changing the
    /// live terminal cursor or its active viewport.
    #[must_use]
    pub fn viewport_text(&self, offset: usize) -> String {
        let mut screen = self.parser.screen().clone();
        screen.set_scrollback(offset);
        screen.contents()
    }

    /// Moves a viewport toward older output using saturating arithmetic.
    #[must_use]
    pub fn scroll_older(&self, offset: usize, rows: usize) -> usize {
        self.clamp_scrollback_offset(offset.saturating_add(rows))
    }

    /// Moves a viewport toward newer output using saturating arithmetic.
    #[must_use]
    pub const fn scroll_newer(&self, offset: usize, rows: usize) -> usize {
        offset.saturating_sub(rows)
    }

    /// Finds the next viewport containing a validated literal query.
    ///
    /// The search is bounded by the configured scrollback limit and returns a
    /// viewport offset rather than terminal-controlled bytes.
    #[must_use]
    pub fn search(
        &self,
        query: &SearchQuery,
        from: usize,
        direction: SearchDirection,
    ) -> Option<usize> {
        let max = self.max_scrollback_offset();
        let from = from.min(max);
        let mut screen = self.parser.screen().clone();
        let mut contains_at = |offset| {
            screen.set_scrollback(offset);
            screen.contents().contains(query.as_str())
        };
        match direction {
            SearchDirection::Older => (from..=max).find(|offset| contains_at(*offset)),
            SearchDirection::Newer => (0..=from).rev().find(|offset| contains_at(*offset)),
        }
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

    fn populated_buffer() -> TerminalBuffer {
        let mut buffer = TerminalBuffer::with_scrollback(TerminalSize::new(2, 12), 4);
        buffer.process(b"zero\r\none\r\ntwo\r\nthree\r\nfour");
        buffer
    }

    #[test]
    fn viewport_navigation_is_bounded_and_does_not_mutate_live_view() {
        let buffer = populated_buffer();
        let live = buffer.visible_text();
        let maximum = buffer.max_scrollback_offset();
        assert!(maximum > 0);
        assert!(maximum <= 4);
        assert_eq!(buffer.scroll_older(0, usize::MAX), maximum);
        assert_eq!(buffer.scroll_newer(1, usize::MAX), 0);
        assert!(buffer.viewport_text(usize::MAX).contains("zero"));
        assert_eq!(buffer.visible_text(), live);
    }

    #[test]
    fn validates_search_queries_at_strict_boundaries() {
        assert_eq!(SearchQuery::parse(""), Err(SearchQueryError::Empty));
        assert_eq!(
            SearchQuery::parse(&"x".repeat(MAX_SEARCH_QUERY_BYTES + 1)),
            Err(SearchQueryError::TooLong)
        );
        assert_eq!(
            SearchQuery::parse("unsafe\u{1b}"),
            Err(SearchQueryError::ControlCharacter)
        );
        assert!(SearchQuery::parse(&"é".repeat(MAX_SEARCH_QUERY_BYTES / 2)).is_ok());
    }

    #[test]
    fn searches_history_in_both_directions_without_regex_evaluation() {
        let buffer = populated_buffer();
        let query = SearchQuery::parse("one").unwrap();
        let older = buffer.search(&query, 0, SearchDirection::Older).unwrap();
        assert!(older > 0);
        let newer = buffer
            .search(
                &query,
                buffer.max_scrollback_offset(),
                SearchDirection::Newer,
            )
            .unwrap();
        assert!(buffer.viewport_text(newer).contains(query.as_str()));
        assert_eq!(
            buffer.search(
                &SearchQuery::parse(".*").unwrap(),
                0,
                SearchDirection::Older
            ),
            None
        );
    }
}
