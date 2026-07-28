use std::env;
use std::process::ExitCode;

use novamux_core::{Rect, Session, SessionName, SplitDirection};

const HELP: &str = "\
NovaMux 0.1.0

USAGE:
    novamux demo [SESSION_NAME]
    novamux help

The demo renders a deterministic screen preview of the core pane engine.
No shell commands are executed.
";

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
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
