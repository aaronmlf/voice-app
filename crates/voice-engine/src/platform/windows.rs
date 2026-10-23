//! User-only Windows control channel. No network listener and no audio callback work.
use crate::service::Paths;
use anyhow::{Context, Result, bail, ensure};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf};
use std::ptr::{addr_of_mut, null, null_mut};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::IO::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::*;

fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    ensure!(!value.contains(&0), "Windows path contains a NUL");
    value.push(0);
    Ok(value)
}

fn owned_handle(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful Win32 creation transferred this unique handle to the caller.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}

/// GetTokenInformation requires a naturally aligned TOKEN_USER buffer. Its embedded
/// SID stays valid because the allocation is never resized after the call.
struct Identity {
    token_user: Vec<usize>,
}
impl Identity {
    fn current() -> Result<Self> {
        // SAFETY: the current process pseudo-handle is valid for token queries.
        Self::process(unsafe { GetCurrentProcess() })
    }
    fn process(process: HANDLE) -> Result<Self> {
        let mut token = null_mut();
        // SAFETY: process is valid, token is a writable out-pointer.
        ensure!(
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
            "cannot query Windows user token: {}",
            io::Error::last_os_error()
        );
        let token = owned_handle(token)?;
        let mut needed = 0;
        // SAFETY: a null buffer with zero bytes requests the required length.
        unsafe { GetTokenInformation(raw(&token), TokenUser, null_mut(), 0, &mut needed) };
        ensure!(
            (size_of::<TOKEN_USER>() as u32..=65536).contains(&needed),
            "invalid Windows token size"
        );
        let mut token_user = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        // SAFETY: allocation is aligned and has at least the returned byte count.
        ensure!(
            unsafe {
                GetTokenInformation(
                    raw(&token),
                    TokenUser,
                    token_user.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            } != 0,
            "cannot read Windows user token: {}",
            io::Error::last_os_error()
        );
        Ok(Self { token_user })
    }
    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation initialized this aligned TOKEN_USER and its SID.
        unsafe { (*self.token_user.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
    fn text(&self) -> Result<String> {
        let mut text = null_mut();
        // SAFETY: sid is token-owned and valid, text is a writable out-pointer.
        ensure!(
            unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } != 0,
            "cannot encode Windows user SID"
        );
        let allocation = LocalAllocation(text.cast());
        let mut length = 0;
        // SAFETY: successful SID conversion yields a terminated UTF-16 allocation.
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
        }
        // SAFETY: length is bounded by the terminated string just inspected.
        let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })?;
        drop(allocation);
        Ok(result)
    }
    fn matches(&self, sid: PSID) -> bool {
        // SAFETY: both SIDs are supplied by the token/security APIs while alive.
        !sid.is_null() && unsafe { EqualSid(self.sid(), sid) } != 0
    }
}

struct LocalAllocation(*mut std::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: this allocation came from a Win32 API documented to use LocalFree.
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct PrivateSecurity {
    descriptor: LocalAllocation,
}
impl PrivateSecurity {
    fn new(identity: &Identity, directory: bool) -> Result<Self> {
        // Protected DACL: only the current account has access. Directories pass
        // that rule to child files/SQLite journals, without inheriting broader ACEs.
        let flags = if directory { "OICI" } else { "" };
        let sid = identity.text()?;
        let sddl = format!("O:{sid}D:P(A;{flags};FA;;;{sid})");
        let text = wide(std::ffi::OsStr::new(&sddl))?;
        let mut descriptor = null_mut();
        // SAFETY: SDDL is terminated and output pointers are valid.
        ensure!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    null_mut(),
                )
            } != 0,
            "cannot create private Windows security descriptor: {}",
            io::Error::last_os_error()
        );
        Ok(Self {
            descriptor: LocalAllocation(descriptor),
        })
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }
}

fn verify_private(handle: HANDLE, identity: &Identity) -> Result<()> {
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: handle is a live file/directory handle; all outputs are writable.
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    ensure!(
        result == ERROR_SUCCESS,
        "cannot inspect private Windows ACL: {}",
        io::Error::from_raw_os_error(result as i32)
    );
    let allocation = LocalAllocation(descriptor);
    ensure!(
        identity.matches(owner),
        "private path is not owned by the current Windows user"
    );
    ensure!(
        !dacl.is_null(),
        "private path has an unrestricted Windows DACL"
    );
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor remains allocated for the complete inspection.
    ensure!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } != 0
            && control & SE_DACL_PROTECTED != 0,
        "private path must have a protected DACL"
    );
    // SAFETY: zero is a valid initial state for an ACL_SIZE_INFORMATION out-buffer.
    let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    // SAFETY: dacl is owned by descriptor and the info buffer has the specified size.
    ensure!(
        unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } != 0,
        "cannot inspect private Windows access entries"
    );
    ensure!(
        info.AceCount > 0,
        "private Windows path has no user access entry"
    );
    for index in 0..info.AceCount {
        let mut ace = null_mut();
        // SAFETY: index is bounded by the ACL-reported count and out-pointer is valid.
        ensure!(
            unsafe { GetAce(dacl, index, &mut ace) } != 0,
            "cannot inspect Windows access entry"
        );
        // SAFETY: GetAce supplied an initialized ACE_HEADER.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        ensure!(
            u32::from(header.AceType) == ACCESS_ALLOWED_ACE_TYPE
                && header.AceSize as usize >= size_of::<ACCESS_ALLOWED_ACE>(),
            "private Windows path has an unexpected access entry"
        );
        // SAFETY: the type and minimum ACE length were checked above.
        let allow = unsafe { &mut *ace.cast::<ACCESS_ALLOWED_ACE>() };
        ensure!(
            identity.matches(addr_of_mut!(allow.SidStart).cast()),
            "private Windows path grants access to another account"
        );
    }
    drop(allocation);
    Ok(())
}

fn reject_reparse_ancestors(path: &Path) -> Result<()> {
    let absolute = std::path::absolute(path)?;
    let mut current = PathBuf::new();
    for component in absolute.components() {
        ensure!(
            !matches!(component, Component::ParentDir),
            "private path must not contain .."
        );
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "private paths must not contain Windows reparse points: {}",
                current.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn reject_reparse_handle(handle: HANDLE) -> Result<()> {
    // SAFETY: zero is valid as an initial file-info out-buffer.
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { zeroed() };
    // SAFETY: the live disk handle supports this query; buffer and size match.
    ensure!(
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } != 0,
        "cannot inspect Windows path attributes: {}",
        io::Error::last_os_error()
    );
    ensure!(
        info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "private path must not be a Windows reparse point"
    );
    Ok(())
}

pub(crate) fn discover_paths() -> Result<Paths> {
    let root = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is required on Windows")?;
    let data = PathBuf::from(root).join("voice-app");
    ensure!(data.is_absolute(), "LOCALAPPDATA must be absolute");
    // This is a logical endpoint. Its user SID and path determine a local pipe;
    // the CLI/GUI can keep --socket and independent test/session paths unchanged.
    Ok(Paths {
        socket: data.join("runtime/control.sock"),
        data,
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    reject_reparse_ancestors(path)?;
    let identity = Identity::current()?;
    let security = PrivateSecurity::new(&identity, true)?;
    create_private_directory(path, &security)?;
    let text = wide(path.as_os_str())?;
    // Do not follow the final component; the handle is inspected below as well.
    // SAFETY: path is terminated; no borrowed pointer escapes this synchronous call.
    let handle = owned_handle(unsafe {
        CreateFileW(
            text.as_ptr(),
            READ_CONTROL | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    })?;
    reject_reparse_handle(raw(&handle))?;
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "private path must be a directory"
    );
    verify_private(raw(&handle), &identity)
}

fn create_private_directory(path: &Path, security: &PrivateSecurity) -> Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !parent.exists()
        {
            create_private_directory(parent, security)?;
        }
        let text = wide(path.as_os_str())?;
        let attributes = security.attributes();
        // SAFETY: path and security descriptor remain alive through this call.
        if unsafe { CreateDirectoryW(text.as_ptr(), &attributes) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

pub(crate) fn private_create_new(path: &Path) -> Result<File> {
    reject_reparse_ancestors(path)?;
    let identity = Identity::current()?;
    let security = PrivateSecurity::new(&identity, false)?;
    let attributes = security.attributes();
    let text = wide(path.as_os_str())?;
    // CREATE_NEW never overwrites an existing file, including a reparse point.
    // SAFETY: the terminated path and descriptor remain valid through the call.
    let handle = owned_handle(unsafe {
        CreateFileW(
            text.as_ptr(),
            GENERIC_WRITE | READ_CONTROL | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    })?;
    if let Err(error) =
        reject_reparse_handle(raw(&handle)).and_then(|()| verify_private(raw(&handle), &identity))
    {
        drop(handle);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(File::from(handle))
}

pub(crate) fn private_temp_file(path: &Path) -> Result<File> {
    match private_create_new(path) {
        Ok(file) => Ok(file),
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::raw_os_error)
                .is_some_and(|code| {
                    code == ERROR_FILE_EXISTS as i32 || code == ERROR_ALREADY_EXISTS as i32
                }) =>
        {
            reject_reparse_ancestors(path)?;
            let identity = Identity::current()?;
            let text = wide(path.as_os_str())?;
            // Open without truncating; verify ownership, ACL and type first.
            // SAFETY: path is terminated and no pointers escape the call.
            let handle = owned_handle(unsafe {
                CreateFileW(
                    text.as_ptr(),
                    GENERIC_WRITE | READ_CONTROL | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    null_mut(),
                )
            })?;
            reject_reparse_handle(raw(&handle))?;
            verify_private(raw(&handle), &identity)?;
            let file = File::from(handle);
            ensure!(
                file.metadata()?.is_file(),
                "temporary path must be a regular file"
            );
            file.set_len(0)?;
            Ok(file)
        }
        Err(error) => Err(error),
    }
}

fn pipe_name(endpoint: &Path, identity: &Identity) -> Result<Vec<u16>> {
    // Hash UTF-16, preserving non-Unicode paths. A versioned explicit FNV-1a
    // algorithm keeps client/server names stable across Rust toolchains.
    let path = std::path::absolute(endpoint)?;
    let mut hash = 0xcbf29ce484222325u64;
    for word in path.as_os_str().encode_wide() {
        for byte in word.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    let name = format!(r"\\.\pipe\voice-app-v1-{}-{hash:016x}", identity.text()?);
    wide(std::ffi::OsStr::new(&name))
}

fn event() -> io::Result<OwnedHandle> {
    // SAFETY: unnamed, manual-reset event; no inherited security or borrowed state.
    owned_handle(unsafe { CreateEventW(null(), 1, 0, null()) })
}

/// A pending OVERLAPPED must stay at a stable address until completion/cancel.
/// This owner also keeps the file handle and event alive for that whole period.
struct Pending {
    file: Arc<OwnedHandle>,
    event: OwnedHandle,
    overlapped: Box<OVERLAPPED>,
    active: bool,
}
// SAFETY: Pending is moved only as an owner; OVERLAPPED itself is heap-stable and
// no concurrent access occurs. All raw handles are owned for the operation lifetime.
unsafe impl Send for Pending {}
impl Pending {
    fn new(file: Arc<OwnedHandle>) -> io::Result<Self> {
        let event = event()?;
        // SAFETY: zero initializes the documented unused OVERLAPPED fields.
        let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { zeroed() });
        overlapped.hEvent = raw(&event);
        Ok(Self {
            file,
            event,
            overlapped,
            active: false,
        })
    }
    fn pointer(&mut self) -> *mut OVERLAPPED {
        &mut *self.overlapped
    }
    fn finish(&mut self, timeout: Duration) -> io::Result<usize> {
        let milliseconds = timeout.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32;
        // SAFETY: event is owned and remains live for the wait.
        match unsafe { WaitForSingleObject(raw(&self.event), milliseconds) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                self.cancel();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "local control I/O timed out",
                ));
            }
            _ => {
                let error = io::Error::last_os_error();
                self.cancel();
                return Err(error);
            }
        }
        let mut bytes = 0;
        // SAFETY: event signaled completion for this live OVERLAPPED and handle.
        let success =
            unsafe { GetOverlappedResult(raw(&self.file), &*self.overlapped, &mut bytes, 0) };
        if success == 0 {
            let error = io::Error::last_os_error();
            // A spurious event must never release an operation still using a buffer.
            if error.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) {
                self.cancel();
            } else {
                self.active = false;
            }
            return Err(error);
        }
        self.active = false;
        Ok(bytes as usize)
    }
    fn cancel(&mut self) {
        if self.active {
            let mut ignored = 0;
            // SAFETY: operation and buffer must remain alive until canceled I/O is
            // observed completed. CancelIoEx alone would not guarantee that.
            unsafe {
                CancelIoEx(raw(&self.file), &*self.overlapped);
                GetOverlappedResult(raw(&self.file), &*self.overlapped, &mut ignored, 1);
            }
            self.active = false;
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) struct LocalListener {
    file: Arc<OwnedHandle>,
    connect: Pending,
    connected: Arc<AtomicBool>,
}
impl LocalListener {
    pub(crate) fn bind(endpoint: &Path) -> Result<Self> {
        let identity = Identity::current()?;
        let name = pipe_name(endpoint, &identity)?;
        let security = PrivateSecurity::new(&identity, false)?;
        let attributes = security.attributes();
        // One retained instance owns the name until the service stops. It is
        // reused after each client, so exclusivity has no between-client gap.
        // SAFETY: name/ACL are live, handles are non-inheritable, buffers bounded.
        let file = Arc::new(
            owned_handle(unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    8192,
                    8192,
                    3000,
                    &attributes,
                )
            })
            .context("create private local control pipe (another engine may own it)")?,
        );
        let connect = Pending::new(file.clone())?;
        Ok(Self {
            file,
            connect,
            connected: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(crate) fn try_accept(&mut self) -> io::Result<Option<LocalConnection>> {
        if self.connected.load(Ordering::Acquire) {
            return Ok(None);
        }
        if !self.connect.active {
            // SAFETY: no operation uses this event/OVERLAPPED until the next call.
            unsafe {
                ResetEvent(raw(&self.connect.event));
            }
            let pointer = self.connect.pointer();
            // SAFETY: Pending owns the stable OVERLAPPED, event and handle.
            if unsafe { ConnectNamedPipe(raw(&self.file), pointer) } == 0 {
                let error = io::Error::last_os_error();
                match error.raw_os_error().map(|value| value as u32) {
                    Some(ERROR_IO_PENDING) => self.connect.active = true,
                    Some(ERROR_PIPE_CONNECTED) => return Ok(Some(self.connection())),
                    Some(ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED | ERROR_BROKEN_PIPE) => {
                        // A client can close between CreateFile and this accept.
                        // SAFETY: no pending connect was registered in this branch.
                        unsafe {
                            DisconnectNamedPipe(raw(&self.file));
                        }
                        return Ok(None);
                    }
                    _ => return Err(error),
                }
            } else {
                return Ok(Some(self.connection()));
            }
        }
        // SAFETY: the pending connection owns this live event; zero never blocks.
        match unsafe { WaitForSingleObject(raw(&self.connect.event), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                match self.connect.finish(Duration::from_millis(1)) {
                    Ok(_) => Ok(Some(self.connection())),
                    Err(error)
                        if matches!(
                            error.raw_os_error().map(|value| value as u32),
                            Some(ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED | ERROR_BROKEN_PIPE)
                        ) =>
                    {
                        // SAFETY: finish observed completion, so the instance is idle.
                        unsafe {
                            DisconnectNamedPipe(raw(&self.file));
                        }
                        Ok(None)
                    }
                    Err(error) => Err(error),
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
    fn connection(&self) -> LocalConnection {
        self.connected.store(true, Ordering::Release);
        LocalConnection {
            file: self.file.clone(),
            server_connected: Some(self.connected.clone()),
            response_written: false,
            read_timeout: Duration::from_secs(2),
            write_timeout: Duration::from_secs(3),
        }
    }
}

pub(crate) struct LocalConnection {
    file: Arc<OwnedHandle>,
    server_connected: Option<Arc<AtomicBool>>,
    response_written: bool,
    read_timeout: Duration,
    write_timeout: Duration,
}
impl LocalConnection {
    pub(crate) fn connect(endpoint: &Path) -> Result<Self> {
        let identity = Identity::current()?;
        let name = pipe_name(endpoint, &identity)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let file = loop {
            // SECURITY_IDENTIFICATION prevents a server from impersonating the client.
            // SAFETY: name is local, terminated and remains live through this call.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                break Arc::new(owned_handle(handle)?);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
                return Err(error.into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("local control connection timed out");
            }
            // SAFETY: local terminated name; timeout is bounded by the connection deadline.
            if unsafe { WaitNamedPipeW(name.as_ptr(), remaining.as_millis().clamp(1, 3000) as u32) }
                == 0
            {
                return Err(io::Error::last_os_error().into());
            }
        };
        let mut process_id = 0;
        // A user-SID suffix alone does not authenticate a pre-created pipe.
        // SAFETY: file is a connected local pipe and output pointer is valid.
        ensure!(
            unsafe { GetNamedPipeServerProcessId(raw(&file), &mut process_id) } != 0,
            "cannot authenticate local control server"
        );
        // SAFETY: process ID was reported by the kernel; query-only access is requested.
        let process =
            owned_handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) })?;
        let server = Identity::process(raw(&process))?;
        ensure!(
            identity.matches(server.sid()),
            "local control server belongs to another Windows user"
        );
        Ok(Self {
            file,
            server_connected: None,
            response_written: false,
            read_timeout: Duration::from_secs(20),
            write_timeout: Duration::from_secs(3),
        })
    }
    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.read_timeout = timeout.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "unbounded pipe I/O is disabled",
            )
        })?;
        Ok(())
    }
    pub(crate) fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.write_timeout = timeout.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "unbounded pipe I/O is disabled",
            )
        })?;
        Ok(())
    }
}
impl Read for LocalConnection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut pending = Pending::new(self.file.clone())?;
        let mut count = 0;
        let pointer = pending.pointer();
        // SAFETY: bytes and Pending remain alive until completion, including cancellation.
        let success = unsafe {
            ReadFile(
                raw(&self.file),
                bytes.as_mut_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                &mut count,
                pointer,
            )
        };
        if success != 0 {
            return Ok(count as usize);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|value| value as u32) {
            Some(ERROR_IO_PENDING) => {
                pending.active = true;
                match pending.finish(self.read_timeout) {
                    Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
                    result => result,
                }
            }
            Some(ERROR_BROKEN_PIPE) => Ok(0),
            _ => Err(error),
        }
    }
}
impl Write for LocalConnection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut pending = Pending::new(self.file.clone())?;
        let mut count = 0;
        let pointer = pending.pointer();
        // SAFETY: bytes and Pending remain alive until completion, including cancellation.
        let success = unsafe {
            WriteFile(
                raw(&self.file),
                bytes.as_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                &mut count,
                pointer,
            )
        };
        if success != 0 {
            self.response_written |= count > 0;
            return Ok(count as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error);
        }
        pending.active = true;
        let count = pending.finish(self.write_timeout)?;
        self.response_written |= count > 0;
        Ok(count)
    }
    // FlushFileBuffers on a named pipe can block indefinitely until the peer reads.
    // write_all already waits for all bytes to enter the bounded kernel pipe.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for LocalConnection {
    fn drop(&mut self) {
        if self.server_connected.is_some() && self.response_written {
            // DisconnectNamedPipe discards unread output. The client closes after
            // reading its one response, so wait for that close with the same bounded
            // read timeout; never use an unbounded FlushFileBuffers here.
            // The preceding frame reader may have reduced its remaining timeout
            // to a few milliseconds. Closing is a separate, bounded operation.
            self.read_timeout = Duration::from_secs(2);
            let _ = self.read(&mut [0u8; 1]);
        }
        if let Some(connected) = self.server_connected.take() {
            // SAFETY: all synchronous wrappers drained their overlapped operations.
            unsafe {
                DisconnectNamedPipe(raw(&self.file));
            }
            connected.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipe_name_is_local_user_scoped_and_stable() -> Result<()> {
        let identity = Identity::current()?;
        let first = pipe_name(Path::new(r"C:\voice-app-test\control.sock"), &identity)?;
        assert_eq!(
            first,
            pipe_name(Path::new(r"C:\voice-app-test\control.sock"), &identity)?
        );
        assert_ne!(
            first,
            pipe_name(Path::new(r"C:\other\control.sock"), &identity)?
        );
        assert!(String::from_utf16_lossy(&first).starts_with(r"\\.\pipe\voice-app-v1-"));
        Ok(())
    }
    #[test]
    fn private_directory_and_file_have_only_current_user_access() -> Result<()> {
        let root = std::env::temp_dir().join(format!("voice-windows-acl-{}", std::process::id()));
        private_directory(&root)?;
        let path = root.join("private.json");
        let file = private_create_new(&path)?;
        verify_private(file.as_raw_handle(), &Identity::current()?)?;
        assert!(private_create_new(&path).is_err());
        drop(file);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn idle_pipe_read_is_canceled_before_its_buffer_is_released() -> Result<()> {
        let endpoint =
            std::env::temp_dir().join(format!("voice-windows-timeout-{}.sock", std::process::id()));
        let mut listener = LocalListener::bind(&endpoint)?;
        assert!(LocalListener::bind(&endpoint).is_err());
        let mut client = LocalConnection::connect(&endpoint)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let _server = loop {
            if let Some(connection) = listener.try_accept()? {
                break connection;
            }
            ensure!(Instant::now() < deadline, "local pipe accept timed out");
            std::thread::sleep(Duration::from_millis(1));
        };
        client.set_read_timeout(Some(Duration::from_millis(20)))?;
        let started = Instant::now();
        let error = client.read(&mut [0u8; 8]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        Ok(())
    }
}
