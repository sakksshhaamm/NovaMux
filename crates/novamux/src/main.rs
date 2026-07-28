use std::env;
use std::io::IsTerminal;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use crossterm::terminal;
use novamux::tui;
use novamux::{session_service, session_transport};
use novamux_core::{Rect, Request, Response, Session, SessionName, SplitDirection};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const HELP: &str = "\
NovaMux 0.1.0

USAGE:
    novamux shell
    novamux start
    novamux new SESSION_NAME
    novamux list
    novamux demo [SESSION_NAME]
    novamux help

The shell command opens the platform's fixed default shell in a real PTY.
Type 'exit' or press Control-D to return to your normal terminal.
";

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("start") => run_start(),
        Some("new") => match arguments.next() {
            Some(name) if arguments.next().is_none() => run_new(&name),
            _ => usage_error("new requires exactly one session name"),
        },
        Some("list") if arguments.next().is_none() => run_list(),
        Some("__server") if arguments.next().is_none() => run_server(),
        Some("shell") => run_shell(),
        Some("demo") => run_demo(arguments.next().as_deref().unwrap_or("dev")),
        Some("help" | "--help" | "-h") | None => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("unknown command: {command}\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("{message}\n\n{HELP}");
    ExitCode::from(2)
}

fn run_server() -> ExitCode {
    match session_service::run_daemon() {
        Ok(())
        | Err(session_service::ServiceError::Transport(
            session_transport::TransportError::AlreadyRunning,
        )) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NovaMux session daemon failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_new(value: &str) -> ExitCode {
    let name = match SessionName::parse(value) {
        Ok(name) => name,
        Err(error) => return usage_error(&format!("invalid session name: {error}")),
    };
    match session_service::request_with_autostart(&Request::Create(name)) {
        Ok(Response::Created(name)) => {
            println!("created session {}", name.as_str());
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message, .. }) => {
            eprintln!("NovaMux: {message}");
            ExitCode::FAILURE
        }
        Ok(_) => {
            eprintln!("NovaMux: unexpected daemon response");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("NovaMux: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_list() -> ExitCode {
    match session_service::request_with_autostart(&Request::List) {
        Ok(Response::Sessions(sessions)) => {
            if sessions.is_empty() {
                println!("no sessions");
            } else {
                for session in sessions {
                    println!(
                        "{}\t{} attached",
                        session.name.as_str(),
                        session.attached_clients
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message, .. }) => {
            eprintln!("NovaMux: {message}");
            ExitCode::FAILURE
        }
        Ok(_) => {
            eprintln!("NovaMux: unexpected daemon response");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("NovaMux: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_start() -> ExitCode {
    match tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NovaMux failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_shell() -> ExitCode {
    eprintln!("NovaMux: entering PTY shell (type 'exit' or press Control-D to return)");
    match run_shell_inner() {
        Ok(code) => {
            eprintln!("NovaMux: PTY shell closed");
            ExitCode::from(code)
        }
        Err(error) => {
            eprintln!("NovaMux shell failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_shell_inner() -> Result<u8, Box<dyn std::error::Error>> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let _raw_mode = RawModeGuard::new(interactive)?;
    let initial_size = current_pty_size();
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(initial_size)?;

    let mut command = CommandBuilder::new(default_shell());
    command.env("TERM", "xterm-256color");
    command.env("NOVAMUX", "1");
    if let Ok(current_directory) = env::current_dir() {
        command.cwd(current_directory);
    }
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;

    let output_thread = thread::Builder::new()
        .name("novamux-pty-output".to_owned())
        .spawn(move || -> io::Result<()> {
            let mut stdout = io::stdout().lock();
            copy_and_flush(&mut reader, &mut stdout)
        })?;

    let input_thread = thread::Builder::new()
        .name("novamux-pty-input".to_owned())
        .spawn(move || -> io::Result<()> {
            let mut stdin = io::stdin().lock();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stdin.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                writer.write_all(&buffer[..read])?;
                writer.flush()?;
            }
            Ok(())
        })?;

    let mut last_size = initial_size;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let size = current_pty_size();
        if size != last_size {
            pair.master.resize(size)?;
            last_size = size;
        }
        thread::sleep(Duration::from_millis(50));
    };
    drop(pair.master);

    match output_thread.join() {
        Ok(result) => result?,
        Err(_) => return Err("PTY output worker stopped unexpectedly".into()),
    }

    // A blocked stdin read ends automatically when the process exits. Joining
    // it here would prevent a shell launched from an interactive terminal from
    // returning after the user types `exit`.
    drop(input_thread);

    Ok(status.exit_code().try_into().unwrap_or(1))
}

struct RawModeGuard {
    enabled: bool,
}

impl RawModeGuard {
    fn new(enabled: bool) -> io::Result<Self> {
        if enabled {
            terminal::enable_raw_mode()?;
        }
        Ok(Self { enabled })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            let _ = terminal::disable_raw_mode();
        }
    }
}

fn current_pty_size() -> PtySize {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn copy_and_flush(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
        writer.flush()?;
    }
}

#[cfg(target_os = "windows")]
const fn default_shell() -> &'static str {
    "cmd.exe"
}

#[cfg(target_os = "macos")]
const fn default_shell() -> &'static str {
    "/bin/zsh"
}

#[cfg(all(unix, not(target_os = "macos")))]
const fn default_shell() -> &'static str {
    "/bin/sh"
}

fn run_demo(name: &str) -> ExitCode {
    let name = match SessionName::parse(name) {
        Ok(name) => name,
        Err(error) => {
            eprintln!("invalid session name: {error}");
            return ExitCode::from(2);
        }
    };

    let mut session = Session::new(name);
    session.split_focused(SplitDirection::Horizontal);
    session.split_focused(SplitDirection::Vertical);

    println!("NovaMux session: {}", session.name().as_str());
    println!(
        "screen: 120x36 cells; focused pane: {}",
        session.focused().get()
    );
    println!("┌────────┬────────┬────────┬────────┐");
    println!("│ pane   │ x      │ y      │ size   │");
    println!("├────────┼────────┼────────┼────────┤");
    for (pane, rect) in session.layout(Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 36,
    }) {
        println!(
            "│ {:<6} │ {:<6} │ {:<6} │ {:>3}x{:<3}│",
            pane.get(),
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
    }
    println!("└────────┴────────┴────────┴────────┘");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Default)]
    struct FlushTrackingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for FlushTrackingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn shell_is_an_absolute_fixed_path_on_macos() {
        #[cfg(target_os = "macos")]
        assert_eq!(default_shell(), "/bin/zsh");
    }

    #[test]
    fn pty_output_is_flushed_without_waiting_for_a_newline() {
        let mut input = Cursor::new(b"exit".as_slice());
        let mut output = FlushTrackingWriter::default();
        copy_and_flush(&mut input, &mut output).unwrap();
        assert_eq!(output.bytes, b"exit");
        assert_eq!(output.flushes, 1);
    }
}
