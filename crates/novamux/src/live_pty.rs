//! Ownership and lifecycle management for one live pane PTY.

use std::env;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use novamux_terminal::{TerminalBuffer, TerminalSize};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

type RuntimeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// A live shell and its parsed, bounded terminal state.
///
/// The fixed platform shell is launched directly, never through command-string
/// evaluation. Dropping the runtime terminates the child if it is still alive.
pub struct LivePty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    terminal: Arc<Mutex<TerminalBuffer>>,
    reader_thread: Option<JoinHandle<io::Result<()>>>,
}

impl LivePty {
    /// Launches the platform shell in the process's current directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the current directory cannot be read or PTY setup,
    /// shell creation, or reader-worker creation fails.
    pub fn spawn(size: TerminalSize) -> RuntimeResult<Self> {
        let current_directory = env::current_dir()?;
        Self::spawn_in(size, &current_directory)
    }

    /// Launches the platform shell in an existing directory.
    ///
    /// # Errors
    ///
    /// Returns an error when `directory` is not a directory or when PTY setup,
    /// shell creation, or reader-worker creation fails.
    pub fn spawn_in(size: TerminalSize, directory: &Path) -> RuntimeResult<Self> {
        let canonical_directory = directory
            .canonicalize()
            .map_err(|_| Box::new(InvalidWorkingDirectory))?;
        if !canonical_directory.is_dir() {
            return Err(Box::new(InvalidWorkingDirectory));
        }

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(to_pty_size(size))?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let mut command = CommandBuilder::new(default_shell());
        command.env("TERM", "xterm-256color");
        command.env("NOVAMUX", "1");
        command.cwd(canonical_directory);

        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let terminal = Arc::new(Mutex::new(TerminalBuffer::new(size)));
        let reader_terminal = Arc::clone(&terminal);
        let reader_thread = match thread::Builder::new()
            .name("novamux-pane-output".to_owned())
            .spawn(move || read_output(&mut reader, &reader_terminal))
        {
            Ok(thread) => thread,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Box::new(error));
            }
        };

        Ok(Self {
            master: pair.master,
            child,
            writer,
            terminal,
            reader_thread: Some(reader_thread),
        })
    }

    /// Writes bytes to the shell attached to this pane.
    ///
    /// # Errors
    ///
    /// Returns an error if the PTY cannot accept or flush all bytes.
    pub fn write(&mut self, bytes: &[u8]) -> RuntimeResult<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resizes both the operating-system PTY and its parsed terminal buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform PTY resize fails or terminal state is
    /// unavailable because another worker panicked while holding its lock.
    pub fn resize(&mut self, size: TerminalSize) -> RuntimeResult<()> {
        self.master.resize(to_pty_size(size))?;
        lock_terminal(&self.terminal)?.resize(size);
        Ok(())
    }

    /// Runs a read-only operation against the pane's latest terminal state.
    ///
    /// Keeping the lock inside this call prevents callers from retaining it
    /// while performing rendering or other blocking work.
    ///
    /// # Errors
    ///
    /// Returns an error if another worker panicked while holding the terminal
    /// state lock.
    pub fn with_terminal<T>(&self, inspect: impl FnOnce(&TerminalBuffer) -> T) -> RuntimeResult<T> {
        let terminal = lock_terminal(&self.terminal)?;
        Ok(inspect(&terminal))
    }

    /// Returns the child exit code when the shell has exited.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot query the child process.
    pub fn try_wait(&mut self) -> RuntimeResult<Option<u32>> {
        Ok(self.child.try_wait()?.map(|status| status.exit_code()))
    }

    /// Terminates the child shell and releases the reader worker.
    ///
    /// # Errors
    ///
    /// Returns an error if the child cannot be queried, terminated, or reaped,
    /// or if the PTY reader fails during shutdown.
    pub fn terminate(&mut self) -> RuntimeResult<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
            let _ = self.child.wait()?;
        }
        self.finish_reader()
    }

    fn finish_reader(&mut self) -> RuntimeResult<()> {
        drop(self.writer.flush());
        if let Some(thread) = self.reader_thread.take() {
            match thread.join() {
                Ok(result) => result?,
                Err(_) => return Err(Box::new(ReaderThreadPanicked)),
            }
        }
        Ok(())
    }
}

impl Drop for LivePty {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = self.finish_reader();
    }
}

fn read_output(reader: &mut impl Read, terminal: &Arc<Mutex<TerminalBuffer>>) -> io::Result<()> {
    let mut bytes = [0_u8; 4096];
    loop {
        let count = reader.read(&mut bytes)?;
        if count == 0 {
            return Ok(());
        }
        let mut terminal = terminal
            .lock()
            .map_err(|_| io::Error::other("terminal buffer lock was poisoned"))?;
        terminal.process(&bytes[..count]);
    }
}

fn lock_terminal(
    terminal: &Arc<Mutex<TerminalBuffer>>,
) -> RuntimeResult<MutexGuard<'_, TerminalBuffer>> {
    terminal
        .lock()
        .map_err(|_| Box::new(TerminalLockPoisoned) as Box<dyn Error + Send + Sync>)
}

const fn to_pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[derive(Debug)]
struct InvalidWorkingDirectory;

impl fmt::Display for InvalidWorkingDirectory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PTY working directory does not exist or is not a directory")
    }
}

impl Error for InvalidWorkingDirectory {}

#[derive(Debug)]
struct TerminalLockPoisoned;

impl fmt::Display for TerminalLockPoisoned {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal buffer lock was poisoned")
    }
}

impl Error for TerminalLockPoisoned {}

#[derive(Debug)]
struct ReaderThreadPanicked;

impl fmt::Display for ReaderThreadPanicked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PTY reader worker stopped unexpectedly")
    }
}

impl Error for ReaderThreadPanicked {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn rejects_a_missing_working_directory() {
        let missing = env::temp_dir().join(format!(
            "novamux-missing-working-directory-{}",
            std::process::id()
        ));
        let result = LivePty::spawn_in(TerminalSize::new(10, 20), &missing);
        assert!(result.is_err());
    }

    #[test]
    fn converts_terminal_dimensions_without_pixel_assumptions() {
        let size = to_pty_size(TerminalSize::new(12, 34));
        assert_eq!(size.rows, 12);
        assert_eq!(size.cols, 34);
        assert_eq!(size.pixel_width, 0);
        assert_eq!(size.pixel_height, 0);
    }

    #[test]
    fn shell_path_is_fixed_on_macos() {
        #[cfg(target_os = "macos")]
        assert_eq!(default_shell(), "/bin/zsh");
    }

    #[test]
    fn live_shell_accepts_input_and_feeds_terminal_buffer() {
        let mut runtime = LivePty::spawn(TerminalSize::new(10, 40)).unwrap();
        runtime
            .write(b"printf 'NOVAMUX_LIVE_PTY_OK\\n'; exit\n")
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while runtime.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "shell did not exit in time");
            thread::sleep(Duration::from_millis(10));
        }
        runtime.terminate().unwrap();

        let output = runtime.with_terminal(TerminalBuffer::visible_text).unwrap();
        assert!(output.contains("NOVAMUX_LIVE_PTY_OK"), "{output:?}");
    }
}
