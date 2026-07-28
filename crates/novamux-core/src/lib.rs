//! Platform-independent state and layout primitives for `NovaMux`.

use std::fmt;

mod input;

pub use input::{InputAction, InputRouter, MultiplexerCommand};

/// A stable identifier for a pane within one session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PaneId(u64);

impl PaneId {
    /// Returns the numeric identifier used for display and persistence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Direction used when splitting a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitDirection {
    /// Places the new pane to the right.
    Horizontal,
    /// Places the new pane below.
    Vertical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    Pane(PaneId),
    Split {
        direction: SplitDirection,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// A terminal-cell rectangle assigned to a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// Zero-based column.
    pub x: u16,
    /// Zero-based row.
    pub y: u16,
    /// Width in terminal cells.
    pub width: u16,
    /// Height in terminal cells.
    pub height: u16,
}

/// A validated session name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionName(String);

impl SessionName {
    /// Validates a user-visible session name.
    ///
    /// Names are deliberately restricted to portable ASCII characters. This
    /// makes them safe for logs, local metadata paths, and future IPC routing.
    ///
    /// # Errors
    ///
    /// Returns [`SessionNameError`] when the value is empty, exceeds 64 bytes,
    /// or includes a character outside the portable allow-list.
    pub fn parse(value: &str) -> Result<Self, SessionNameError> {
        if value.is_empty() {
            return Err(SessionNameError::Empty);
        }
        if value.len() > 64 {
            return Err(SessionNameError::TooLong);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(SessionNameError::InvalidCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a session name was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionNameError {
    /// The name contained no characters.
    Empty,
    /// The name exceeded 64 bytes.
    TooLong,
    /// The name included a character outside `[A-Za-z0-9_-]`.
    InvalidCharacter,
}

impl fmt::Display for SessionNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("session name cannot be empty"),
            Self::TooLong => formatter.write_str("session name cannot exceed 64 characters"),
            Self::InvalidCharacter => {
                formatter.write_str("session name may contain only letters, numbers, '-' and '_'")
            }
        }
    }
}

impl std::error::Error for SessionNameError {}

/// Mutable pane tree for a single multiplexer session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    name: SessionName,
    root: Node,
    focused: PaneId,
    next_id: u64,
}

impl Session {
    /// Creates a session containing one focused pane.
    #[must_use]
    pub fn new(name: SessionName) -> Self {
        let initial = PaneId(1);
        Self {
            name,
            root: Node::Pane(initial),
            focused: initial,
            next_id: 2,
        }
    }

    /// Returns the session name.
    #[must_use]
    pub fn name(&self) -> &SessionName {
        &self.name
    }

    /// Returns the focused pane.
    #[must_use]
    pub const fn focused(&self) -> PaneId {
        self.focused
    }

    /// Returns pane identifiers in stable visual-tree order.
    #[must_use]
    pub fn panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        collect_panes(&self.root, &mut panes);
        panes
    }

    /// Splits the focused pane and focuses the newly created pane.
    ///
    /// # Panics
    ///
    /// Panics if a single session exhausts all `u64` pane identifiers.
    pub fn split_focused(&mut self, direction: SplitDirection) -> PaneId {
        let new_pane = PaneId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("pane identifier space exhausted");
        split_node(&mut self.root, self.focused, new_pane, direction);
        self.focused = new_pane;
        new_pane
    }

    /// Focuses the next pane, wrapping at the end.
    ///
    /// # Panics
    ///
    /// Panics only if the internal session invariant is violated and the
    /// focused pane is absent from the pane tree.
    pub fn focus_next(&mut self) {
        let panes = self.panes();
        let current = panes
            .iter()
            .position(|pane| *pane == self.focused)
            .expect("focused pane must belong to session");
        self.focused = panes[(current + 1) % panes.len()];
    }

    /// Closes the focused pane.
    ///
    /// The final pane cannot be closed because a live session must always have
    /// a display target. Returns `true` when a pane was removed.
    ///
    /// # Panics
    ///
    /// Panics only if an internal session invariant is violated.
    pub fn close_focused(&mut self) -> bool {
        let panes = self.panes();
        if panes.len() == 1 {
            return false;
        }
        let current = panes
            .iter()
            .position(|pane| *pane == self.focused)
            .expect("focused pane must belong to session");
        let next_focus = panes[if current == 0 { 1 } else { current - 1 }];
        self.root = remove_pane(self.root.clone(), self.focused)
            .expect("closing one of several panes must leave a tree");
        self.focused = next_focus;
        true
    }

    /// Calculates rectangles for all panes in terminal-cell coordinates.
    ///
    /// Odd cells are assigned to the first child. Zero-area child rectangles
    /// are retained so callers can render a clear “terminal too small” state.
    #[must_use]
    pub fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut result = Vec::new();
        layout_node(&self.root, area, &mut result);
        result
    }
}

fn collect_panes(node: &Node, output: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => output.push(*id),
        Node::Split { first, second, .. } => {
            collect_panes(first, output);
            collect_panes(second, output);
        }
    }
}

fn split_node(
    node: &mut Node,
    target: PaneId,
    new_pane: PaneId,
    direction: SplitDirection,
) -> bool {
    match node {
        Node::Pane(id) if *id == target => {
            *node = Node::Split {
                direction,
                first: Box::new(Node::Pane(target)),
                second: Box::new(Node::Pane(new_pane)),
            };
            true
        }
        Node::Pane(_) => false,
        Node::Split { first, second, .. } => {
            split_node(first, target, new_pane, direction)
                || split_node(second, target, new_pane, direction)
        }
    }
}

fn remove_pane(node: Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Pane(id) => (id != target).then_some(Node::Pane(id)),
        Node::Split {
            direction,
            first,
            second,
        } => match (remove_pane(*first, target), remove_pane(*second, target)) {
            (Some(first), Some(second)) => Some(Node::Split {
                direction,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (Some(survivor), None) | (None, Some(survivor)) => Some(survivor),
            (None, None) => None,
        },
    }
}

fn layout_node(node: &Node, area: Rect, output: &mut Vec<(PaneId, Rect)>) {
    match node {
        Node::Pane(id) => output.push((*id, area)),
        Node::Split {
            direction,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction);
            layout_node(first, first_area, output);
            layout_node(second, second_area, output);
        }
    }
}

fn split_rect(area: Rect, direction: SplitDirection) -> (Rect, Rect) {
    match direction {
        SplitDirection::Horizontal => {
            let first_width = area.width.saturating_add(1) / 2;
            (
                Rect {
                    width: first_width,
                    ..area
                },
                Rect {
                    x: area.x.saturating_add(first_width),
                    width: area.width.saturating_sub(first_width),
                    ..area
                },
            )
        }
        SplitDirection::Vertical => {
            let first_height = area.height.saturating_add(1) / 2;
            (
                Rect {
                    height: first_height,
                    ..area
                },
                Rect {
                    y: area.y.saturating_add(first_height),
                    height: area.height.saturating_sub(first_height),
                    ..area
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(SessionName::parse("team_dev").unwrap())
    }

    #[test]
    fn validates_portable_session_names() {
        assert!(SessionName::parse("release-42_a").is_ok());
        assert_eq!(SessionName::parse(""), Err(SessionNameError::Empty));
        assert_eq!(
            SessionName::parse("../escape"),
            Err(SessionNameError::InvalidCharacter)
        );
        assert_eq!(
            SessionName::parse(&"a".repeat(65)),
            Err(SessionNameError::TooLong)
        );
    }

    #[test]
    fn splits_and_focuses_new_pane() {
        let mut session = session();
        let created = session.split_focused(SplitDirection::Horizontal);
        assert_eq!(created, PaneId(2));
        assert_eq!(session.focused(), PaneId(2));
        assert_eq!(session.panes(), vec![PaneId(1), PaneId(2)]);
    }

    #[test]
    fn nested_layout_covers_requested_area() {
        let mut session = session();
        session.split_focused(SplitDirection::Horizontal);
        session.split_focused(SplitDirection::Vertical);
        let layout = session.layout(Rect {
            x: 0,
            y: 0,
            width: 81,
            height: 25,
        });
        assert_eq!(
            layout,
            vec![
                (
                    PaneId(1),
                    Rect {
                        x: 0,
                        y: 0,
                        width: 41,
                        height: 25
                    }
                ),
                (
                    PaneId(2),
                    Rect {
                        x: 41,
                        y: 0,
                        width: 40,
                        height: 13
                    }
                ),
                (
                    PaneId(3),
                    Rect {
                        x: 41,
                        y: 13,
                        width: 40,
                        height: 12
                    }
                ),
            ]
        );
    }

    #[test]
    fn closes_focused_pane_and_collapses_parent_split() {
        let mut session = session();
        session.split_focused(SplitDirection::Horizontal);
        session.split_focused(SplitDirection::Vertical);
        assert!(session.close_focused());
        assert_eq!(session.panes(), vec![PaneId(1), PaneId(2)]);
        assert_eq!(session.focused(), PaneId(2));
    }

    #[test]
    fn refuses_to_close_last_pane() {
        let mut session = session();
        assert!(!session.close_focused());
        assert_eq!(session.panes(), vec![PaneId(1)]);
    }

    #[test]
    fn next_focus_wraps() {
        let mut session = session();
        session.split_focused(SplitDirection::Horizontal);
        session.focus_next();
        assert_eq!(session.focused(), PaneId(1));
    }
}
