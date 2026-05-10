//! Platform-specific positional read helpers that don't mutate the file cursor.
//!
//! Lets multiple threads read from a single `File` handle concurrently and
//! cuts one syscall per random read (no separate seek).

use std::fs::File;
use std::io;

/// Read exactly `buf.len()` bytes from `file` starting at absolute byte
/// offset `offset`. Loops until the buffer is full or returns
/// `UnexpectedEof`.
#[cfg(windows)]
pub fn pread_exact(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut total = 0;
    while total < buf.len() {
        let n = file.seek_read(&mut buf[total..], offset + total as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        total += n;
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes from `file` starting at absolute byte
/// offset `offset`.
#[cfg(unix)]
pub fn pread_exact(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}
