//! Named pipe doorbell for cross-process wakeup (Windows).
//!
//! Uses Windows named pipes for bidirectional signaling between processes
//! sharing memory. The host creates a named pipe server, and guests connect
//! as clients using the pipe name.
//!
//! **Important:** Synchronous operations (`signal_now`, `try_drain`) use raw
//! Win32 `WriteFile`/`PeekNamedPipe`/`ReadFile` instead of tokio's
//! `try_write`/`try_read`, because tokio's methods require prior async
//! readiness polling which is unavailable in sync contexts.

use std::io::{self, ErrorKind};
use std::os::windows::io::AsRawHandle;
use std::string::String;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::Interest;
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

use std::format;
use std::string::ToString;

/// Result of a doorbell signal attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalResult {
    /// Signal was sent successfully.
    Sent,
    /// Buffer was full but peer is alive (signal coalesced with pending ones).
    BufferFull,
    /// Peer has disconnected (pipe broken).
    PeerDead,
}

/// Opaque handle for passing doorbell endpoints between processes.
///
/// On Windows, this wraps a named pipe path.
/// On Unix, this wraps a raw file descriptor (see doorbell.rs).
///
/// Use [`Doorbell::create_pair`] to create a pair, then pass this handle
/// to the child process and call [`Doorbell::from_handle`] to reconstruct.
#[derive(Debug)]
pub struct DoorbellHandle(String);

impl DoorbellHandle {
    /// Get the pipe name (for passing to child processes).
    pub fn as_pipe_name(&self) -> &str {
        &self.0
    }

    /// Create from a pipe name (in child process after spawn).
    pub fn from_pipe_name(name: String) -> Self {
        Self(name)
    }

    /// Format as a command-line argument value.
    pub fn to_arg(&self) -> String {
        self.0.clone()
    }

    /// Parse from a command-line argument value.
    /// # Safety
    /// The caller must ensure the pipe name refers to a valid doorbell pipe.
    pub unsafe fn from_arg(s: &str) -> Result<Self, std::convert::Infallible> {
        Ok(Self(s.to_string()))
    }

    /// The CLI argument name for this platform.
    pub const ARG_NAME: &'static str = "--doorbell-pipe";
}

/// A doorbell for cross-process wakeup.
///
/// On Windows, uses named pipes for bidirectional signaling.
/// The host creates a server pipe, and guests connect as clients.
pub struct Doorbell {
    /// The pipe (either server or client side)
    pipe: DoorbellPipe,
    /// The pipe name (for diagnostics and reconnection)
    pipe_name: String,
    /// Whether we've already logged that the peer is dead (to avoid spam).
    peer_dead_logged: AtomicBool,
    /// Whether the server pipe has called ConnectNamedPipe.
    /// Always true for client pipes.
    server_connected: AtomicBool,
}

enum DoorbellPipe {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

impl Doorbell {
    /// Get the raw OS handle for the pipe.
    fn raw_handle(&self) -> HANDLE {
        match &self.pipe {
            DoorbellPipe::Server(s) => s.as_raw_handle() as HANDLE,
            DoorbellPipe::Client(c) => c.as_raw_handle() as HANDLE,
        }
    }

    /// Write a signal byte using raw Win32 WriteFile.
    ///
    /// This bypasses tokio's readiness tracking, which is necessary because
    /// tokio's `try_write()` returns WouldBlock unless `ready(WRITABLE)` was
    /// previously awaited — impossible from a sync context.
    fn raw_write_signal(&self) -> SignalResult {
        let handle = self.raw_handle();
        let buf = [1u8];
        let mut written: u32 = 0;
        let ok = unsafe { WriteFile(handle, buf.as_ptr(), 1, &mut written, std::ptr::null_mut()) };
        if ok != 0 && written == 1 {
            return SignalResult::Sent;
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(e) if e == ERROR_BROKEN_PIPE as i32 => SignalResult::PeerDead,
            Some(e) if e == ERROR_NO_DATA as i32 => SignalResult::PeerDead,
            Some(232) => SignalResult::PeerDead, // ERROR_NO_DATA (pipe closing)
            Some(233) => SignalResult::PeerDead, // ERROR_PIPE_NOT_CONNECTED
            _ => {
                if !self.peer_dead_logged.swap(true, Ordering::Relaxed) {
                    tracing::debug!(pipe = %self.pipe_name, error = %err, "doorbell raw write signal failed");
                }
                SignalResult::PeerDead
            }
        }
    }

    /// Check if signal bytes are pending using raw Win32 PeekNamedPipe.
    ///
    /// Returns true if at least one byte is available to read.
    /// Does NOT consume the bytes — that's left to tokio's `try_read` in
    /// the `wait()` readiness loop where it works reliably.
    fn raw_has_pending(&self) -> bool {
        let handle = self.raw_handle();
        let mut total_available: u32 = 0;

        let ok = unsafe {
            PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut total_available,
                std::ptr::null_mut(),
            )
        };
        ok != 0 && total_available > 0
    }

    /// Signal the other side without awaiting readiness.
    ///
    /// Performs a single non-blocking write attempt and returns immediately.
    /// Uses raw Win32 `WriteFile` to bypass tokio's readiness tracking.
    pub fn signal_now(&self) -> SignalResult {
        self.raw_write_signal()
    }

    /// Create a named pipe server and return (host_doorbell, guest_handle).
    ///
    /// The guest_handle should be passed to the plugin (e.g., via command line).
    /// The host keeps the Doorbell.
    pub fn create_pair() -> io::Result<(Self, DoorbellHandle)> {
        // Generate unique pipe name
        let uuid = generate_uuid();
        let pipe_name = format!(r"\\.\pipe\roam-shm-{}", uuid);

        // Create server pipe
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)?;

        Ok((
            Self {
                pipe: DoorbellPipe::Server(server),
                pipe_name: pipe_name.clone(),
                peer_dead_logged: AtomicBool::new(false),
                server_connected: AtomicBool::new(false),
            },
            DoorbellHandle(pipe_name),
        ))
    }

    /// Create a Doorbell from an opaque handle (guest/plugin side).
    ///
    /// This is the cross-platform way to reconstruct a Doorbell in a spawned process.
    pub fn from_handle(handle: DoorbellHandle) -> io::Result<Self> {
        Self::connect(&handle.0)
    }

    /// Connect to an existing named pipe as a client.
    ///
    /// Prefer [`from_handle`] for cross-platform code.
    ///
    /// This is for spawned guest processes that receive the pipe name.
    pub fn connect(pipe_name: &str) -> io::Result<Self> {
        let client = ClientOptions::new().open(pipe_name)?;

        Ok(Self {
            pipe: DoorbellPipe::Client(client),
            pipe_name: pipe_name.to_string(),
            peer_dead_logged: AtomicBool::new(false),
            server_connected: AtomicBool::new(true), // client is always connected
        })
    }

    /// Signal the other side.
    ///
    /// Sends a 1-byte message. If the pipe buffer is full, the signal is
    /// coalesced with pending ones (returns `BufferFull`).
    ///
    /// Returns `SignalResult::PeerDead` if the peer has disconnected.
    pub async fn signal(&self) -> SignalResult {
        let signal_result = self.signal_now();
        tracing::trace!(pipe = %self.pipe_name, ?signal_result, "doorbell signal");
        signal_result
    }

    /// Check if the peer appears to be dead (signal has failed).
    pub fn is_peer_dead(&self) -> bool {
        self.peer_dead_logged.load(Ordering::Relaxed)
    }

    /// Wait for a signal from the other side.
    pub async fn wait(&self) -> io::Result<()> {
        // Lazy connect: on Windows, the server pipe must call ConnectNamedPipe
        // before I/O works.  If the client already connected (in-process test),
        // this returns immediately with ERROR_PIPE_CONNECTED.
        if !self.server_connected.load(Ordering::Acquire)
            && let DoorbellPipe::Server(server) = &self.pipe {
                server.connect().await?;
                self.server_connected.store(true, Ordering::Release);
            }

        // Fast path: check if signal bytes are already pending via raw
        // PeekNamedPipe (bypasses tokio readiness, always non-blocking).
        if self.raw_has_pending() {
            // Data IS available.  We need to actually consume it so the pipe
            // buffer doesn't fill up.  Use tokio's try_read — it may return
            // WouldBlock if tokio's readiness state is stale, in which case
            // we fall through to the readiness loop below which will handle it.
            if let Ok(true) = self.try_drain_tokio(false) {
                return Ok(());
            }
            // Tokio readiness not set yet but data is there — the readiness
            // loop below will pick it up promptly.
        }

        loop {
            // Wait for readability using tokio's async reactor.
            let ready = match &self.pipe {
                DoorbellPipe::Server(server) => server.ready(Interest::READABLE).await?,
                DoorbellPipe::Client(client) => client.ready(Interest::READABLE).await?,
            };

            if ready.is_readable() {
                // After tokio signals readability, use tokio's try_read (readiness
                // is guaranteed after ready() returns).
                match self.try_drain_tokio(true) {
                    Ok(true) => return Ok(()),
                    Ok(false) => continue, // spurious wakeup
                    Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// Drain using tokio's try_read — only valid after `ready()` has been awaited.
    fn try_drain_tokio(&self, would_block_is_error: bool) -> io::Result<bool> {
        let mut buf = [0u8; 64];
        let mut drained = false;

        loop {
            let result = match &self.pipe {
                DoorbellPipe::Server(server) => server.try_read(&mut buf),
                DoorbellPipe::Client(client) => client.try_read(&mut buf),
            };

            match result {
                Ok(0) => return Ok(drained), // EOF
                Ok(_) => {
                    drained = true;
                    continue;
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    if drained {
                        return Ok(true);
                    }
                    return if would_block_is_error {
                        Err(io::Error::from(ErrorKind::WouldBlock))
                    } else {
                        Ok(false)
                    };
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Drain any pending signals without blocking.
    ///
    /// Note: On Windows, this only checks if data is pending (via
    /// PeekNamedPipe). Actual consumption happens in the `wait()` loop
    /// via tokio's try_read. The practical effect is the same — the bytes
    /// will be consumed on the next `wait()` call.
    pub fn drain(&self) {
        // Best-effort: try tokio's try_read, which works if readiness is set.
        let _ = self.try_drain_tokio(false);
    }

    /// Accept an incoming connection (required on Windows).
    ///
    /// On Windows, named pipe servers must call this to accept the client connection
    /// before data can flow. On Unix, this is a no-op since socketpairs are already connected.
    ///
    /// This should be called after the guest process has been spawned and before
    /// starting to wait on the doorbell.
    pub async fn accept(&self) -> io::Result<()> {
        match &self.pipe {
            DoorbellPipe::Server(server) => {
                server.connect().await?;
                self.server_connected.store(true, Ordering::Release);
                Ok(())
            }
            DoorbellPipe::Client(_) => {
                // Clients don't need to accept
                Ok(())
            }
        }
    }

    /// Get the pipe name (for diagnostics).
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }
}

/// Generate a simple UUID-like string for pipe names.
fn generate_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let nanos = duration.as_nanos();
    let pid = std::process::id();

    // Combine timestamp and pid for uniqueness
    format!("{:x}-{:x}", nanos, pid)
}

/// Close a handle (Windows equivalent of close_peer_fd).
///
/// # Safety
///
/// handle must be a valid handle that the caller owns.
pub unsafe fn close_handle(handle: HANDLE) {
    if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
        unsafe {
            CloseHandle(handle);
        }
    }
}

/// Validate that a handle is valid.
///
/// On Windows, we check if the handle is not INVALID_HANDLE_VALUE or null.
pub fn validate_handle(handle: HANDLE) -> io::Result<()> {
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid handle",
        ))
    } else {
        Ok(())
    }
}

/// Make a handle inheritable by child processes.
///
/// This is the Windows equivalent of clearing FD_CLOEXEC.
///
/// r[impl shm.spawn.fd-inheritance]
///
/// # Safety
///
/// Handle must be valid
pub unsafe fn set_handle_inheritable(handle: HANDLE) -> io::Result<()> {
    use windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT;
    use windows_sys::Win32::Foundation::SetHandleInformation;

    let result = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_connect() {
        let (host_doorbell, guest_handle) = Doorbell::create_pair().unwrap();

        // Connect in a separate task since server needs to accept
        let connect_handle = tokio::spawn(async move {
            // Small delay to ensure server is ready
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Doorbell::from_handle(guest_handle)
        });

        // Server needs to accept the client connection
        host_doorbell.accept().await.unwrap();

        let guest_doorbell = connect_handle.await.unwrap().unwrap();

        // Test signaling
        assert_eq!(host_doorbell.signal().await, SignalResult::Sent);

        // Give some time for the signal to propagate
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        guest_doorbell.drain();
    }

    #[tokio::test]
    async fn test_bidirectional_signal_and_wait() {
        let (host_doorbell, guest_handle) = Doorbell::create_pair().unwrap();

        let connect_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Doorbell::from_handle(guest_handle)
        });

        host_doorbell.accept().await.unwrap();
        let guest_doorbell = connect_handle.await.unwrap().unwrap();

        // Host signals, guest waits
        assert_eq!(host_doorbell.signal_now(), SignalResult::Sent);
        guest_doorbell.wait().await.unwrap();

        // Guest signals, host waits
        assert_eq!(guest_doorbell.signal_now(), SignalResult::Sent);
        host_doorbell.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_cross_task_signal_and_wait() {
        let (host_doorbell, guest_handle) = Doorbell::create_pair().unwrap();
        let connect_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Doorbell::from_handle(guest_handle)
        });
        host_doorbell.accept().await.unwrap();
        let guest_doorbell = std::sync::Arc::new(connect_handle.await.unwrap().unwrap());
        let host_doorbell = std::sync::Arc::new(host_doorbell);

        // Spawn a task that waits, then signals from the other side
        let hd = host_doorbell.clone();
        let gd = guest_doorbell.clone();
        let waiter = tokio::spawn(async move {
            hd.wait().await.unwrap();
        });
        // Small delay so waiter enters wait() first
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(gd.signal_now(), SignalResult::Sent);
        waiter.await.unwrap();
    }
}
