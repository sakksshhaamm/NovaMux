use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::style::{
    Attribute, Color as TerminalColor, Print, ResetColor, SetAttribute, SetBackgroundColor,
    SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use novamux_core::{
    CopyMode, CopyModeAction, InputAction, InputRouter, MultiplexerCommand, PaneId, PaneRegistry,
    PaneSnapshot, Rect, Request, Response, Session, SessionName, SplitDirection,
};
use novamux_terminal::TerminalSize;

use crate::live_pty::LivePty;
use crate::session_service::AttachedClient;
use crate::{
    config,
    config::{Config, Theme},
};

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

    let config = config::load()?;
    let _screen = ScreenGuard::enter()?;
    let mut app = App::new(config)?;
    let result = app.event_loop();
    app.shutdown();
    result
}

/// Attaches the current terminal to a daemon-owned named session.
///
/// # Errors
///
/// Returns if the terminal is non-interactive, daemon communication fails, or
/// terminal rendering/state restoration fails.
pub fn run_attached(name: SessionName) -> AppResult<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("novamux attach requires an interactive terminal".into());
    }
    let config = config::load()?;
    let animation_enabled = animation_enabled(config);
    let animation_start = Instant::now();
    let mut client = AttachedClient::connect(name)?;
    let _screen = ScreenGuard::enter()?;
    let mut router = InputRouter::new();
    let mut copy_mode = CopyMode::default();
    let mut size = terminal::size()?;
    let mut response = client.exchange(&Request::Resize {
        cols: size.0,
        rows: size.1,
    })?;
    let mut stdout = io::stdout().lock();
    loop {
        render_remote(
            &mut stdout,
            size,
            screen(&response)?,
            copy_mode.is_active(),
            animated_theme(config, animation_enabled, animation_start.elapsed()),
        )?;
        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if copy_mode.is_active() {
                        if matches!(copy_mode_action(key), Some(CopyModeAction::Exit)) {
                            copy_mode.exit();
                        }
                        continue;
                    }
                    for byte in key_bytes(key) {
                        response = match router.route(byte) {
                            InputAction::Pending => continue,
                            InputAction::Forward(bytes) => {
                                client.exchange(&Request::Input(bytes))?
                            }
                            InputAction::Command(
                                MultiplexerCommand::Detach | MultiplexerCommand::Quit,
                            ) => {
                                client.detach()?;
                                return Ok(());
                            }
                            InputAction::Command(MultiplexerCommand::EnterCopyMode) => {
                                copy_mode.enter();
                                continue;
                            }
                            InputAction::Command(command) => {
                                client.exchange(&Request::Command(command_code(command)))?
                            }
                        };
                    }
                }
                Event::Resize(cols, rows) => {
                    size = (cols, rows);
                    response = client.exchange(&Request::Resize { cols, rows })?;
                }
                _ => {}
            }
        } else {
            response = client.exchange(&Request::Snapshot)?;
        }
    }
}

fn screen(response: &Response) -> AppResult<&[PaneSnapshot]> {
    match response {
        Response::Screen(panes) => Ok(panes),
        Response::Error { message, .. } => Err(message.clone().into()),
        _ => Err("unexpected daemon screen response".into()),
    }
}

const fn command_code(command: MultiplexerCommand) -> u8 {
    match command {
        MultiplexerCommand::SplitHorizontal => 1,
        MultiplexerCommand::SplitVertical => 2,
        MultiplexerCommand::FocusNext => 3,
        MultiplexerCommand::CloseFocused => 4,
        MultiplexerCommand::Detach
        | MultiplexerCommand::Quit
        | MultiplexerCommand::EnterCopyMode => 0,
    }
}

fn render_remote(
    output: &mut impl Write,
    size: (u16, u16),
    panes: &[PaneSnapshot],
    copy_mode: bool,
    theme: Theme,
) -> AppResult<()> {
    queue!(
        output,
        Hide,
        SetForegroundColor(to_terminal_color(theme.foreground)),
        SetBackgroundColor(to_terminal_color(theme.background)),
        Clear(ClearType::All)
    )?;
    for pane in panes {
        let rect = Rect {
            x: pane.x,
            y: pane.y,
            width: pane.width,
            height: pane.height,
        };
        draw_remote_border(output, rect, pane.id, pane.focused, theme)?;
        if rect.width > 2 && rect.height > 2 {
            queue!(
                output,
                SetForegroundColor(to_terminal_color(theme.foreground)),
                SetBackgroundColor(to_terminal_color(theme.background))
            )?;
            draw_text(output, rect, &pane.text)?;
        }
    }
    queue!(
        output,
        MoveTo(0, size.1.saturating_sub(1)),
        SetForegroundColor(to_terminal_color(theme.status)),
        SetBackgroundColor(to_terminal_color(theme.status_background)),
        Print(fit(
            if copy_mode {
                " COPY MODE (current remote viewport)  Esc/q exit "
            } else {
                " NovaMux attached  Ctrl-B [ copy | d detach | % split | \" split | o focus | x close "
            },
            size.0
        )),
        ResetColor
    )?;
    output.flush()?;
    Ok(())
}

fn draw_remote_border(
    output: &mut impl Write,
    rect: Rect,
    pane: u64,
    focused: bool,
    theme: Theme,
) -> io::Result<()> {
    if rect.width == 0 || rect.height == 0 {
        return Ok(());
    }
    queue!(
        output,
        SetForegroundColor(to_terminal_color(if focused {
            theme.focused_border
        } else {
            theme.pane_border
        })),
        SetBackgroundColor(to_terminal_color(theme.background))
    )?;
    let glyph = if focused { '#' } else { '+' };
    for x in rect.x..rect.x.saturating_add(rect.width) {
        queue!(output, MoveTo(x, rect.y), Print(glyph))?;
        if rect.height > 1 {
            queue!(output, MoveTo(x, rect.y + rect.height - 1), Print(glyph))?;
        }
    }
    for y in rect.y..rect.y.saturating_add(rect.height) {
        queue!(output, MoveTo(rect.x, y), Print(glyph))?;
        if rect.width > 1 {
            queue!(output, MoveTo(rect.x + rect.width - 1, y), Print(glyph))?;
        }
    }
    queue!(
        output,
        MoveTo(rect.x.saturating_add(1), rect.y),
        Print(fit(
            &format!(" pane {pane}{} ", if focused { " *" } else { "" }),
            rect.width.saturating_sub(2)
        ))
    )?;
    Ok(())
}

struct App {
    session: Session,
    panes: PaneRegistry<LivePty>,
    router: InputRouter,
    last_size: (u16, u16),
    copy_mode: CopyMode,
    config: Config,
    animation_enabled: bool,
    animation_start: Instant,
}

impl App {
    fn new(config: Config) -> AppResult<Self> {
        let session = Session::new(SessionName::parse("local")?);
        let mut panes = PaneRegistry::new();
        let size = terminal::size()?;
        panes.reconcile(&session, |_| LivePty::spawn(TerminalSize::new(2, 2)))?;
        let mut app = Self {
            session,
            panes,
            router: InputRouter::new(),
            last_size: size,
            copy_mode: CopyMode::default(),
            config,
            animation_enabled: animation_enabled(config),
            animation_start: Instant::now(),
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
                        if self.copy_mode.is_active() {
                            self.navigate_copy_mode(key, size)?;
                            continue;
                        }
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
            MultiplexerCommand::EnterCopyMode => self.copy_mode.enter(),
            MultiplexerCommand::Detach | MultiplexerCommand::Quit => return Ok(true),
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
        let theme = animated_theme(
            self.config,
            self.animation_enabled,
            self.animation_start.elapsed(),
        );
        queue!(
            output,
            Hide,
            SetForegroundColor(to_terminal_color(theme.foreground)),
            SetBackgroundColor(to_terminal_color(theme.background)),
            Clear(ClearType::All)
        )?;
        for (pane, rect) in self.layout(size) {
            let focused = pane == self.session.focused();
            draw_border(output, rect, pane, focused, theme)?;
            if rect.width > 2 && rect.height > 2 {
                let text = self
                    .panes
                    .get(pane)
                    .map(|runtime| {
                        runtime.with_terminal(|terminal| {
                            if focused && self.copy_mode.is_active() {
                                terminal.viewport_text(self.copy_mode.offset())
                            } else {
                                terminal.visible_text()
                            }
                        })
                    })
                    .transpose()?
                    .unwrap_or_default();
                queue!(
                    output,
                    SetForegroundColor(to_terminal_color(theme.foreground)),
                    SetBackgroundColor(to_terminal_color(theme.background))
                )?;
                draw_text(output, rect, &text)?;
            }
        }
        let status_y = size.1.saturating_sub(1);
        queue!(
            output,
            MoveTo(0, status_y),
            SetForegroundColor(to_terminal_color(theme.status)),
            SetBackgroundColor(to_terminal_color(theme.status_background)),
            Print(fit(self.status_text(), size.0)),
            ResetColor
        )?;
        output.flush()?;
        Ok(())
    }

    fn navigate_copy_mode(&mut self, key: KeyEvent, size: (u16, u16)) -> AppResult<()> {
        let Some(action) = copy_mode_action(key) else {
            return Ok(());
        };
        let maximum = self
            .panes
            .get(self.session.focused())
            .map(|runtime| {
                runtime.with_terminal(novamux_terminal::TerminalBuffer::max_scrollback_offset)
            })
            .transpose()?
            .unwrap_or_default();
        let page_rows = usize::from(size.1.saturating_sub(STATUS_ROWS + 2).max(1));
        self.copy_mode.apply(action, page_rows, maximum);
        Ok(())
    }

    fn status_text(&self) -> &'static str {
        if self.copy_mode.is_active() {
            " COPY MODE  ↑/k older | ↓/j newer | PgUp/PgDn page | Esc/q exit "
        } else {
            " NovaMux  Ctrl-B [ copy | % split | \" split | o focus | x close | q quit "
        }
    }

    fn shutdown(&mut self) {
        for pane in self.session.panes() {
            if let Some(runtime) = self.panes.get_mut(pane) {
                let _ = runtime.terminate();
            }
        }
    }
}

fn copy_mode_action(key: KeyEvent) -> Option<CopyModeAction> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(CopyModeAction::OlderLine),
        KeyCode::Down | KeyCode::Char('j') => Some(CopyModeAction::NewerLine),
        KeyCode::PageUp => Some(CopyModeAction::OlderPage),
        KeyCode::PageDown => Some(CopyModeAction::NewerPage),
        KeyCode::Esc | KeyCode::Char('q') => Some(CopyModeAction::Exit),
        _ => None,
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

fn draw_border(
    output: &mut impl Write,
    rect: Rect,
    pane: PaneId,
    focused: bool,
    theme: Theme,
) -> io::Result<()> {
    if rect.width == 0 || rect.height == 0 {
        return Ok(());
    }
    queue!(
        output,
        SetForegroundColor(to_terminal_color(if focused {
            theme.focused_border
        } else {
            theme.pane_border
        })),
        SetBackgroundColor(to_terminal_color(theme.background))
    )?;
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

const fn to_terminal_color(color: config::Color) -> TerminalColor {
    match color {
        config::Color::Black => TerminalColor::Black,
        config::Color::Red => TerminalColor::Red,
        config::Color::Green => TerminalColor::Green,
        config::Color::Yellow => TerminalColor::Yellow,
        config::Color::Blue => TerminalColor::Blue,
        config::Color::Magenta => TerminalColor::Magenta,
        config::Color::Cyan => TerminalColor::Cyan,
        config::Color::White => TerminalColor::White,
        config::Color::DarkGrey => TerminalColor::DarkGrey,
        config::Color::Grey => TerminalColor::Grey,
        config::Color::Rgb(red, green, blue) => TerminalColor::Rgb {
            r: red,
            g: green,
            b: blue,
        },
    }
}

fn animation_enabled(config: Config) -> bool {
    config.animation == config::Animation::Subtle
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

fn animated_theme(config: Config, enabled: bool, elapsed: Duration) -> Theme {
    if !enabled {
        return config.theme;
    }
    let frame = (elapsed.as_millis() / 125) as usize % 4;
    let mut theme = config.theme;
    theme.focused_border = pulse(theme.focused_border, frame);
    theme.status_background = pulse(theme.status_background, frame);
    theme
}

const fn pulse(color: config::Color, frame: usize) -> config::Color {
    let amount = match frame % 4 {
        1 | 3 => 10,
        2 => 18,
        _ => 0,
    };
    match color {
        config::Color::Rgb(red, green, blue) => config::Color::Rgb(
            red.saturating_add(amount),
            green.saturating_add(amount),
            blue.saturating_add(amount),
        ),
        named => named,
    }
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
            ResetColor,
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

    #[test]
    fn maps_copy_mode_navigation_without_forwarding_shell_input() {
        assert_eq!(
            copy_mode_action(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(CopyModeAction::OlderPage)
        );
        assert_eq!(
            copy_mode_action(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(CopyModeAction::Exit)
        );
        assert_eq!(
            copy_mode_action(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn subtle_animation_frames_are_bounded_and_repeat() {
        let config = Config {
            theme: Theme::sakura(),
            animation: config::Animation::Subtle,
        };
        assert_eq!(
            animated_theme(config, true, Duration::ZERO),
            animated_theme(config, true, Duration::from_millis(500))
        );
        assert_eq!(
            animated_theme(config, false, Duration::from_millis(250)),
            config.theme
        );
        assert_eq!(
            pulse(config::Color::Rgb(250, 250, 250), 2),
            config::Color::Rgb(255, 255, 255)
        );
    }
}
