//! Control socket for delivering mmap file descriptors between processes.
//!
//! Uses a Unix domain socketpair (SOCK_SEQPACKET) with SCM_RIGHTS to transfer
//! file descriptors alongside 16-byte metadata messages.
//!
//! r[impl shm.mmap.attach]
//! r[impl shm.mmap.attach.unix]

use std::io::{self, ErrorKind};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use super::doorbell::set_nonblocking;

/// 16-byte metadata sent alongside each file descriptor.
///
/// r[impl shm.mmap.attach.message]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MmapAttachMessage {
    pub map_id: u32,
    pub map_generation: u32,
    pub mapping_length: u64,
}

impl MmapAttachMessage {
    pub fn to_le_bytes(self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&self.map_id.to_le_bytes());
        buf[4..8].copy_from_slice(&self.map_generation.to_le_bytes());
        buf[8..16].copy_from_slice(&self.mapping_length.to_le_bytes());
        buf
    }

    pub fn from_le_bytes(buf: [u8; 16]) -> Self {
        Self {
            map_id: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            map_generation: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            mapping_length: u64::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]),
        }
    }
}

/// Opaque handle for passing mmap control endpoints between processes.
#[derive(Debug)]
pub struct MmapControlHandle(OwnedFd);

impl MmapControlHandle {
    pub fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    /// Consume this handle and return the owned raw fd.
    pub fn into_raw_fd(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        self.0.into_raw_fd()
    }

    /// # Safety
    /// The caller must ensure the FD is valid and not owned by anything else.
    pub unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    pub fn to_arg(&self) -> String {
        self.0.as_raw_fd().to_string()
    }

    /// # Safety
    /// The FD must be valid and not owned by anything else.
    pub unsafe fn from_arg(s: &str) -> Result<Self, std::num::ParseIntError> {
        let fd: RawFd = s.parse()?;
        Ok(unsafe { Self::from_raw_fd(fd) })
    }

    pub const ARG_NAME: &'static str = "--mmap-control-fd";
}

/// Sender half of the mmap control socket.
pub struct MmapControlSender {
    fd: OwnedFd,
}

/// Receiver half of the mmap control socket.
pub struct MmapControlReceiver {
    async_fd: AsyncFd<OwnedFd>,
}

fn create_dgram_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];

    // SOCK_DGRAM preserves message boundaries on AF_UNIX (like SEQPACKET)
    // and works on macOS where SEQPACKET is not supported.
    #[cfg(target_os = "linux")]
    let sock_type = libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let sock_type = libc::SOCK_DGRAM;

    let ret = unsafe { libc::socketpair(libc::AF_UNIX, sock_type, 0, fds.as_mut_ptr()) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    let fd0 = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let fd1 = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    #[cfg(not(target_os = "linux"))]
    {
        set_nonblocking(fd0.as_raw_fd())?;
        set_nonblocking(fd1.as_raw_fd())?;
    }

    Ok((fd0, fd1))
}

/// Send one fd + 16-byte metadata using SCM_RIGHTS.
fn send_fd_with_metadata(sock_fd: RawFd, fd: RawFd, msg: &MmapAttachMessage) -> io::Result<()> {
    let payload = msg.to_le_bytes();
    let mut iov = libc::iovec {
        iov_base: payload.as_ptr() as *mut libc::c_void,
        iov_len: payload.len(),
    };

    let fds = [fd];
    let data_len = std::mem::size_of_val(&fds);
    let cmsg_space = unsafe { libc::CMSG_SPACE(data_len as u32) as usize };
    let mut control = vec![0u8; cmsg_space];

    let mut msghdr: libc::msghdr = unsafe { std::mem::zeroed() };
    msghdr.msg_iov = &mut iov;
    msghdr.msg_iovlen = 1;
    msghdr.msg_control = control.as_mut_ptr().cast();
    msghdr.msg_controllen = control.len() as _;

    let cmsg = unsafe { libc::CMSG_FIRSTHDR(&msghdr) };
    if cmsg.is_null() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "failed to build cmsg header",
        ));
    }

    unsafe {
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(data_len as u32) as _;
        let data_ptr = libc::CMSG_DATA(cmsg).cast::<RawFd>();
        std::ptr::copy_nonoverlapping(fds.as_ptr(), data_ptr, 1);
    }

    let n = unsafe { libc::sendmsg(sock_fd, &msghdr, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Err(io::Error::new(
            ErrorKind::WriteZero,
            "sendmsg wrote 0 bytes",
        ));
    }
    Ok(())
}

/// Receive one fd + 16-byte metadata using SCM_RIGHTS.
fn recv_fd_with_metadata(sock_fd: RawFd) -> io::Result<(OwnedFd, MmapAttachMessage)> {
    let mut payload = [0u8; 16];
    let mut iov = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };

    let data_len = std::mem::size_of::<RawFd>();
    let cmsg_space = unsafe { libc::CMSG_SPACE(data_len as u32) as usize };
    let mut control = vec![0u8; cmsg_space];

    let mut msghdr: libc::msghdr = unsafe { std::mem::zeroed() };
    msghdr.msg_iov = &mut iov;
    msghdr.msg_iovlen = 1;
    msghdr.msg_control = control.as_mut_ptr().cast();
    msghdr.msg_controllen = control.len() as _;

    let n = unsafe { libc::recvmsg(sock_fd, &mut msghdr, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "peer closed"));
    }
    if (n as usize) < 16 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "short read on mmap control socket",
        ));
    }
    if (msghdr.msg_flags & libc::MSG_CTRUNC) != 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "control message truncated",
        ));
    }

    let mut received_fd: Option<RawFd> = None;
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msghdr);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let cmsg_len = (*cmsg).cmsg_len as usize;
                let base_len = libc::CMSG_LEN(0) as usize;
                if cmsg_len >= base_len + std::mem::size_of::<RawFd>() {
                    let data_ptr = libc::CMSG_DATA(cmsg).cast::<RawFd>();
                    received_fd = Some(*data_ptr);
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msghdr, cmsg);
        }
    }

    let raw_fd = received_fd.ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidData, "no fd received in control message")
    })?;

    let owned_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let msg = MmapAttachMessage::from_le_bytes(payload);
    Ok((owned_fd, msg))
}

impl MmapControlSender {
    /// Expose the sender fd for process-spawn handoff plumbing.
    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Consume this sender and return the owned raw fd.
    pub fn into_raw_fd(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        self.fd.into_raw_fd()
    }

    /// Send a file descriptor with metadata to the receiver.
    pub fn send(&self, fd: RawFd, msg: &MmapAttachMessage) -> io::Result<()> {
        send_fd_with_metadata(self.fd.as_raw_fd(), fd, msg)
    }
}

impl MmapControlReceiver {
    /// Non-blocking receive of one fd + metadata.
    pub fn try_recv(&self) -> io::Result<Option<(OwnedFd, MmapAttachMessage)>> {
        match recv_fd_with_metadata(self.async_fd.get_ref().as_raw_fd()) {
            Ok(pair) => Ok(Some(pair)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Async receive — waits for readiness then receives.
    pub async fn recv(&self) -> io::Result<(OwnedFd, MmapAttachMessage)> {
        loop {
            let mut guard = self.async_fd.ready(Interest::READABLE).await?;
            match guard.try_io(|inner| recv_fd_with_metadata(inner.get_ref().as_raw_fd())) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }
}

/// Create a paired mmap control channel.
///
/// Returns `(sender, receiver_handle)`. The receiver handle should be passed
/// to the peer process and reconstructed with `MmapControlReceiver::from_handle`.
pub fn create_mmap_control_pair() -> io::Result<(MmapControlSender, MmapControlHandle)> {
    let (sender_fd, receiver_fd) = create_dgram_pair()?;
    Ok((
        MmapControlSender { fd: sender_fd },
        MmapControlHandle(receiver_fd),
    ))
}

impl MmapControlReceiver {
    /// Reconstruct a receiver from a handle (in the peer process).
    pub fn from_handle(handle: MmapControlHandle) -> io::Result<Self> {
        let fd = handle.0;
        set_nonblocking(fd.as_raw_fd())?;
        let async_fd = AsyncFd::new(fd)?;
        Ok(Self { async_fd })
    }

    /// Create directly from an owned fd.
    ///
    /// # Safety
    /// The fd must be valid and from a socketpair.
    pub unsafe fn from_raw_fd(fd: RawFd) -> io::Result<Self> {
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        set_nonblocking(fd)?;
        let async_fd = AsyncFd::new(owned)?;
        Ok(Self { async_fd })
    }
}

impl MmapControlSender {
    /// Create directly from an owned fd.
    ///
    /// # Safety
    /// The fd must be valid and from a socketpair.
    pub unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        }
    }
}

/// Create a fully connected in-process pair (both sides ready to use).
pub fn create_mmap_control_pair_connected() -> io::Result<(MmapControlSender, MmapControlReceiver)>
{
    let (sender_fd, receiver_fd) = create_dgram_pair()?;
    set_nonblocking(receiver_fd.as_raw_fd())?;
    let async_fd = AsyncFd::new(receiver_fd)?;
    Ok((
        MmapControlSender { fd: sender_fd },
        MmapControlReceiver { async_fd },
    ))
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_fd_with_metadata() {
        let (sender, receiver_handle) = create_mmap_control_pair().unwrap();
        let receiver = MmapControlReceiver::from_handle(receiver_handle).unwrap();

        // Create a temp file to use as the fd
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let fd = tmp.as_file().as_raw_fd();

        let msg = MmapAttachMessage {
            map_id: 42,
            map_generation: 7,
            mapping_length: 65536,
        };

        sender.send(fd, &msg).unwrap();

        let (received_fd, received_msg) = receiver.recv().await.unwrap();
        assert_eq!(received_msg, msg);

        // Verify the received fd is valid and different from the original
        assert_ne!(received_fd.as_raw_fd(), fd);
        // Verify it's actually usable (can fstat it)
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::fstat(received_fd.as_raw_fd(), &mut stat) };
        assert_eq!(ret, 0);
    }

    #[test]
    fn attach_message_roundtrip() {
        let msg = MmapAttachMessage {
            map_id: 0xDEAD_BEEF,
            map_generation: 0xCAFE_BABE,
            mapping_length: 0x1234_5678_9ABC_DEF0,
        };
        let bytes = msg.to_le_bytes();
        let decoded = MmapAttachMessage::from_le_bytes(bytes);
        assert_eq!(msg, decoded);
    }

    #[tokio::test]
    async fn try_recv_returns_none_when_empty() {
        let (_sender, handle) = create_mmap_control_pair().unwrap();
        let receiver = MmapControlReceiver::from_handle(handle).unwrap();
        assert!(receiver.try_recv().unwrap().is_none());
    }
}
