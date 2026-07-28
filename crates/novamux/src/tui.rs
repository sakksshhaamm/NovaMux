use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use novamux_core::{
    InputAction, InputRouter, MultiplexerCommand, PaneId, PaneRegistry, Rect, Session, SessionName,
    SplitDirection,
};
use novamux_terminal::TerminalSize;

use crate::live_pty::LivePty;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const STATUS_ROWS: u16 = 1;

/// Runs an attached, interactive multiplexer client.
///
/// # Errors
///
/// Returns an error if the terminal is not interactive, its state cannot be
/// changed, a pane shell cannot be managed, or input/output fails.
pub fn run() -> AppResult<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("novamux start requires an interactive terminal".into());
    }

    let _screen = ScreenGuard::enter()?;
    let mut app = App::new()?;
    let result = app.event_loop();
    app.shutdown();
    result
}

struct App {
    session: Session,
    panes: PaneRegistry<LivePty>,
    router: InputRouter,
    last_size: (u16, u16),
}

impl App {
    fn new() -> AppResult<Self> {
        let session = Session::new(SessionName::parse("local")?);
        let mut panes = PaneRegistry::new();
        let size = terminal::size()?;
        panes.reconcile(&session, |_| LivePty::spawn(TerminalSize::new(2, 2)))?;
        let mut app = Self {
            session,
            panes,
            router: InputRouter::new(),
            last_size: size,
        };
        app.resize_panes(size)?;
        Ok(app)
    }

    fn event_loop(&mut self) -> AppResult<()> {
        let mut stdout = io::stdout().lock();
        loop {
            let size = terminal::size()?;
            if size != self.last_size {
                self.resize_panes(size)?;
                self.last_size = size;
            }
            self.render(&mut stdout, size)?;

            if event::poll(Duration::from_millis(33))? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        for byte in key_bytes(key) {
                            match self.router.route(byte) {
                                InputAction::Pending => {}
                                InputAction::Forward(bytes) => {
                                    if let Some(pane) = self.panes.get_mut(self.session.focused()) {
                                        pane.write(&bytes)?;
                                    }
                                }
                                InputAction::Command(command) => {
                                    if self.command(command, size)? {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    Event::Resize(cols, rows) => {
                        self.resize_panes((cols, rows))?;
                        self.last_size = (cols, rows);
                    }
                    _ => {}
                }
            }

            for pane_id in self.session.panes() {
                if self
                    .panes
                    .get_mut(pane_id)
                    .is_some_and(|pane| pane.try_wait().ok().flatten().is_some())
                    && self.session.panes().len() > 1
                {
                    if self.session.focused() != pane_id {
                        while self.session.focused() != pane_id {
                            self.session.focus_next();
                        }
                    }
                    self.close_focused()?;
                    break;
                }
            }
        }
    }

    fn command(&mut self, command: MultiplexerCommand, size: (u16, u16)) -> AppResult<bool> {
        match command {
            MultiplexerCommand::SplitHorizontal => {
                self.split(SplitDirection::Horizontal, size)?;
            }
            MultiplexerCommand::SplitVertical => {
                self.split(SplitDirection::Vertical, size)?;
            }
            MultiplexerCommand::FocusNext => self.session.focus_next(),
            MultiplexerCommand::CloseFocused => self.close_focused()?,
            MultiplexerCommand::Quit => return Ok(true),
        }
        Ok(false)
    }

    fn split(&mut self, direction: SplitDirection, size: (u16, u16)) -> AppResult<()> {
        let mut candidate = self.session.clone();
        candidate.split_focused(direction);
        self.panes
            .reconcile(&candidate, |_| LivePty::spawn(TerminalSize::new(2, 2)))?;
        self.session = candidate;
        self.resize_panes(size)
    }

    fn close_focused(&mut self) -> AppResult<()> {
        if !self.session.close_focused() {
            return Ok(());
        }
        for (_, mut pane) in self
            .panes
            .reconcile(&self.session, |_| LivePty::spawn(TerminalSize::new(2, 2)))?
        {
            pane.terminate()?;
        }
        Ok(())
    }

    fn resize_panes(&mut self, size: (u16, u16)) -> AppResult<()> {
        for (pane, rect) in self.layout(size) {
            if let Some(runtime) = self.panes.get_mut(pane) {
                runtime.resize(content_size(rect))?;
            }
        }
        Ok(())
    }

    fn layout(&self, (cols, rows): (u16, u16)) -> Vec<(PaneId, Rect)> {
        self.session.layout(Rect {
            x: 0,
            y: 0,
            width: cols,
            height: rows.saturating_sub(STATUS_ROWS),
        })
    }

    fn render(&self, output: &mut impl Write, size: (u16, u16)) -> AppResult<()> {
        queue!(output, Hide, Clear(ClearType::All))?;
        for (pane, rect) in self.layout(size) {
            let focused = pane == self.session.focused();
            draw_border(output, rect, pane, focused)?;
            if rect.width > 2 && rect.height > 2 {
                let text = self
                    .panes
                    .get(pane)
                    .map(|runtime| {
                        runtime.with_terminal(novamux_terminal::TerminalBuffer::visible_text)
                    })
                    .transpose()?
                    .unwrap_or_default();
                draw_text(output, rect, &text)?;
            }
        }
        let status_y = size.1.saturating_sub(1);
        queue!(
            output,
            MoveTo(0, status_y),
            SetAttribute(Attribute::Reverse),
            Print(fit(
                " NovaMux  Ctrl-B % split | \" split | o focus | x close | q quit ",
                size.0
            )),
            SetAttribute(Attribute::Reset)
        )?;
        output.flush()?;
        Ok(())
    }

    fn shutdown(&mut self) {
        for pane in self.session.panes() {
            if let Some(runtime) = self.panes.get_mut(pane) {
                let _ = runtime.terminate();
            }
        }
    }
}

fn key_bytes(key: KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(character) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = character.to_ascii_lowercase() as u32;
            if (u32::from(b'a')..=u32::from(b'z')).contains(&lower) {
                vec![u8::try_from(lower - u32::from(b'a') + 1).unwrap_or_default()]
            } else {
                Vec::new()
            }
        }
        KeyCode::Char(character) => {
            let mut bytes = [0; 4];
            character.encode_utf8(&mut bytes).as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => Vec::new(),
    }
}

fn content_size(rect: Rect) -> TerminalSize {
    TerminalSize::new(
        rect.height.saturating_sub(2).max(2),
        rect.width.saturating_sub(2).max(2),
    )
}

fn draw_border(output: &mut impl Write, rect: Rect, pane: PaneId, focused: bool) -> io::Result<()> {
    if rect.width == 0 || rect.height == 0 {
        return Ok(());
    }
    let glyph = if focused { '#' } else { '+' };
    for x in rect.x..rect.x.saturating_add(rect.width) {
        queue!(output, MoveTo(x, rect.y), Print(glyph))?;
        if rect.height > 1 {
            queue!(
                output,
                MoveTo(x, rect.y + rect.height.saturating_sub(1)),
                Print(glyph)
            )?;
        }
    }
    for y in rect.y..rect.y.saturating_add(rect.height) {
        queue!(output, MoveTo(rect.x, y), Print(glyph))?;
        if rect.width > 1 {
            queue!(
                output,
                MoveTo(rect.x + rect.width.saturating_sub(1), y),
                Print(glyph)
            )?;
        }
    }
    let label = format!(" pane {}{} ", pane.get(), if focused { " *" } else { "" });
    queue!(
        output,
        MoveTo(rect.x.saturating_add(1), rect.y),
        Print(fit(&label, rect.width.saturating_sub(2)))
    )?;
    Ok(())
}

fn draw_text(output: &mut impl Write, rect: Rect, text: &str) -> io::Result<()> {
    let width = usize::from(rect.width.saturating_sub(2));
    let height = usize::from(rect.height.saturating_sub(2));
    let lines: Vec<_> = text.lines().collect();
    let start = lines.len().saturating_sub(height);
    for (row, line) in lines[start..].iter().enumerate() {
        queue!(
            output,
            MoveTo(
                rect.x + 1,
                rect.y + 1 + u16::try_from(row).unwrap_or(u16::MAX)
            ),
            Print(line.chars().take(width).collect::<String>())
        )?;
    }
    Ok(())
}

fn fit(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let mut value: String = text.chars().take(width).collect();
    value.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(value.chars().count()),
    ));
    value
}

struct ScreenGuard;

impl ScreenGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(error) = crossterm::execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetAttribute(Attribute::Reset),
            Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_navigation_and_control_keys_for_a_pty() {
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            b"\x1b[A"
        );
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            vec![2]
        );
    }

    #[test]
    fn content_dimensions_exclude_borders_and_never_reach_zero() {
        assert_eq!(
            content_size(Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }),
            TerminalSize::new(2, 2)
        );
        assert_eq!(
            content_size(Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 10
            }),
            TerminalSize::new(8, 18)
        );
    }

    #[test]
    fn fit_truncates_and_pads_to_the_requested_width() {
        assert_eq!(fit("abcdef", 4), "abcd");
        assert_eq!(fit("ab", 4), "ab  ");
    }
}
