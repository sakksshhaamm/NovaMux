//! Byte-preserving keyboard routing for the multiplexer command prefix.

const COMMAND_PREFIX: u8 = 0x02;

/// A command intercepted by the multiplexer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MultiplexerCommand {
    /// Split the focused pane into left and right children.
    SplitHorizontal,
    /// Split the focused pane into top and bottom children.
    SplitVertical,
    /// Focus the next pane in visual-tree order.
    FocusNext,
    /// Close the focused pane after confirmation by the UI layer.
    CloseFocused,
    /// Disconnect this client while preserving a daemon-owned session.
    Detach,
    /// Enter client-local scrollback and copy mode.
    EnterCopyMode,
    /// Exit the attached client after confirmation by the UI layer.
    Quit,
}

/// Result of processing one input byte.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputAction {
    /// No output yet because the router is waiting for a command key.
    Pending,
    /// Bytes that must be delivered unchanged to the focused PTY.
    Forward(Vec<u8>),
    /// A command for the multiplexer client.
    Command(MultiplexerCommand),
}

/// Stateful router for `Ctrl-B` multiplexer shortcuts.
///
/// Ordinary bytes are returned unchanged. An unknown prefixed key is also
/// preserved as `Ctrl-B` followed by that byte, preventing accidental input
/// loss in terminal applications.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputRouter {
    prefix_pending: bool,
}

impl InputRouter {
    /// Creates a router with no pending prefix.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prefix_pending: false,
        }
    }

    /// Processes one raw input byte.
    pub fn route(&mut self, byte: u8) -> InputAction {
        if !self.prefix_pending {
            if byte == COMMAND_PREFIX {
                self.prefix_pending = true;
                return InputAction::Pending;
            }
            return InputAction::Forward(vec![byte]);
        }

        self.prefix_pending = false;
        match byte {
            b'%' => InputAction::Command(MultiplexerCommand::SplitHorizontal),
            b'"' => InputAction::Command(MultiplexerCommand::SplitVertical),
            b'o' => InputAction::Command(MultiplexerCommand::FocusNext),
            b'x' => InputAction::Command(MultiplexerCommand::CloseFocused),
            b'd' => InputAction::Command(MultiplexerCommand::Detach),
            b'[' => InputAction::Command(MultiplexerCommand::EnterCopyMode),
            b'q' => InputAction::Command(MultiplexerCommand::Quit),
            COMMAND_PREFIX => InputAction::Forward(vec![COMMAND_PREFIX]),
            other => InputAction::Forward(vec![COMMAND_PREFIX, other]),
        }
    }

    /// Flushes a pending prefix when the input stream closes.
    ///
    /// This guarantees that a trailing `Ctrl-B` is not silently discarded.
    #[must_use]
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        self.prefix_pending.then(|| {
            self.prefix_pending = false;
            vec![COMMAND_PREFIX]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_ordinary_input_unchanged() {
        let mut router = InputRouter::new();
        assert_eq!(router.route(b'a'), InputAction::Forward(vec![b'a']));
        assert_eq!(router.route(b'\n'), InputAction::Forward(vec![b'\n']));
    }

    #[test]
    fn recognizes_every_supported_command() {
        let cases = [
            (b'%', MultiplexerCommand::SplitHorizontal),
            (b'"', MultiplexerCommand::SplitVertical),
            (b'o', MultiplexerCommand::FocusNext),
            (b'x', MultiplexerCommand::CloseFocused),
            (b'd', MultiplexerCommand::Detach),
            (b'[', MultiplexerCommand::EnterCopyMode),
            (b'q', MultiplexerCommand::Quit),
        ];

        for (key, command) in cases {
            let mut router = InputRouter::new();
            assert_eq!(router.route(COMMAND_PREFIX), InputAction::Pending);
            assert_eq!(router.route(key), InputAction::Command(command));
        }
    }

    #[test]
    fn doubled_prefix_sends_one_literal_prefix() {
        let mut router = InputRouter::new();
        router.route(COMMAND_PREFIX);
        assert_eq!(
            router.route(COMMAND_PREFIX),
            InputAction::Forward(vec![COMMAND_PREFIX])
        );
    }

    #[test]
    fn unknown_command_is_not_lost() {
        let mut router = InputRouter::new();
        router.route(COMMAND_PREFIX);
        assert_eq!(
            router.route(b'z'),
            InputAction::Forward(vec![COMMAND_PREFIX, b'z'])
        );
    }

    #[test]
    fn finish_flushes_a_trailing_prefix_once() {
        let mut router = InputRouter::new();
        router.route(COMMAND_PREFIX);
        assert_eq!(router.finish(), Some(vec![COMMAND_PREFIX]));
        assert_eq!(router.finish(), None);
    }
}
