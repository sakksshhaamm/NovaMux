use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::thread;

use novamux_core::{Rect, Session, SessionName, SplitDirection};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const HELP: &str = "\
NovaMux 0.1.0

USAGE:
    novamux shell
    novamux demo [SESSION_NAME]
    novamux help

The shell command opens the platform's fixed default shell in a real PTY.
Type 'exit' or press Control-D to return to your normal terminal.
";

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
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
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

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
            io::copy(&mut reader, &mut stdout)?;
            stdout.flush()
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

    let status = child.wait()?;
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

    #[test]
    fn shell_is_an_absolute_fixed_path_on_macos() {
        #[cfg(target_os = "macos")]
        assert_eq!(default_shell(), "/bin/zsh");
    }
}
