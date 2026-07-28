//! Per-user detached-session daemon and its bounded local client.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use novamux_core::{
    ErrorCode, MAX_FRAME_BYTES, MAX_LISTED_SESSIONS, PaneRegistry, PaneSnapshot, Rect, Request,
    Response, Session, SessionInfo, SessionName, SplitDirection, decode_request, decode_response,
    encode_request, encode_response,
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
    attached: bool,
    viewport: (u16, u16),
}

impl HostedSession {
    fn create(name: SessionName) -> Result<Self, ServiceError> {
        let session = Session::new(name);
        let mut panes = PaneRegistry::new();
        panes
            .reconcile(&session, |_| LivePty::spawn(TerminalSize::new(24, 80)))
            .map_err(|error| ServiceError::Runtime(error.to_string()))?;
        Ok(Self {
            session,
            panes,
            attached: false,
            viewport: (80, 24),
        })
    }

    fn info(&self) -> SessionInfo {
        debug_assert_eq!(self.panes.len(), self.session.panes().len());
        SessionInfo {
            name: self.session.name().clone(),
            attached_clients: u16::from(self.attached),
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), ServiceError> {
        self.viewport = (cols.max(2), rows.max(2));
        for (pane, rect) in self.layout() {
            if let Some(runtime) = self.panes.get_mut(pane) {
                runtime
                    .resize(TerminalSize::new(
                        rect.height.saturating_sub(2).max(2),
                        rect.width.saturating_sub(2).max(2),
                    ))
                    .map_err(|error| ServiceError::Runtime(error.to_string()))?;
            }
        }
        Ok(())
    }

    fn layout(&self) -> Vec<(novamux_core::PaneId, Rect)> {
        self.session.layout(Rect {
            x: 0,
            y: 0,
            width: self.viewport.0,
            height: self.viewport.1.saturating_sub(1),
        })
    }

    fn snapshot(&self) -> Result<Response, ServiceError> {
        let layout = self.layout();
        let allowance = 6_500 / layout.len().max(1);
        let mut panes = Vec::with_capacity(layout.len());
        for (pane, rect) in layout {
            let mut text = self
                .panes
                .get(pane)
                .map(|runtime| {
                    runtime.with_terminal(novamux_terminal::TerminalBuffer::visible_text)
                })
                .transpose()
                .map_err(|error| ServiceError::Runtime(error.to_string()))?
                .unwrap_or_default();
            truncate_utf8(&mut text, allowance);
            panes.push(PaneSnapshot {
                id: pane.get(),
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                focused: pane == self.session.focused(),
                text,
            });
        }
        Ok(Response::Screen(panes))
    }

    fn command(&mut self, command: u8) -> Result<(), ServiceError> {
        match command {
            1 | 2 => {
                let mut candidate = self.session.clone();
                candidate.split_focused(if command == 1 {
                    SplitDirection::Horizontal
                } else {
                    SplitDirection::Vertical
                });
                self.panes
                    .reconcile(&candidate, |_| LivePty::spawn(TerminalSize::new(2, 2)))
                    .map_err(|error| ServiceError::Runtime(error.to_string()))?;
                self.session = candidate;
                self.resize(self.viewport.0, self.viewport.1)?;
            }
            3 => self.session.focus_next(),
            4 => {
                if self.session.close_focused() {
                    for (_, mut pane) in self
                        .panes
                        .reconcile(&self.session, |_| LivePty::spawn(TerminalSize::new(2, 2)))
                        .map_err(|error| ServiceError::Runtime(error.to_string()))?
                    {
                        pane.terminate()
                            .map_err(|error| ServiceError::Runtime(error.to_string()))?;
                    }
                }
            }
            _ => return Err(ServiceError::Protocol("unknown attached command".into())),
        }
        Ok(())
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
    let sessions = Arc::new(Mutex::new(BTreeMap::<String, HostedSession>::new()));
    loop {
        let stream = match endpoint.accept_authenticated() {
            Ok(stream) => stream,
            Err(TransportError::PeerIdentityMismatch) => continue,
            Err(error) => return Err(error.into()),
        };
        let sessions = Arc::clone(&sessions);
        thread::Builder::new()
            .name("novamux-session-client".into())
            .spawn(move || {
                let _ = serve_connection(stream, &sessions);
            })?;
    }
}

#[cfg(unix)]
fn serve_connection(
    mut stream: std::os::unix::net::UnixStream,
    sessions: &Arc<Mutex<BTreeMap<String, HostedSession>>>,
) -> Result<(), ServiceError> {
    let request = read_request(&mut stream)?;
    let Request::Attach(name) = request else {
        let mut guard = sessions
            .lock()
            .map_err(|_| ServiceError::Runtime("session lock poisoned".into()))?;
        let response = dispatch(&mut guard, request);
        return write_response(&mut stream, &response);
    };
    {
        let mut sessions = sessions
            .lock()
            .map_err(|_| ServiceError::Runtime("session lock poisoned".into()))?;
        let Some(hosted) = sessions.get_mut(name.as_str()) else {
            return write_response(
                &mut stream,
                &service_error(ErrorCode::NotFound, "session not found"),
            );
        };
        if hosted.attached {
            return write_response(
                &mut stream,
                &service_error(
                    ErrorCode::Conflict,
                    "session already has an attached client",
                ),
            );
        }
        hosted.attached = true;
    }
    write_response(&mut stream, &Response::Attached(name.clone()))?;
    let result = attached_loop(&mut stream, sessions, &name);
    if let Ok(mut sessions) = sessions.lock()
        && let Some(hosted) = sessions.get_mut(name.as_str())
    {
        hosted.attached = false;
    }
    result
}

#[cfg(unix)]
fn attached_loop(
    stream: &mut std::os::unix::net::UnixStream,
    sessions: &Arc<Mutex<BTreeMap<String, HostedSession>>>,
    name: &SessionName,
) -> Result<(), ServiceError> {
    loop {
        let request = read_request(stream)?;
        let mut sessions = sessions
            .lock()
            .map_err(|_| ServiceError::Runtime("session lock poisoned".into()))?;
        let hosted = sessions
            .get_mut(name.as_str())
            .ok_or_else(|| ServiceError::Runtime("attached session disappeared".into()))?;
        let response = match request {
            Request::Snapshot => hosted.snapshot(),
            Request::Input(bytes) => {
                hosted
                    .panes
                    .get_mut(hosted.session.focused())
                    .ok_or_else(|| ServiceError::Runtime("focused pane is missing".into()))?
                    .write(&bytes)
                    .map_err(|error| ServiceError::Runtime(error.to_string()))?;
                hosted.snapshot()
            }
            Request::Resize { cols, rows } => {
                hosted.resize(cols, rows)?;
                hosted.snapshot()
            }
            Request::Command(command) => {
                hosted.command(command)?;
                hosted.snapshot()
            }
            Request::Detach(detach_name) if detach_name == *name => {
                write_response(stream, &Response::Detached(name.clone()))?;
                return Ok(());
            }
            _ => Ok(service_error(
                ErrorCode::Conflict,
                "invalid attached request",
            )),
        }?;
        write_response(stream, &response)?;
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
        Request::Attach(_)
        | Request::Detach(_)
        | Request::Input(_)
        | Request::Resize { .. }
        | Request::Command(_)
        | Request::Snapshot => service_error(
            ErrorCode::Conflict,
            "attach and detach streaming are not implemented",
        ),
    }
}

fn truncate_utf8(value: &mut String, maximum: usize) {
    if value.len() <= maximum {
        return;
    }
    let mut start = value.len() - maximum;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    *value = value[start..].to_owned();
}

/// One persistent authenticated attachment to a daemon-owned session.
#[cfg(unix)]
pub struct AttachedClient {
    stream: std::os::unix::net::UnixStream,
    name: SessionName,
}

#[cfg(unix)]
impl AttachedClient {
    /// Connects to one existing session, starting the fixed daemon if needed.
    ///
    /// # Errors
    ///
    /// Returns when daemon startup, authenticated transport, or attach fails.
    pub fn connect(name: SessionName) -> Result<Self, ServiceError> {
        request_with_autostart(&Request::Ping(0))?;
        let mut stream =
            std::os::unix::net::UnixStream::connect(default_runtime_dir()?.join("control.sock"))?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        write_request(&mut stream, &Request::Attach(name.clone()))?;
        match read_response(&mut stream)? {
            Response::Attached(attached) if attached == name => Ok(Self { stream, name }),
            Response::Error { message, .. } => Err(ServiceError::Protocol(message)),
            _ => Err(ServiceError::Protocol("unexpected attach response".into())),
        }
    }

    /// Exchanges one bounded request and response on this attachment.
    ///
    /// # Errors
    ///
    /// Returns when encoding, transport, or response decoding fails.
    pub fn exchange(&mut self, request: &Request) -> Result<Response, ServiceError> {
        write_request(&mut self.stream, request)?;
        read_response(&mut self.stream)
    }

    /// Explicitly releases this attachment without terminating its session.
    ///
    /// # Errors
    ///
    /// Returns when the detach exchange fails or is rejected.
    pub fn detach(mut self) -> Result<(), ServiceError> {
        let name = self.name.clone();
        match self.exchange(&Request::Detach(name))? {
            Response::Detached(_) => Ok(()),
            Response::Error { message, .. } => Err(ServiceError::Protocol(message)),
            _ => Err(ServiceError::Protocol("unexpected detach response".into())),
        }
    }
}

fn write_request(writer: &mut impl Write, request: &Request) -> Result<(), ServiceError> {
    let frame =
        encode_request(request).map_err(|error| ServiceError::Protocol(error.to_string()))?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

fn read_response(reader: &mut impl Read) -> Result<Response, ServiceError> {
    let frame = read_frame(reader)?;
    decode_response(&frame).map_err(|error| ServiceError::Protocol(error.to_string()))
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
    #[cfg(unix)]
    use std::os::unix::net::UnixStream;

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

    #[cfg(unix)]
    fn attached_pair(
        sessions: &Arc<Mutex<BTreeMap<String, HostedSession>>>,
        session_name: &SessionName,
    ) -> (AttachedClient, thread::JoinHandle<()>) {
        let (server, client) = UnixStream::pair().unwrap();
        server.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        server.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        client.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        client.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        let shared = Arc::clone(sessions);
        let worker = thread::spawn(move || {
            let _ = serve_connection(server, &shared);
        });
        let mut attached = AttachedClient {
            stream: client,
            name: session_name.clone(),
        };
        write_request(&mut attached.stream, &Request::Attach(session_name.clone())).unwrap();
        assert_eq!(
            read_response(&mut attached.stream).unwrap(),
            Response::Attached(session_name.clone())
        );
        (attached, worker)
    }

    #[cfg(unix)]
    #[test]
    fn detach_and_reattach_preserve_live_output() {
        let session_name = name("lifecycle");
        let mut map = BTreeMap::new();
        map.insert(
            session_name.as_str().to_owned(),
            HostedSession::create(session_name.clone()).unwrap(),
        );
        let sessions = Arc::new(Mutex::new(map));

        let (mut first, first_worker) = attached_pair(&sessions, &session_name);
        let marker = format!("NOVAMUX_REATTACH_{}\n", std::process::id());
        let mut response = first
            .exchange(&Request::Input(marker.as_bytes().to_vec()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !screen_contains(&response, marker.trim()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
            response = first.exchange(&Request::Snapshot).unwrap();
        }
        assert!(screen_contains(&response, marker.trim()));
        first.detach().unwrap();
        first_worker.join().unwrap();

        let (mut second, second_worker) = attached_pair(&sessions, &session_name);
        let response = second.exchange(&Request::Snapshot).unwrap();
        assert!(screen_contains(&response, marker.trim()));
        second.detach().unwrap();
        second_worker.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn disconnect_releases_attachment_and_second_live_client_is_rejected() {
        let session_name = name("exclusive");
        let mut map = BTreeMap::new();
        map.insert(
            session_name.as_str().to_owned(),
            HostedSession::create(session_name.clone()).unwrap(),
        );
        let sessions = Arc::new(Mutex::new(map));
        let (first, first_worker) = attached_pair(&sessions, &session_name);

        let (server, mut client) = UnixStream::pair().unwrap();
        let shared = Arc::clone(&sessions);
        let second_worker = thread::spawn(move || {
            let _ = serve_connection(server, &shared);
        });
        write_request(&mut client, &Request::Attach(session_name.clone())).unwrap();
        assert!(matches!(
            read_response(&mut client).unwrap(),
            Response::Error {
                code: ErrorCode::Conflict,
                ..
            }
        ));
        second_worker.join().unwrap();

        drop(first);
        first_worker.join().unwrap();
        assert!(!sessions.lock().unwrap().get("exclusive").unwrap().attached);
    }

    fn screen_contains(response: &Response, needle: &str) -> bool {
        matches!(response, Response::Screen(panes) if panes.iter().any(|pane| pane.text.contains(needle)))
    }
}
