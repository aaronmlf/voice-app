//! Private OS integration for control IPC and user-owned files. Never called from audio callbacks.
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
pub(crate) use windows::*;

use anyhow::{Context, Result};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "local control frame deadline exceeded",
        ));
    }
    Ok(remaining)
}

fn normalize_timeout(error: io::Error) -> io::Error {
    // Unix socket timeouts can surface as WouldBlock; callers see one portable kind.
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ) {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "local control frame deadline exceeded",
        )
    } else {
        error
    }
}

/// A timeout belongs to the complete frame, rather than restarting for every
/// small read. Slow clients therefore cannot suspend the engine health loop.
pub(crate) struct DeadlineReader<'a> {
    stream: &'a mut LocalConnection,
    deadline: Instant,
}
impl<'a> DeadlineReader<'a> {
    pub(crate) fn new(stream: &'a mut LocalConnection, timeout: Duration) -> Self {
        Self {
            stream,
            deadline: Instant::now() + timeout,
        }
    }
}
impl Read for DeadlineReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        self.stream
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        let count = self.stream.read(bytes).map_err(normalize_timeout)?;
        remaining(self.deadline)?;
        Ok(count)
    }
}

/// Serialization and the terminating newline share one write deadline.
pub(crate) struct DeadlineWriter<'a> {
    stream: &'a mut LocalConnection,
    deadline: Instant,
}
impl<'a> DeadlineWriter<'a> {
    pub(crate) fn new(stream: &'a mut LocalConnection, timeout: Duration) -> Self {
        Self {
            stream,
            deadline: Instant::now() + timeout,
        }
    }
}
impl Write for DeadlineWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        self.stream
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        let count = self.stream.write(bytes).map_err(normalize_timeout)?;
        remaining(self.deadline)?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        self.stream.flush().map_err(normalize_timeout)?;
        remaining(self.deadline)?;
        Ok(())
    }
}

/// Persist in the same private directory before changing the live state. An unsuccessful
/// write or replacement leaves the destination intact; a stale private temp can be retried.
pub(crate) fn persist_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("private file needs a parent directory")?;
    private_directory(parent)?;
    let name = path.file_name().context("private file needs a filename")?;
    let mut temp_name = name.to_os_string();
    temp_name.push(".tmp");
    let temp = parent.join(temp_name);
    let mut file = private_temp_file(&temp)?;
    file.write_all(bytes)
        .context("write private temporary file")?;
    file.sync_all().context("sync private temporary file")?;
    drop(file);
    std::fs::rename(temp, path).context("replace private file")
}
