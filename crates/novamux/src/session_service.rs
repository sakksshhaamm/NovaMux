//! Per-user detached-session daemon and its bounded local client.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use novamux_core::{
    ErrorCode, MAX_FRAME_BYTES, MAX_LISTED_SESSIONS, PaneRegistry, Request, Response, Session,
    SessionInfo, SessionName, decode_request, decode_response, encode_request, encode_response,
};
use novamux_terminal::TerminalSize;

use crate::live_pty::LivePty;
use crate::session_transport::{IO_TIMEOUT, LocalEndpoint, TransportError, default_runtime_dir};

const HEADER_BYTES: usize = 10;
const START_TIMEOUT: Duration = Duration::from_secs(3);
const START_RETRY: Duration = Duration::from_millis(25);

/// A daemon-owned session whose PTYs survive client process exit.
pub struct HostedSession {
    session: Session,
    panes: PaneRegistry<LivePty>,
}

impl HostedSession {
    fn create(name: SessionName) -> Result<Self, ServiceError> {
        let session = Session::new(name);
        let mut panes = PaneRegistry::new();
        panes
            .reconcile(&session, |_| LivePty::spawn(TerminalSize::new(24, 80)))
            .map_err(|error| ServiceError::Runtime(error.to_string()))?;
        Ok(Self { session, panes })
    }

    fn info(&self) -> SessionInfo {
        debug_assert_eq!(self.panes.len(), self.session.panes().len());
        SessionInfo {
            name: self.session.name().clone(),
            attached_clients: 0,
        }
    }
}

/// Session-service failures.
#[derive(Debug)]
pub enum ServiceError {
    /// Local endpoint setup or authentication failed.
    Transport(TransportError),
    /// Protocol data was malformed or could not be encoded.
    Protocol(String),
    /// Process or stream I/O failed.
    Io(io::Error),
    /// A PTY runtime could not be created.
    Runtime(String),
    /// The daemon did not become ready within its fixed deadline.
    StartTimeout,
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Protocol(error) => write!(formatter, "session protocol failed: {error}"),
            Self::Io(error) => write!(formatter, "session service I/O failed: {error}"),
            Self::Runtime(error) => write!(formatter, "session runtime failed: {error}"),
            Self::StartTimeout => formatter.write_str("session daemon did not become ready"),
        }
    }
}

impl Error for ServiceError {}

impl From<TransportError> for ServiceError {
    fn from(value: TransportError) -> Self {
        Self::Transport(value)
    }
}

impl From<io::Error> for ServiceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Runs the private daemon entry point in the current process.
///
/// # Errors
///
/// Returns when endpoint setup or an accepted request fails.
pub fn run_daemon() -> Result<(), ServiceError> {
    let endpoint = LocalEndpoint::bind(&default_runtime_dir()?)?;
    let mut sessions = BTreeMap::<String, HostedSession>::new();
    loop {
        let mut stream = match endpoint.accept_authenticated() {
            Ok(stream) => stream,
            Err(TransportError::PeerIdentityMismatch) => continue,
            Err(error) => return Err(error.into()),
        };
        let response = match read_request(&mut stream) {
            Ok(request) => dispatch(&mut sessions, request),
            Err(error) => Response::Error {
                code: ErrorCode::Internal,
                message: bounded_message(&error.to_string()),
            },
        };
        // A same-user client disappearing mid-response must not terminate all
        // hosted sessions.
        let _ = write_response(&mut stream, &response);
    }
}

/// Sends a request, starting the fixed current executable as daemon if needed.
///
/// # Errors
///
/// Returns if the daemon cannot start or communication fails.
pub fn request_with_autostart(request: &Request) -> Result<Response, ServiceError> {
    match send_request(request) {
        Ok(response) => Ok(response),
        Err(ServiceError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            spawn_daemon()?;
            let deadline = Instant::now() + START_TIMEOUT;
            loop {
                match send_request(request) {
                    Ok(response) => return Ok(response),
                    Err(ServiceError::Io(error))
                        if Instant::now() < deadline
                            && matches!(
                                error.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                            ) =>
                    {
                        thread::sleep(START_RETRY);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Err(error) => Err(error),
    }
}

fn spawn_daemon() -> Result<(), ServiceError> {
    let executable = std::env::current_exe()?;
    Command::new(executable)
        .arg("__server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(unix)]
fn send_request(request: &Request) -> Result<Response, ServiceError> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(default_runtime_dir()?.join("control.sock"))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let frame =
        encode_request(request).map_err(|error| ServiceError::Protocol(error.to_string()))?;
    stream.write_all(&frame)?;
    let response = read_frame(&mut stream)?;
    decode_response(&response).map_err(|error| ServiceError::Protocol(error.to_string()))
}

#[cfg(not(unix))]
fn send_request(_request: &Request) -> Result<Response, ServiceError> {
    Err(ServiceError::Transport(TransportError::UnsupportedPlatform))
}

fn read_request(reader: &mut impl Read) -> Result<Request, ServiceError> {
    let frame = read_frame(reader)?;
    decode_request(&frame).map_err(|error| ServiceError::Protocol(error.to_string()))
}

fn write_response(writer: &mut impl Write, response: &Response) -> Result<(), ServiceError> {
    let frame =
        encode_response(response).map_err(|error| ServiceError::Protocol(error.to_string()))?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, ServiceError> {
    let mut header = [0_u8; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let payload_len = u32::from_be_bytes(
        header[6..10]
            .try_into()
            .expect("fixed header slice has four bytes"),
    ) as usize;
    let total = HEADER_BYTES
        .checked_add(payload_len)
        .filter(|length| *length <= MAX_FRAME_BYTES)
        .ok_or_else(|| ServiceError::Protocol("frame exceeds size limit".into()))?;
    let mut frame = vec![0; total];
    frame[..HEADER_BYTES].copy_from_slice(&header);
    reader.read_exact(&mut frame[HEADER_BYTES..])?;
    Ok(frame)
}

fn dispatch(sessions: &mut BTreeMap<String, HostedSession>, request: Request) -> Response {
    match request {
        Request::Ping(nonce) => Response::Pong(nonce),
        Request::List => Response::Sessions(sessions.values().map(HostedSession::info).collect()),
        Request::Create(name) => {
            if sessions.contains_key(name.as_str()) {
                return service_error(ErrorCode::AlreadyExists, "session already exists");
            }
            if sessions.len() >= MAX_LISTED_SESSIONS {
                return service_error(ErrorCode::Conflict, "session limit reached");
            }
            match HostedSession::create(name.clone()) {
                Ok(hosted) => {
                    sessions.insert(name.as_str().to_owned(), hosted);
                    Response::Created(name)
                }
                Err(error) => service_error(ErrorCode::Internal, &error.to_string()),
            }
        }
        Request::Attach(_) | Request::Detach(_) => service_error(
            ErrorCode::Conflict,
            "attach and detach streaming are not implemented",
        ),
    }
}

fn service_error(code: ErrorCode, message: &str) -> Response {
    Response::Error {
        code,
        message: bounded_message(message),
    }
}

fn bounded_message(message: &str) -> String {
    if message.len() <= 256 {
        return message.to_owned();
    }
    let mut end = 256;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> SessionName {
        SessionName::parse(value).unwrap()
    }

    #[test]
    fn ping_is_correlated() {
        assert_eq!(
            dispatch(&mut BTreeMap::new(), Request::Ping(42)),
            Response::Pong(42)
        );
    }

    #[test]
    fn malformed_length_is_rejected_before_allocation() {
        let mut frame = [0_u8; HEADER_BYTES];
        frame[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            read_frame(&mut &frame[..]),
            Err(ServiceError::Protocol(_))
        ));
    }

    #[test]
    fn duplicate_session_is_rejected_without_replacing_original() {
        let mut sessions = BTreeMap::new();
        assert_eq!(
            dispatch(&mut sessions, Request::Create(name("one"))),
            Response::Created(name("one"))
        );
        assert!(matches!(
            dispatch(&mut sessions, Request::Create(name("one"))),
            Response::Error {
                code: ErrorCode::AlreadyExists,
                ..
            }
        ));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn attach_is_honestly_rejected() {
        assert!(matches!(
            dispatch(&mut BTreeMap::new(), Request::Attach(name("one"))),
            Response::Error {
                code: ErrorCode::Conflict,
                ..
            }
        ));
    }

    #[test]
    fn error_text_is_bounded_by_encoded_bytes() {
        let message = "é".repeat(200);
        let bounded = bounded_message(&message);
        assert!(bounded.len() <= 256);
        assert!(bounded.is_char_boundary(bounded.len()));
    }
}
