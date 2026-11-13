//! Sparse files for the parallel download.
//!
//! A file that is merely extended to its final size keeps a "valid data length" of zero, and
//! NTFS fills everything below the first far write with zeroes before that write returns: the
//! segment at the end of a 4 GiB file would wait for 3 GiB of zeroes. A sparse file has no such
//! debt; unwritten ranges simply read as zeroes and take no space.

use std::{fs::File, io, os::windows::io::AsRawHandle, path::Path, ptr};

use windows_sys::Win32::System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl};

/// Creates `path` (replacing what is there) as a sparse file of `len` bytes. Fails on volumes
/// without sparse files, such as FAT and exFAT.
pub(super) fn create_sparse_file(path: &Path, len: u64) -> io::Result<()> {
    let file = File::create(path)?;
    let mut returned = 0_u32;
    // SAFETY: `file` is an open handle with write access. FSCTL_SET_SPARSE without an input
    // buffer sets the flag and uses neither buffer; `returned` outlives the call.
    let marked = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            ptr::null(),
            0,
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if marked == 0 {
        return Err(io::Error::last_os_error());
    }
    file.set_len(len)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Seek, SeekFrom, Write},
        os::windows::fs::MetadataExt,
        time::{Duration, Instant},
    };

    use super::*;

    const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x0000_0200;
    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_far_write_into_a_large_sparse_file_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.part");
        let started = Instant::now();

        create_sparse_file(&path, 4 * GIB).unwrap();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(3 * GIB)).unwrap();
        file.write_all(b"tail").unwrap();
        file.sync_data().unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.len(), 4 * GIB);
        assert_ne!(metadata.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE, 0);
        // Zero-filling three gibibytes first would take far longer than this.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn creating_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reused.part");
        std::fs::write(&path, vec![0xAB_u8; 4096]).unwrap();

        create_sparse_file(&path, 1024).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), vec![0_u8; 1024]);
    }
}
