//! Client-local copy-mode navigation state.

/// A navigation action interpreted while copy mode is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyModeAction {
    /// Move one row toward older output.
    OlderLine,
    /// Move one row toward newer output.
    NewerLine,
    /// Move one visible page toward older output.
    OlderPage,
    /// Move one visible page toward newer output.
    NewerPage,
    /// Return to the live viewport and leave copy mode.
    Exit,
}

/// Viewport state owned by one attached client, never by the session daemon.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CopyMode {
    active: bool,
    offset: usize,
}

impl CopyMode {
    /// Enters copy mode at the live viewport.
    pub fn enter(&mut self) {
        self.active = true;
        self.offset = 0;
    }

    /// Leaves copy mode and returns to live output.
    pub fn exit(&mut self) {
        self.active = false;
        self.offset = 0;
    }

    /// Returns whether navigation keys should be intercepted.
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.active
    }

    /// Returns the requested historical row offset.
    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// Applies navigation while clamping to available retained history.
    pub fn apply(&mut self, action: CopyModeAction, page_rows: usize, maximum: usize) {
        if !self.active {
            return;
        }
        match action {
            CopyModeAction::OlderLine => {
                self.offset = self.offset.saturating_add(1).min(maximum);
            }
            CopyModeAction::NewerLine => self.offset = self.offset.saturating_sub(1),
            CopyModeAction::OlderPage => {
                self.offset = self.offset.saturating_add(page_rows).min(maximum);
            }
            CopyModeAction::NewerPage => {
                self.offset = self.offset.saturating_sub(page_rows);
            }
            CopyModeAction::Exit => self.exit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_inert_until_entered_and_saturates() {
        let mut mode = CopyMode::default();
        mode.apply(CopyModeAction::OlderLine, 10, 5);
        assert_eq!(mode.offset(), 0);
        mode.enter();
        mode.apply(CopyModeAction::OlderPage, usize::MAX, 5);
        assert_eq!(mode.offset(), 5);
        mode.apply(CopyModeAction::NewerPage, usize::MAX, 5);
        assert_eq!(mode.offset(), 0);
    }

    #[test]
    fn exit_resets_client_local_viewport() {
        let mut mode = CopyMode::default();
        mode.enter();
        mode.apply(CopyModeAction::OlderLine, 1, 3);
        mode.apply(CopyModeAction::Exit, 1, 3);
        assert!(!mode.is_active());
        assert_eq!(mode.offset(), 0);
    }
}
