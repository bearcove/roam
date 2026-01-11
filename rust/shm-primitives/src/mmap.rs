//! File-backed memory-mapped regions for cross-process shared memory.
//!
//! This module provides `MmapRegion`, a file-backed memory region that can be
//! shared across processes using mmap with `MAP_SHARED`.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::Region;

/// File-backed memory-mapped region for cross-process shared memory.
///
/// shm[impl shm.file.mmap-posix]
pub struct MmapRegion {
    /// Pointer to the mapped memory
    ptr: *mut u8,
    /// Length of the mapping in bytes
    len: usize,
    /// The underlying file (kept open to maintain the mapping)
    #[allow(dead_code)]
    file: File,
    /// Path to the file (for cleanup)
    path: PathBuf,
    /// Whether this region owns the file (should delete on drop)
    owns_file: bool,
}

impl MmapRegion {
    /// Create a new file-backed region.
    ///
    /// This creates the file, truncates it to the given size, and maps it
    /// into memory with `MAP_SHARED`. The file is created with permissions 0600.
    ///
    /// shm[impl shm.file.create]
    /// shm[impl shm.file.permissions]
    pub fn create(path: &Path, size: usize) -> io::Result<Self> {
        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "size must be > 0",
            ));
        }

        // 1. Open or create file with read/write, truncate
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        // 2. Set permissions to 0600 (owner read/write only)
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;

        // 3. Truncate to desired size
        file.set_len(size as u64)?;

        // 4. mmap with MAP_SHARED
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            ptr: ptr as *mut u8,
            len: size,
            file,
            path: path.to_path_buf(),
            owns_file: true,
        })
    }

    /// Attach to an existing file-backed region.
    ///
    /// This opens the file and maps it into memory with `MAP_SHARED`.
    /// The file size determines the mapping size.
    ///
    /// shm[impl shm.file.attach]
    pub fn attach(path: &Path) -> io::Result<Self> {
        // Open existing file for read/write
        let file = OpenOptions::new().read(true).write(true).open(path)?;

        // Get file size
        let metadata = file.metadata()?;
        let size = metadata.len() as usize;

        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "segment file is empty",
            ));
        }

        // mmap with MAP_SHARED
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            ptr: ptr as *mut u8,
            len: size,
            file,
            path: path.to_path_buf(),
            owns_file: false, // Attached regions don't own the file
        })
    }

    /// Get a `Region` view of this mmap.
    #[inline]
    pub fn region(&self) -> Region {
        // SAFETY: The mmap is valid for the lifetime of MmapRegion
        unsafe { Region::from_raw(self.ptr, self.len) }
    }

    /// Get the size of the region in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the region is empty (zero bytes).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the path to the backing file.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Take ownership of the file for cleanup purposes.
    ///
    /// After calling this, the file will be deleted when this region is dropped.
    pub fn take_ownership(&mut self) {
        self.owns_file = true;
    }

    /// Release ownership of the file.
    ///
    /// After calling this, the file will NOT be deleted when this region is dropped.
    pub fn release_ownership(&mut self) {
        self.owns_file = false;
    }
}

impl Drop for MmapRegion {
    fn drop(&mut self) {
        // Unmap the memory
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.len);
        }

        // Delete the file if we own it
        // shm[impl shm.file.cleanup]
        if self.owns_file {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// SAFETY: The mmap region is valid for the lifetime of MmapRegion and can be
// safely accessed from multiple threads (the underlying memory is shared).
unsafe impl Send for MmapRegion {}
unsafe impl Sync for MmapRegion {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_attach() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.shm");

        // Create region
        let region1 = MmapRegion::create(&path, 4096).unwrap();
        assert_eq!(region1.len(), 4096);
        assert!(path.exists());

        // Write some data
        let data = region1.region();
        unsafe {
            std::ptr::write(data.as_ptr(), 0x42);
            std::ptr::write(data.as_ptr().add(1), 0x43);
        }

        // Attach from another "process" (same process, different mapping)
        let region2 = MmapRegion::attach(&path).unwrap();
        assert_eq!(region2.len(), 4096);

        // Verify data is visible
        let data2 = region2.region();
        unsafe {
            assert_eq!(std::ptr::read(data2.as_ptr()), 0x42);
            assert_eq!(std::ptr::read(data2.as_ptr().add(1)), 0x43);
        }
    }

    #[test]
    fn test_cleanup_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cleanup.shm");

        {
            let _region = MmapRegion::create(&path, 1024).unwrap();
            assert!(path.exists());
        }

        // File should be deleted after owner drops
        assert!(!path.exists());
    }

    #[test]
    fn test_attached_does_not_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attached.shm");

        let owner = MmapRegion::create(&path, 1024).unwrap();

        {
            let _attached = MmapRegion::attach(&path).unwrap();
            assert!(path.exists());
        }

        // File should still exist after attached drops
        assert!(path.exists());

        // File should be deleted after owner drops
        drop(owner);
        assert!(!path.exists());
    }

    #[test]
    fn test_shared_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.shm");

        let region1 = MmapRegion::create(&path, 4096).unwrap();
        let region2 = MmapRegion::attach(&path).unwrap();

        // Write from region2
        let data2 = region2.region();
        unsafe {
            std::ptr::write(data2.as_ptr().add(100), 0xAB);
        }

        // Read from region1
        let data1 = region1.region();
        unsafe {
            assert_eq!(std::ptr::read(data1.as_ptr().add(100)), 0xAB);
        }
    }

    #[test]
    fn test_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perms.shm");

        let _region = MmapRegion::create(&path, 1024).unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_zero_size_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zero.shm");

        let result = MmapRegion::create(&path, 0);
        assert!(result.is_err());
    }
}
