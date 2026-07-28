//! Authenticated, local-only transport for the future session daemon.
//!
//! This layer owns only endpoint creation and peer authentication. It does not
//! dispatch protocol messages or manage sessions.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The fixed name of the per-user control socket.
pub const CONTROL_SOCKET_NAME: &str = "control.sock";
/// Read and write operations are bounded so an idle local peer cannot retain a
/// daemon worker indefinitely.
pub const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Failures establishing the local session transport.
#[derive(Debug)]
pub enum TransportError {
    /// The current platform has no implemented secure local transport.
    UnsupportedPlatform,
    /// A runtime directory or endpoint failed a security invariant.
    InsecurePath(String),
    /// A live daemon already owns the endpoint.
    AlreadyRunning,
    /// The connected process does not have the server's effective user ID.
    PeerIdentityMismatch,
    /// An operating-system operation failed.
    Io(io::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter
                .write_str("secure local session transport is unsupported on this platform"),
            Self::InsecurePath(reason) => write!(formatter, "insecure runtime path: {reason}"),
            Self::AlreadyRunning => {
                formatter.write_str("a NovaMux session daemon is already running")
            }
            Self::PeerIdentityMismatch => {
                formatter.write_str("local peer effective user ID does not match")
            }
            Self::Io(error) => write!(formatter, "local transport I/O failed: {error}"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Returns the platform-specific per-user runtime directory.
///
/// `XDG_RUNTIME_DIR` is used on Unix when present. Otherwise a UID-qualified
/// directory under the operating system's temporary directory is used.
///
/// # Errors
///
/// Returns [`TransportError::UnsupportedPlatform`] when no secure transport
/// exists, or [`TransportError::InsecurePath`] if the selected base is not
/// absolute.
pub fn default_runtime_dir() -> Result<PathBuf, TransportError> {
    platform::default_runtime_dir()
}

/// Returns whether the secure transport has an implementation on this target.
#[must_use]
pub const fn is_supported() -> bool {
    cfg!(unix)
}

#[cfg(unix)]
mod platform {
    use super::{CONTROL_SOCKET_NAME, IO_TIMEOUT, Path, PathBuf, TransportError, io};
    #[cfg(any(target_os = "linux", target_os = "android"))]
    use nix::sys::socket::{getsockopt, sockopt};
    use nix::unistd::{Uid, geteuid};
    use std::env;
    use std::fs::{self, DirBuilder, Metadata, Permissions};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    // macOS has the smallest supported sockaddr_un.sun_path (104 bytes). Leave
    // room for its terminating NUL.
    const MAX_SOCKET_PATH_BYTES: usize = 103;

    /// A bound, private per-user server endpoint.
    pub struct LocalEndpoint {
        listener: UnixListener,
        path: PathBuf,
        socket_identity: (u64, u64),
    }

    impl LocalEndpoint {
        /// Creates and binds the endpoint, recovering only a verified stale
        /// socket owned by this effective user.
        ///
        /// # Errors
        ///
        /// Returns an error if the runtime directory fails validation, the
        /// socket path is too long, a live server exists, or binding fails.
        pub fn bind(runtime_dir: &Path) -> Result<Self, TransportError> {
            ensure_runtime_dir(runtime_dir)?;
            let path = runtime_dir.join(CONTROL_SOCKET_NAME);
            validate_socket_path(&path)?;

            match bind_private(&path) {
                Ok(listener) => Self::from_bound(listener, path),
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    recover_stale_socket(&path)?;
                    let listener = bind_private(&path)?;
                    Self::from_bound(listener, path)
                }
                Err(error) => Err(error.into()),
            }
        }

        fn from_bound(listener: UnixListener, path: PathBuf) -> Result<Self, TransportError> {
            let metadata = fs::symlink_metadata(&path)?;
            Ok(Self {
                listener,
                path,
                socket_identity: (metadata.dev(), metadata.ino()),
            })
        }

        /// Accepts one connection and verifies its effective user ID before
        /// returning a stream to protocol code.
        ///
        /// # Errors
        ///
        /// Returns an error when accepting, applying timeouts, or retrieving
        /// peer credentials fails, or if the peer has a different user ID.
        pub fn accept_authenticated(&self) -> Result<UnixStream, TransportError> {
            let (stream, _) = self.listener.accept()?;
            verify_same_user(&stream)?;
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            Ok(stream)
        }

        /// Filesystem path of the bound control socket.
        #[must_use]
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for LocalEndpoint {
        fn drop(&mut self) {
            // Never remove an object substituted after binding.
            if let Ok(metadata) = fs::symlink_metadata(&self.path)
                && metadata.file_type().is_socket()
                && metadata.uid() == geteuid().as_raw()
                && (metadata.dev(), metadata.ino()) == self.socket_identity
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    pub fn default_runtime_dir() -> Result<PathBuf, TransportError> {
        let base = match env::var_os("XDG_RUNTIME_DIR") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => env::temp_dir(),
        };
        if !base.is_absolute() {
            return Err(TransportError::InsecurePath(
                "runtime base must be absolute".into(),
            ));
        }
        Ok(base.join(format!("novamux-{}", geteuid().as_raw())))
    }

    fn ensure_runtime_dir(path: &Path) -> Result<(), TransportError> {
        if !path.is_absolute() {
            return Err(TransportError::InsecurePath(
                "runtime directory must be absolute".into(),
            ));
        }

        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_runtime_metadata(&metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = DirBuilder::new();
                builder.mode(0o700);
                match builder.create(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(path)?;
                validate_runtime_metadata(&metadata)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn validate_runtime_metadata(metadata: &Metadata) -> Result<(), TransportError> {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(TransportError::InsecurePath(
                "runtime path is not a real directory".into(),
            ));
        }
        if metadata.uid() != geteuid().as_raw() {
            return Err(TransportError::InsecurePath(
                "runtime directory is owned by another user".into(),
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(TransportError::InsecurePath(
                "runtime directory permits group or other access".into(),
            ));
        }
        Ok(())
    }

    fn validate_socket_path(path: &Path) -> Result<(), TransportError> {
        if path.as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES {
            return Err(TransportError::InsecurePath(format!(
                "control socket path exceeds {MAX_SOCKET_PATH_BYTES} bytes"
            )));
        }
        Ok(())
    }

    fn bind_private(path: &Path) -> io::Result<UnixListener> {
        let listener = UnixListener::bind(path)?;
        if let Err(error) = fs::set_permissions(path, Permissions::from_mode(0o600)) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(listener)
    }

    fn recover_stale_socket(path: &Path) -> Result<(), TransportError> {
        if UnixStream::connect(path).is_ok() {
            return Err(TransportError::AlreadyRunning);
        }

        let metadata = fs::symlink_metadata(path)?;
        validate_stale_socket(&metadata)?;
        fs::remove_file(path)?;
        Ok(())
    }

    fn validate_stale_socket(metadata: &Metadata) -> Result<(), TransportError> {
        if !metadata.file_type().is_socket() {
            return Err(TransportError::InsecurePath(
                "existing control endpoint is not a socket".into(),
            ));
        }
        if metadata.uid() != geteuid().as_raw() {
            return Err(TransportError::InsecurePath(
                "existing control socket is owned by another user".into(),
            ));
        }
        Ok(())
    }

    fn verify_same_user(stream: &UnixStream) -> Result<(), TransportError> {
        let peer_uid = peer_effective_uid(stream)?;
        if peer_uid != geteuid() {
            return Err(TransportError::PeerIdentityMismatch);
        }
        Ok(())
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    fn peer_effective_uid(stream: &UnixStream) -> Result<Uid, TransportError> {
        let (uid, _) = nix::unistd::getpeereid(stream)
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
        Ok(uid)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn peer_effective_uid(stream: &UnixStream) -> Result<Uid, TransportError> {
        let credentials = getsockopt(stream, sockopt::PeerCredentials)
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
        Ok(Uid::from_raw(credentials.uid()))
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    )))]
    fn peer_effective_uid(_stream: &UnixStream) -> Result<Uid, TransportError> {
        Err(TransportError::UnsupportedPlatform)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::mpsc;
        use std::thread;

        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        struct TestDir(PathBuf);

        impl TestDir {
            fn new() -> Self {
                let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
                let path = env::temp_dir().join(format!(
                    "novamux-transport-test-{}-{suffix}",
                    std::process::id()
                ));
                let mut builder = DirBuilder::new();
                builder.mode(0o700).create(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for TestDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn creates_private_runtime_directory_and_socket() {
            let parent = TestDir::new();
            let runtime = parent.0.join("runtime");
            let endpoint = LocalEndpoint::bind(&runtime).unwrap();

            let directory = fs::symlink_metadata(&runtime).unwrap();
            assert_eq!(directory.mode() & 0o777, 0o700);
            let socket = fs::symlink_metadata(endpoint.path()).unwrap();
            assert!(socket.file_type().is_socket());
            assert_eq!(socket.mode() & 0o777, 0o600);
        }

        #[test]
        fn rejects_symlink_runtime_directory() {
            let parent = TestDir::new();
            let real = parent.0.join("real");
            fs::create_dir(&real).unwrap();
            fs::set_permissions(&real, Permissions::from_mode(0o700)).unwrap();
            let link = parent.0.join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();

            assert!(matches!(
                LocalEndpoint::bind(&link),
                Err(TransportError::InsecurePath(_))
            ));
        }

        #[test]
        fn rejects_group_accessible_runtime_directory() {
            let parent = TestDir::new();
            let runtime = parent.0.join("runtime");
            fs::create_dir(&runtime).unwrap();
            fs::set_permissions(&runtime, Permissions::from_mode(0o750)).unwrap();

            assert!(matches!(
                LocalEndpoint::bind(&runtime),
                Err(TransportError::InsecurePath(_))
            ));
        }

        #[test]
        fn refuses_to_replace_non_socket_endpoint() {
            let parent = TestDir::new();
            let endpoint_path = parent.0.join(CONTROL_SOCKET_NAME);
            fs::write(&endpoint_path, b"do not replace").unwrap();

            assert!(matches!(
                LocalEndpoint::bind(&parent.0),
                Err(TransportError::InsecurePath(_))
            ));
            assert_eq!(fs::read(endpoint_path).unwrap(), b"do not replace");
        }

        #[test]
        fn recovers_owned_stale_socket() {
            let parent = TestDir::new();
            let endpoint_path = parent.0.join(CONTROL_SOCKET_NAME);
            drop(UnixListener::bind(&endpoint_path).unwrap());

            let endpoint = LocalEndpoint::bind(&parent.0).unwrap();
            assert_eq!(endpoint.path(), endpoint_path);
        }

        #[test]
        fn refuses_second_live_server() {
            let parent = TestDir::new();
            let _first = LocalEndpoint::bind(&parent.0).unwrap();

            assert!(matches!(
                LocalEndpoint::bind(&parent.0),
                Err(TransportError::AlreadyRunning)
            ));
        }

        #[test]
        fn drop_does_not_remove_a_substituted_owned_socket() {
            let parent = TestDir::new();
            let endpoint = LocalEndpoint::bind(&parent.0).unwrap();
            let path = endpoint.path().to_owned();
            fs::remove_file(&path).unwrap();
            let replacement = UnixListener::bind(&path).unwrap();

            drop(endpoint);

            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
            drop(replacement);
        }

        #[test]
        fn authenticates_same_user_before_returning_stream() {
            let parent = TestDir::new();
            let endpoint = LocalEndpoint::bind(&parent.0).unwrap();
            let path = endpoint.path().to_owned();
            let (release_sender, release_receiver) = mpsc::channel();
            let client = thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                stream.write_all(b"authenticated").unwrap();
                release_receiver.recv().unwrap();
            });

            let stream = endpoint.accept_authenticated().unwrap();
            assert_eq!(stream.read_timeout().unwrap(), Some(IO_TIMEOUT));
            release_sender.send(()).unwrap();
            client.join().unwrap();
        }

        #[test]
        fn rejects_overlong_socket_path_before_binding() {
            let parent = TestDir::new();
            let runtime = parent.0.join("x".repeat(MAX_SOCKET_PATH_BYTES));

            assert!(matches!(
                LocalEndpoint::bind(&runtime),
                Err(TransportError::InsecurePath(_))
            ));
        }
    }
}

#[cfg(unix)]
pub use platform::LocalEndpoint;

#[cfg(not(unix))]
/// Explicit unsupported transport for platforms without an implementation.
pub struct LocalEndpoint;

#[cfg(not(unix))]
impl LocalEndpoint {
    /// Always returns [`TransportError::UnsupportedPlatform`].
    ///
    /// # Errors
    ///
    /// Always returns [`TransportError::UnsupportedPlatform`].
    pub fn bind(_runtime_dir: &Path) -> Result<Self, TransportError> {
        Err(TransportError::UnsupportedPlatform)
    }
}

#[cfg(not(unix))]
mod platform {
    use super::{PathBuf, TransportError};

    pub fn default_runtime_dir() -> Result<PathBuf, TransportError> {
        Err(TransportError::UnsupportedPlatform)
    }
}
