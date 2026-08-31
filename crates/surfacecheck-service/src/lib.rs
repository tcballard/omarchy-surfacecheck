//! The optional user-service boundary for SurfaceCheck.
//!
//! The service owns the long-lived operation state and never evaluates shell
//! text.  IPC is a length-prefixed, bounded byte frame over a private Unix
//! socket.  A higher layer is responsible for parsing the frame as a v1 JSON
//! request.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use surfacecheck_core::MAX_JSON_FRAME_BYTES;

pub const DEFAULT_SOCKET_NAME: &str = "surfacecheck.sock";

#[derive(Debug)]
pub enum ServiceError {
    Io(io::Error),
    InvalidRuntimePath,
    InvalidSocketName,
    FrameTooLarge,
    EmptyFrame,
    FrameTruncated,
    UnauthorizedPeer,
    UnsupportedPlatform,
    AlreadyRunning,
    ActiveOperation,
}

impl PartialEq for ServiceError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for ServiceError {}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io(_) => "runtime service I/O failed",
            Self::InvalidRuntimePath => "runtime path is invalid",
            Self::InvalidSocketName => "socket name is invalid",
            Self::FrameTooLarge => "IPC frame exceeds the bounded limit",
            Self::EmptyFrame => "IPC frame must not be empty",
            Self::FrameTruncated => "IPC frame ended before its declared length",
            Self::UnauthorizedPeer => "IPC peer is not the current user",
            Self::UnsupportedPlatform => "authenticated runtime IPC is unsupported here",
            Self::AlreadyRunning => "runtime service is already running",
            Self::ActiveOperation => "another operation is already active",
        })
    }
}

impl Error for ServiceError {}

impl From<io::Error> for ServiceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub root: PathBuf,
    pub socket: PathBuf,
}

impl RuntimePaths {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ServiceError> {
        let root = root.into();
        if root.as_os_str().is_empty() || root.is_file() || is_symlink(&root) {
            return Err(ServiceError::InvalidRuntimePath);
        }
        let socket = root.join(DEFAULT_SOCKET_NAME);
        if socket.file_name().and_then(|name| name.to_str()) != Some(DEFAULT_SOCKET_NAME) {
            return Err(ServiceError::InvalidSocketName);
        }
        Ok(Self { root, socket })
    }

    pub fn from_environment() -> Result<Self, ServiceError> {
        let root = std::env::var_os("SURFACECHECK_RUNTIME_DIR")
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
            .ok_or(ServiceError::InvalidRuntimePath)?;
        Self::new(PathBuf::from(root).join("surfacecheck"))
    }

    pub fn prepare(&self) -> Result<(), ServiceError> {
        if self.root.exists() {
            if is_symlink(&self.root) || !self.root.is_dir() {
                return Err(ServiceError::InvalidRuntimePath);
            }
        } else {
            fs::create_dir_all(&self.root)?;
        }
        set_private_mode(&self.root, true)?;
        if self.socket.exists() && is_symlink(&self.socket) {
            return Err(ServiceError::InvalidRuntimePath);
        }
        Ok(())
    }

    #[cfg(unix)]
    pub fn bind(&self) -> Result<std::os::unix::net::UnixListener, ServiceError> {
        use std::os::unix::net::{UnixListener, UnixStream};
        self.prepare()?;
        if self.socket.exists() {
            if !is_socket(&self.socket) {
                return Err(ServiceError::InvalidRuntimePath);
            }
            match UnixStream::connect(&self.socket) {
                Ok(_) => return Err(ServiceError::AlreadyRunning),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::NotFound
                            | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    fs::remove_file(&self.socket)?;
                }
                Err(_) => return Err(ServiceError::AlreadyRunning),
            }
        }
        let listener = UnixListener::bind(&self.socket)?;
        set_private_mode(&self.socket, false)?;
        Ok(listener)
    }

    #[cfg(not(unix))]
    pub fn bind(&self) -> Result<(), ServiceError> {
        Err(ServiceError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
pub fn authenticate_peer(
    stream: &std::os::unix::net::UnixStream,
    expected_uid: u32,
) -> Result<(), ServiceError> {
    use std::os::fd::AsRawFd;
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 || credentials.uid != expected_uid {
        return Err(ServiceError::UnauthorizedPeer);
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn authenticate_peer<T>(_stream: &T, _expected_uid: u32) -> Result<(), ServiceError> {
    Err(ServiceError::UnsupportedPlatform)
}

#[cfg(unix)]
pub fn serve_once<F>(
    listener: &std::os::unix::net::UnixListener,
    expected_uid: u32,
    handler: F,
) -> Result<(), ServiceError>
where
    F: FnOnce(Vec<u8>) -> Vec<u8>,
{
    let (mut stream, _) = listener.accept()?;
    authenticate_peer(&stream, expected_uid)?;
    let request = read_frame(&mut stream)?;
    let response = handler(request);
    write_frame(&mut stream, &response)
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, ServiceError> {
    let mut length_bytes = [0u8; 4];
    reader
        .read_exact(&mut length_bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => ServiceError::FrameTruncated,
            _ => ServiceError::Io(error),
        })?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 {
        return Err(ServiceError::EmptyFrame);
    }
    if length > MAX_JSON_FRAME_BYTES {
        return Err(ServiceError::FrameTooLarge);
    }
    let mut frame = vec![0u8; length];
    reader
        .read_exact(&mut frame)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => ServiceError::FrameTruncated,
            _ => ServiceError::Io(error),
        })?;
    Ok(frame)
}

pub fn write_frame<W: Write>(writer: &mut W, frame: &[u8]) -> Result<(), ServiceError> {
    if frame.is_empty() {
        return Err(ServiceError::EmptyFrame);
    }
    if frame.len() > MAX_JSON_FRAME_BYTES || frame.len() > u32::MAX as usize {
        return Err(ServiceError::FrameTooLarge);
    }
    writer.write_all(&(frame.len() as u32).to_be_bytes())?;
    writer.write_all(frame)?;
    writer.flush()?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct OperationLease {
    pub operation_id: String,
    pub generation: u64,
    cancellation: Arc<AtomicBool>,
}

impl PartialEq for OperationLease {
    fn eq(&self, other: &Self) -> bool {
        self.operation_id == other.operation_id && self.generation == other.generation
    }
}

impl Eq for OperationLease {}

impl OperationLease {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }

    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusyOperation {
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Debug, Default)]
struct FlightState {
    active: Option<OperationLease>,
    next_generation: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SingleFlight {
    state: Arc<Mutex<FlightState>>,
}

impl SingleFlight {
    pub fn begin(&self, operation_id: &str) -> Result<OperationLease, ServiceError> {
        if !valid_operation_id(operation_id) {
            return Err(ServiceError::InvalidRuntimePath);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ServiceError::Io(io::Error::other("flight lock poisoned")))?;
        if state.active.is_some() {
            return Err(ServiceError::ActiveOperation);
        }
        state.next_generation = state.next_generation.wrapping_add(1).max(1);
        let lease = OperationLease {
            operation_id: operation_id.to_owned(),
            generation: state.next_generation,
            cancellation: Arc::new(AtomicBool::new(false)),
        };
        state.active = Some(lease.clone());
        Ok(lease)
    }

    pub fn finish(&self, lease: &OperationLease) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state
            .active
            .as_ref()
            .is_some_and(|active| same_lease(active, lease))
        {
            state.active = None;
            true
        } else {
            false
        }
    }

    pub fn cancel(&self, operation_id: &str, generation: u64) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if let Some(active) = &state.active {
            if active.operation_id == operation_id && active.generation == generation {
                active.cancellation.store(true, Ordering::Release);
                return true;
            }
        }
        false
    }

    pub fn active(&self) -> Option<BusyOperation> {
        self.state.lock().ok().and_then(|state| {
            state.active.as_ref().map(|active| BusyOperation {
                operation_id: active.operation_id.clone(),
                generation: active.generation,
            })
        })
    }
}

fn same_lease(left: &OperationLease, right: &OperationLease) -> bool {
    left.operation_id == right.operation_id && left.generation == right.generation
}

fn valid_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn set_private_mode(path: &Path, directory: bool) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if directory { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    let _ = directory;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn bounded_frames_round_trip_and_reject_hostile_lengths() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"hello").expect("write");
        assert_eq!(read_frame(&mut Cursor::new(bytes)).expect("read"), b"hello");
        assert_eq!(
            read_frame(&mut Cursor::new(0u32.to_be_bytes())),
            Err(ServiceError::EmptyFrame)
        );
        assert_eq!(
            read_frame(&mut Cursor::new(
                (MAX_JSON_FRAME_BYTES as u32 + 1).to_be_bytes()
            )),
            Err(ServiceError::FrameTooLarge)
        );
    }

    #[test]
    fn single_flight_is_atomic_and_stale_cancellation_is_ignored() {
        let flight = SingleFlight::default();
        let lease = flight.begin("capture-1").expect("begin");
        assert_eq!(
            flight.begin("capture-2"),
            Err(ServiceError::ActiveOperation)
        );
        assert!(!flight.cancel("capture-1", lease.generation + 1));
        assert!(!lease.is_cancelled());
        assert!(flight.cancel("capture-1", lease.generation));
        assert!(lease.is_cancelled());
        assert!(flight.finish(&lease));
        assert!(!flight.finish(&lease));
    }

    #[test]
    fn concurrent_begin_has_one_winner() {
        let flight = Arc::new(SingleFlight::default());
        let mut handles = Vec::new();
        for index in 0..8 {
            let flight = Arc::clone(&flight);
            handles.push(thread::spawn(move || flight.begin(&format!("op-{index}"))));
        }
        let winners = handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().and_then(Result::ok))
            .count();
        assert_eq!(winners, 1);
    }

    #[cfg(unix)]
    #[test]
    fn private_runtime_rejects_a_non_socket_path() {
        let root =
            std::env::temp_dir().join(format!("surfacecheck-service-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = RuntimePaths::new(&root).expect("paths");
        paths.prepare().expect("prepare");
        fs::write(&paths.socket, b"not a socket").expect("sentinel");
        assert!(matches!(
            paths.bind(),
            Err(ServiceError::InvalidRuntimePath)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
