//! Bounded, platform-independent messages for the local session service.
//!
//! The codec is deliberately small and dependency-free. It defines messages
//! only; transport framing, peer authentication, and daemon behavior belong to
//! later layers.

use std::fmt;

use crate::{SessionName, SessionNameError};

const MAGIC: &[u8; 4] = b"NVMX";
const HEADER_BYTES: usize = 10;

/// Version of the session protocol implemented by this crate.
pub const PROTOCOL_VERSION: u8 = 1;
/// Largest complete encoded frame accepted by the codec.
pub const MAX_FRAME_BYTES: usize = 8 * 1024;
/// Largest session collection accepted in one list response.
pub const MAX_LISTED_SESSIONS: usize = 64;
/// Largest UTF-8 error description accepted from a peer.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 256;

const CREATE: u8 = 1;
const LIST: u8 = 2;
const ATTACH: u8 = 3;
const DETACH: u8 = 4;
const PING: u8 = 5;
const CREATED: u8 = 0x81;
const SESSIONS: u8 = 0x82;
const ATTACHED: u8 = 0x83;
const DETACHED: u8 = 0x84;
const PONG: u8 = 0x85;
const ERROR: u8 = 0xff;

/// A request sent by an attached client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Create a new named session.
    Create(SessionName),
    /// List sessions belonging to the current operating-system user.
    List,
    /// Attach to an existing named session.
    Attach(SessionName),
    /// Detach this client from a named session.
    Detach(SessionName),
    /// Verify service liveness while correlating the response.
    Ping(u64),
}

/// Public information returned for a session listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionInfo {
    /// Validated session name.
    pub name: SessionName,
    /// Number of clients currently attached.
    pub attached_clients: u16,
}

/// Stable machine-readable error categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ErrorCode {
    /// The requested session already exists.
    AlreadyExists = 1,
    /// The requested session does not exist.
    NotFound = 2,
    /// The peer is not authorized for this operation.
    PermissionDenied = 3,
    /// The request is valid but conflicts with current service state.
    Conflict = 4,
    /// An internal service failure occurred.
    Internal = 5,
}

impl ErrorCode {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::AlreadyExists),
            2 => Some(Self::NotFound),
            3 => Some(Self::PermissionDenied),
            4 => Some(Self::Conflict),
            5 => Some(Self::Internal),
            _ => None,
        }
    }
}

/// A structured response sent by the session service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Response {
    /// A session was created.
    Created(SessionName),
    /// Current sessions, in daemon-defined stable order.
    Sessions(Vec<SessionInfo>),
    /// The client attached to a session.
    Attached(SessionName),
    /// The client detached from a session.
    Detached(SessionName),
    /// Reply to a liveness check.
    Pong(u64),
    /// A request failed without exposing unbounded diagnostic data.
    Error {
        /// Machine-readable category.
        code: ErrorCode,
        /// Human-readable, bounded UTF-8 description.
        message: String,
    },
}

/// Why an IPC frame could not be encoded or decoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The input is shorter than a complete header or declared payload.
    Truncated,
    /// The frame does not carry the `NovaMux` protocol magic.
    InvalidMagic,
    /// The peer uses an unsupported protocol version.
    UnsupportedVersion(u8),
    /// The message type is unknown or invalid for the selected direction.
    UnknownMessageType(u8),
    /// The declared frame exceeds [`MAX_FRAME_BYTES`].
    FrameTooLarge,
    /// The frame contains bytes beyond its declared payload.
    TrailingBytes,
    /// A length-prefixed field is malformed or exceeds its field limit.
    InvalidLength,
    /// A text field is not valid UTF-8.
    InvalidUtf8,
    /// A session name failed its portable validation rules.
    InvalidSessionName(SessionNameError),
    /// An error response contains an unknown code.
    UnknownErrorCode(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("protocol frame is truncated"),
            Self::InvalidMagic => formatter.write_str("invalid protocol magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::UnknownMessageType(kind) => write!(formatter, "unknown message type {kind}"),
            Self::FrameTooLarge => formatter.write_str("protocol frame exceeds its size limit"),
            Self::TrailingBytes => formatter.write_str("protocol frame contains trailing bytes"),
            Self::InvalidLength => formatter.write_str("protocol field has an invalid length"),
            Self::InvalidUtf8 => formatter.write_str("protocol text is not valid UTF-8"),
            Self::InvalidSessionName(error) => write!(formatter, "invalid session name: {error}"),
            Self::UnknownErrorCode(code) => write!(formatter, "unknown protocol error code {code}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Encodes a request as one complete bounded frame.
///
/// # Errors
///
/// Returns [`DecodeError::FrameTooLarge`] if the result exceeds the protocol
/// maximum.
pub fn encode_request(request: &Request) -> Result<Vec<u8>, DecodeError> {
    let mut payload = Vec::new();
    let kind = match request {
        Request::Create(name) => {
            put_name(&mut payload, name);
            CREATE
        }
        Request::List => LIST,
        Request::Attach(name) => {
            put_name(&mut payload, name);
            ATTACH
        }
        Request::Detach(name) => {
            put_name(&mut payload, name);
            DETACH
        }
        Request::Ping(nonce) => {
            payload.extend_from_slice(&nonce.to_be_bytes());
            PING
        }
    };
    frame(kind, &payload)
}

/// Decodes exactly one complete request frame.
///
/// # Errors
///
/// Rejects malformed, oversized, unsupported, or trailing data.
pub fn decode_request(bytes: &[u8]) -> Result<Request, DecodeError> {
    let (kind, payload) = unframe(bytes)?;
    let mut cursor = Cursor::new(payload);
    let request = match kind {
        CREATE => Request::Create(cursor.name()?),
        LIST => Request::List,
        ATTACH => Request::Attach(cursor.name()?),
        DETACH => Request::Detach(cursor.name()?),
        PING => Request::Ping(cursor.u64()?),
        _ => return Err(DecodeError::UnknownMessageType(kind)),
    };
    cursor.finish()?;
    Ok(request)
}

/// Encodes a structured response as one complete bounded frame.
///
/// # Errors
///
/// Rejects an oversized session list, error message, or complete frame.
pub fn encode_response(response: &Response) -> Result<Vec<u8>, DecodeError> {
    let mut payload = Vec::new();
    let kind = match response {
        Response::Created(name) => {
            put_name(&mut payload, name);
            CREATED
        }
        Response::Sessions(sessions) => {
            if sessions.len() > MAX_LISTED_SESSIONS {
                return Err(DecodeError::InvalidLength);
            }
            payload.push(u8::try_from(sessions.len()).map_err(|_| DecodeError::InvalidLength)?);
            for session in sessions {
                put_name(&mut payload, &session.name);
                payload.extend_from_slice(&session.attached_clients.to_be_bytes());
            }
            SESSIONS
        }
        Response::Attached(name) => {
            put_name(&mut payload, name);
            ATTACHED
        }
        Response::Detached(name) => {
            put_name(&mut payload, name);
            DETACHED
        }
        Response::Pong(nonce) => {
            payload.extend_from_slice(&nonce.to_be_bytes());
            PONG
        }
        Response::Error { code, message } => {
            if message.len() > MAX_ERROR_MESSAGE_BYTES {
                return Err(DecodeError::InvalidLength);
            }
            payload.push(*code as u8);
            put_bytes(&mut payload, message.as_bytes())?;
            ERROR
        }
    };
    frame(kind, &payload)
}

/// Decodes exactly one complete response frame.
///
/// # Errors
///
/// Rejects malformed, oversized, unsupported, or trailing data.
pub fn decode_response(bytes: &[u8]) -> Result<Response, DecodeError> {
    let (kind, payload) = unframe(bytes)?;
    let mut cursor = Cursor::new(payload);
    let response = match kind {
        CREATED => Response::Created(cursor.name()?),
        SESSIONS => {
            let count = usize::from(cursor.byte()?);
            if count > MAX_LISTED_SESSIONS {
                return Err(DecodeError::InvalidLength);
            }
            let mut sessions = Vec::with_capacity(count);
            for _ in 0..count {
                sessions.push(SessionInfo {
                    name: cursor.name()?,
                    attached_clients: cursor.u16()?,
                });
            }
            Response::Sessions(sessions)
        }
        ATTACHED => Response::Attached(cursor.name()?),
        DETACHED => Response::Detached(cursor.name()?),
        PONG => Response::Pong(cursor.u64()?),
        ERROR => {
            let raw_code = cursor.byte()?;
            let code =
                ErrorCode::from_byte(raw_code).ok_or(DecodeError::UnknownErrorCode(raw_code))?;
            let message_bytes = cursor.bytes(MAX_ERROR_MESSAGE_BYTES)?;
            let message = std::str::from_utf8(message_bytes)
                .map_err(|_| DecodeError::InvalidUtf8)?
                .to_owned();
            Response::Error { code, message }
        }
        _ => return Err(DecodeError::UnknownMessageType(kind)),
    };
    cursor.finish()?;
    Ok(response)
}

fn frame(kind: u8, payload: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let frame_len = HEADER_BYTES
        .checked_add(payload.len())
        .ok_or(DecodeError::FrameTooLarge)?;
    if frame_len > MAX_FRAME_BYTES {
        return Err(DecodeError::FrameTooLarge);
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| DecodeError::FrameTooLarge)?;
    let mut bytes = Vec::with_capacity(frame_len);
    bytes.extend_from_slice(MAGIC);
    bytes.push(PROTOCOL_VERSION);
    bytes.push(kind);
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn unframe(bytes: &[u8]) -> Result<(u8, &[u8]), DecodeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(DecodeError::Truncated);
    }
    if &bytes[..4] != MAGIC {
        return Err(DecodeError::InvalidMagic);
    }
    if bytes[4] != PROTOCOL_VERSION {
        return Err(DecodeError::UnsupportedVersion(bytes[4]));
    }
    let payload_len = u32::from_be_bytes(
        bytes[6..10]
            .try_into()
            .map_err(|_| DecodeError::Truncated)?,
    ) as usize;
    let frame_len = HEADER_BYTES
        .checked_add(payload_len)
        .ok_or(DecodeError::FrameTooLarge)?;
    if frame_len > MAX_FRAME_BYTES {
        return Err(DecodeError::FrameTooLarge);
    }
    match bytes.len().cmp(&frame_len) {
        std::cmp::Ordering::Less => Err(DecodeError::Truncated),
        std::cmp::Ordering::Greater => Err(DecodeError::TrailingBytes),
        std::cmp::Ordering::Equal => Ok((bytes[5], &bytes[HEADER_BYTES..])),
    }
}

fn put_name(output: &mut Vec<u8>, name: &SessionName) {
    // SessionName is capped at 64 bytes, so this conversion cannot fail.
    output
        .push(u8::try_from(name.as_str().len()).expect("validated session name fits in one byte"));
    output.extend_from_slice(name.as_str().as_bytes());
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), DecodeError> {
    let len = u16::try_from(bytes.len()).map_err(|_| DecodeError::InvalidLength)?;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let value = *self.bytes.get(self.offset).ok_or(DecodeError::Truncated)?;
        self.offset += 1;
        Ok(value)
    }

    fn bytes(&mut self, maximum: usize) -> Result<&'a [u8], DecodeError> {
        let len = usize::from(self.u16()?);
        if len > maximum {
            return Err(DecodeError::InvalidLength);
        }
        self.take(len)
    }

    fn name(&mut self) -> Result<SessionName, DecodeError> {
        let len = usize::from(self.byte()?);
        if len == 0 || len > 64 {
            return Err(DecodeError::InvalidLength);
        }
        let value = std::str::from_utf8(self.take(len)?).map_err(|_| DecodeError::InvalidUtf8)?;
        SessionName::parse(value).map_err(DecodeError::InvalidSessionName)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| DecodeError::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| DecodeError::Truncated)?,
        ))
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(DecodeError::InvalidLength)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DecodeError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self) -> Result<(), DecodeError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> SessionName {
        SessionName::parse(value).expect("test name must be valid")
    }

    #[test]
    fn every_request_round_trips() {
        let requests = [
            Request::Create(name("build")),
            Request::List,
            Request::Attach(name("build")),
            Request::Detach(name("build")),
            Request::Ping(u64::MAX),
        ];
        for request in requests {
            let encoded = encode_request(&request).expect("request should encode");
            assert_eq!(decode_request(&encoded), Ok(request));
        }
    }

    #[test]
    fn every_response_round_trips() {
        let responses = [
            Response::Created(name("build")),
            Response::Sessions(vec![
                SessionInfo {
                    name: name("build"),
                    attached_clients: 2,
                },
                SessionInfo {
                    name: name("review"),
                    attached_clients: 0,
                },
            ]),
            Response::Attached(name("build")),
            Response::Detached(name("build")),
            Response::Pong(42),
            Response::Error {
                code: ErrorCode::NotFound,
                message: "session not found".to_owned(),
            },
        ];
        for response in responses {
            let encoded = encode_response(&response).expect("response should encode");
            assert_eq!(decode_response(&encoded), Ok(response));
        }
    }

    #[test]
    fn rejects_oversized_declared_frame_before_payload_read() {
        let mut frame = encode_request(&Request::List).expect("request should encode");
        let oversized_payload =
            u32::try_from(MAX_FRAME_BYTES).expect("frame limit fits in protocol length field");
        frame[6..10].copy_from_slice(&oversized_payload.to_be_bytes());
        assert_eq!(decode_request(&frame), Err(DecodeError::FrameTooLarge));
    }

    #[test]
    fn rejects_truncation_and_trailing_bytes() {
        let frame = encode_request(&Request::Ping(7)).expect("request should encode");
        assert_eq!(
            decode_request(&frame[..frame.len() - 1]),
            Err(DecodeError::Truncated)
        );
        let mut trailing = frame;
        trailing.push(0);
        assert_eq!(decode_request(&trailing), Err(DecodeError::TrailingBytes));
    }

    #[test]
    fn rejects_invalid_session_name_from_peer() {
        let mut frame = encode_request(&Request::Create(name("safe"))).expect("should encode");
        frame[10] = 3;
        frame[11..14].copy_from_slice(b"../");
        frame.truncate(14);
        frame[6..10].copy_from_slice(&4_u32.to_be_bytes());
        assert_eq!(
            decode_request(&frame),
            Err(DecodeError::InvalidSessionName(
                SessionNameError::InvalidCharacter
            ))
        );
    }

    #[test]
    fn rejects_invalid_utf8_and_unknown_types() {
        let invalid_utf8 = frame(CREATE, &[1, 0xff]).expect("test frame should encode");
        assert_eq!(decode_request(&invalid_utf8), Err(DecodeError::InvalidUtf8));
        let unknown = frame(0x40, &[]).expect("test frame should encode");
        assert_eq!(
            decode_request(&unknown),
            Err(DecodeError::UnknownMessageType(0x40))
        );
    }

    #[test]
    fn rejects_oversized_collections_and_messages() {
        let sessions = (0..=MAX_LISTED_SESSIONS)
            .map(|index| SessionInfo {
                name: name(&format!("s{index}")),
                attached_clients: 0,
            })
            .collect();
        assert_eq!(
            encode_response(&Response::Sessions(sessions)),
            Err(DecodeError::InvalidLength)
        );
        assert_eq!(
            encode_response(&Response::Error {
                code: ErrorCode::Internal,
                message: "x".repeat(MAX_ERROR_MESSAGE_BYTES + 1),
            }),
            Err(DecodeError::InvalidLength)
        );
    }

    #[test]
    fn request_and_response_namespaces_are_separate() {
        let response = encode_response(&Response::Pong(1)).expect("response should encode");
        assert_eq!(
            decode_request(&response),
            Err(DecodeError::UnknownMessageType(PONG))
        );
    }
}
