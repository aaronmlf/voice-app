use crate::service::Paths;
use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

pub(crate) type LocalConnection = UnixStream;

pub(crate) fn discover_paths() -> Result<Paths> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| anyhow!("HOME or XDG_DATA_HOME is required"))?
        .join("voice-app");
    // SAFETY: geteuid has no arguments or memory safety preconditions.
    let uid = unsafe { libc::geteuid() };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("voice-app-{uid}")));
    Ok(Paths {
        data,
        socket: runtime.join("voice-app/control.sock"),
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let meta = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or memory safety preconditions.
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
    {
        bail!(
            "{} must be a directory owned by this user with permissions 0700",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn private_create_new(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("create private file {}", path.display()))
}

pub(crate) fn private_temp_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    // SAFETY: geteuid has no arguments or memory safety preconditions.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        bail!("temporary file must be private and owned by this user");
    }
    file.set_len(0)?;
    Ok(file)
}

struct SocketGuard {
    path: PathBuf,
    _lock: File,
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) struct LocalListener {
    listener: UnixListener,
    _guard: SocketGuard,
}
impl LocalListener {
    pub(crate) fn bind(endpoint: &Path) -> Result<Self> {
        private_directory(
            endpoint
                .parent()
                .context("socket must have a parent directory")?,
        )?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(endpoint.with_extension("lock"))?;
        // SAFETY: lock is a live owned descriptor and flock does not retain pointers.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another engine owns {}", endpoint.display());
        }
        if let Ok(meta) = fs::symlink_metadata(endpoint) {
            // SAFETY: geteuid has no arguments or memory safety preconditions.
            if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
                bail!("refusing to replace non-owned socket path");
            }
            fs::remove_file(endpoint)?;
        }
        let listener = UnixListener::bind(endpoint)?;
        fs::set_permissions(endpoint, fs::Permissions::from_mode(0o600))?;
        let guard = SocketGuard {
            path: endpoint.to_owned(),
            _lock: lock,
        };
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            _guard: guard,
        })
    }

    pub(crate) fn try_accept(&mut self) -> io::Result<Option<LocalConnection>> {
        match self.listener.accept() {
            Ok((stream, _)) => Ok(Some(stream)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}
