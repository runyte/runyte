// SPDX-License-Identifier: MPL-2.0

//! ConPTY ownership. Each terminal has independent reader, writer and lifecycle
//! threads. The editor only queues bounded input and coalesced resize requests.
//! Closing a pane kills its job; draining and ClosePseudoConsole run off-loop.
#[cfg(test)]
use std::sync::Condvar;
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{Console::*, JobObjects::*, Pipes::CreatePipe, Threading::*},
};

pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
const INPUT_QUEUE: usize = 8;
const READ_CHUNK: usize = 64 * 1024;

#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
}

/// Used only while a plugin terminal is prepared but has not been installed.
/// Its process stays suspended until native editor ownership is established.
#[derive(Clone)]
pub(super) struct PendingActivation {
    pub(super) wait: Arc<dyn Fn() -> bool + Send + Sync>,
    pub(super) cancel: Arc<dyn Fn() + Send + Sync>,
    pub(super) lifetime: Arc<dyn Send + Sync>,
}

/// Keeps unpublished accounting attached to every partial setup owner. Once
/// all ConPTY threads exist, the activation owner alone carries that lease.
struct SetupRetention(Mutex<Option<Arc<dyn Send + Sync>>>);

struct Input {
    bytes: Vec<u8>,
    delivery: Option<super::proposal::Delivery>,
}
impl Drop for Input {
    fn drop(&mut self) {
        if let Some(delivery) = &self.delivery {
            delivery.abandoned();
        }
    }
}

struct Control {
    stop: OwnedHandle,
    readable: OwnedHandle,
    resize: OwnedHandle,
    dimensions: AtomicU32,
    input: Mutex<VecDeque<Input>>,
    completed: OwnedHandle,
    #[cfg(test)]
    cleanup_hold: Arc<(Mutex<bool>, Condvar)>,
}
impl Control {
    fn stop(&self) {
        unsafe {
            SetEvent(self.stop.as_raw_handle());
        }
        self.input.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    fn stopped(&self) -> bool {
        unsafe { WaitForSingleObject(self.stop.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }
    fn enqueue(&self, input: Input) -> io::Result<()> {
        let mut pending = self.input.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopped() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Terminal input writer is closed",
            ));
        }
        if pending.len() >= INPUT_QUEUE {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Terminal input queue is full",
            ));
        }
        pending.push_back(input);
        unsafe {
            SetEvent(self.readable.as_raw_handle());
        }
        Ok(())
    }
}

struct Console {
    handle: HPCON,
    undrained_output: Option<OwnedHandle>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum SpawnCheckpoint {
    ChildOwned,
    ReaderStarted,
    WriterStarted,
    LifecycleStarted,
}
impl Drop for Console {
    fn drop(&mut self) {
        // Early setup failures have no reader yet. Close its pipe before the
        // console, whose final frame must never wait on an undrained pipe.
        drop(self.undrained_output.take());
        unsafe {
            ClosePseudoConsole(self.handle);
        }
    }
}
struct Attributes(Vec<usize>);
impl Attributes {
    fn new(console: HPCON) -> io::Result<Self> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        }
        let mut storage = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
        if unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut bytes)
        } == 0
        {
            // Initialization failed, so DeleteProcThreadAttributeList is not valid.
            return Err(io::Error::last_os_error());
        }
        let mut attributes = Self(storage);
        if unsafe {
            UpdateProcThreadAttribute(
                attributes.0.as_mut_ptr().cast(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const _,
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(attributes)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}

/// A suspended process remains owned through every fallible setup step.
struct SpawnGuard {
    process: Arc<OwnedHandle>,
    control: Arc<Control>,
    cancel_pending: Option<Arc<dyn Fn() + Send + Sync>>,
    armed: bool,
}
impl Drop for SpawnGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(cancel) = self.cancel_pending.take() {
                cancel();
            }
            self.control.stop();
            unsafe {
                TerminateProcess(self.process.as_raw_handle(), 1);
                // Every guarded failure precedes activation, so the child is
                // still suspended and this wait runs on the handoff worker.
                // Keep setup accounting alive until the exact owned process
                // has completed termination; later checkpoints retain the
                // same lease in their reader, writer and lifecycle owners.
                WaitForSingleObject(self.process.as_raw_handle(), INFINITE);
            }
        }
    }
}

pub struct Pty {
    process: Arc<OwnedHandle>,
    job: Arc<OwnedHandle>,
    control: Arc<Control>,
    pid: u32,
}
impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}
fn event(manual: bool) -> io::Result<OwnedHandle> {
    owned(unsafe { CreateEventW(ptr::null(), manual as i32, 0, ptr::null()) })
}
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((owned(read)?, owned(write)?))
}
fn hresult(result: i32) -> io::Result<()> {
    if result >= 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ConPTY failed (HRESULT 0x{:08x})",
            result as u32
        )))
    }
}
fn dimensions(columns: u16, rows: u16) -> COORD {
    COORD {
        X: columns.clamp(1, i16::MAX as u16) as i16,
        Y: rows.clamp(1, i16::MAX as u16) as i16,
    }
}

/// Framework-based console programs can reject their own configuration path
/// when launched with an extended executable spelling. Prefer an ordinary
/// spelling only after checking native identity. Paths that require extended
/// syntax retain the previous behavior; metadata/identity failures do not fall
/// back silently. This does not change the separate working-directory contract.
fn executable_path(path: &Path) -> io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    let mut parts = canonical.components();
    let mut ordinary = match parts.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                PathBuf::from(format!("{}:", drive as char))
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                let mut value = OsString::from(r"\\");
                value.push(server);
                value.push(r"\");
                value.push(share);
                PathBuf::from(value)
            }
            _ => return Ok(canonical),
        },
        _ => return Ok(canonical),
    };
    for part in parts {
        if let Component::Normal(name) = part
            && crate::windows_fs::validate_relative(Path::new(name)).is_err()
        {
            return Ok(canonical);
        }
        ordinary.push(part.as_os_str());
    }
    if ordinary.as_os_str().encode_wide().count() >= 260 {
        return Ok(canonical);
    }
    if crate::windows_fs::Identity::read(&ordinary)?
        != crate::windows_fs::Identity::read(&canonical)?
    {
        return Err(io::Error::other(
            "terminal executable changed while resolving its Windows spelling",
        ));
    }
    Ok(ordinary)
}

impl Pty {
    #[cfg(test)]
    pub(crate) fn cleanup_waiter(&self) -> Box<dyn FnOnce()> {
        let control = Arc::clone(&self.control);
        Box::new(move || {
            assert_eq!(
                unsafe { WaitForSingleObject(control.completed.as_raw_handle(), 5000) },
                WAIT_OBJECT_0,
                "ConPTY lifecycle completed before fixture storage removal"
            )
        })
    }
    #[cfg(test)]
    pub(super) fn hold_cleanup_for_test(&self) -> Box<dyn FnOnce()> {
        let hold = self.control.cleanup_hold.clone();
        *hold.0.lock().unwrap_or_else(|error| error.into_inner()) = true;
        Box::new(move || {
            *hold.0.lock().unwrap_or_else(|error| error.into_inner()) = false;
            hold.1.notify_all();
        })
    }
    pub fn process_id(&self) -> u32 {
        self.pid
    }

    pub(super) fn contains_live_peer(
        &self,
        peer: &crate::workspace::windows_process_identity::PinnedProcess,
    ) -> io::Result<bool> {
        if self.control.stopped() {
            return Ok(false);
        }
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => return Ok(false),
            WAIT_TIMEOUT => {}
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected terminal leader wait result",
                ));
            }
        }
        if !peer.is_alive()? {
            return Ok(false);
        }
        let mut member = 0;
        if unsafe {
            IsProcessInJob(
                peer.as_handle().as_raw_handle(),
                self.job.as_raw_handle(),
                &mut member,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(member != 0)
    }
    pub fn spawn(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        events: impl Fn(PtyEvent) + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_in_context(program, arguments, directory, columns, rows, None, events)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_in_context(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        parent_context: Option<&str>,
        events: impl Fn(PtyEvent) + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_checked(
            program,
            arguments,
            directory,
            columns,
            rows,
            parent_context,
            events,
            |_, _| Ok(()),
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_gated_in_context(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        parent_context: Option<&str>,
        events: impl Fn(PtyEvent) + Send + 'static,
        activation: PendingActivation,
    ) -> io::Result<Self> {
        Self::spawn_checked(
            program,
            arguments,
            directory,
            columns,
            rows,
            parent_context,
            events,
            |_, _| Ok(()),
            Some(activation),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn spawn_checked(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        parent_context: Option<&str>,
        events: impl Fn(PtyEvent) + Send + 'static,
        mut checkpoint: impl FnMut(SpawnCheckpoint, u32) -> io::Result<()>,
        activation: Option<PendingActivation>,
    ) -> io::Result<Self> {
        let setup_retention = activation.as_ref().map(|activation| {
            Arc::new(SetupRetention(Mutex::new(Some(
                activation.lifetime.clone(),
            ))))
        });
        let program = super::windows_command::resolve_for_terminal(
            Path::new(program),
            directory,
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        )
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "terminal executable was not found on PATH",
            )
        })?;
        let program = executable_path(&program)?;
        if program
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "batch files require an explicit cmd.exe /c command",
            ));
        }
        let application = crate::windows_fs::wide(&program)?;
        let mut command = super::windows_command::line(program.as_os_str(), arguments)?;
        let directory = super::windows_command::working_directory(directory)?;
        if program
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
            && matches!(directory.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::UNC(_, _)))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cmd.exe cannot use a UNC working directory; choose a shell with UNC support",
            ));
        }
        let directory = crate::windows_fs::wide(&directory)?;
        let environment = super::windows_command::environment(parent_context);
        let control = Arc::new(Control {
            stop: event(true)?,
            readable: event(false)?,
            resize: event(false)?,
            dimensions: AtomicU32::new(columns as u32 | ((rows as u32) << 16)),
            input: Mutex::new(VecDeque::new()),
            completed: event(true)?,
            #[cfg(test)]
            cleanup_hold: Arc::new((Mutex::new(false), Condvar::new())),
        });
        let job = Arc::new(crate::windows_process::new_job()?);
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let mut console = 0;
        hresult(unsafe {
            CreatePseudoConsole(
                dimensions(columns, rows),
                input_read.as_raw_handle(),
                output_write.as_raw_handle(),
                0,
                &mut console,
            )
        })?;
        let mut console = Console {
            handle: console,
            undrained_output: Some(output_read),
        };
        drop(input_read);
        drop(output_write);
        let mut attributes = Attributes::new(console.handle)?;
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of_val(&startup) as u32;
        // NULL standard handles explicitly bind the child to this ConPTY,
        // including when Runyte's own standard streams are redirected.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
        let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let process = Arc::new(owned(info.hProcess)?);
        let primary_thread = owned(info.hThread)?;
        let mut guard = SpawnGuard {
            process: process.clone(),
            control: control.clone(),
            cancel_pending: activation
                .as_ref()
                .map(|activation| activation.cancel.clone()),
            armed: true,
        };
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        checkpoint(SpawnCheckpoint::ChildOwned, info.dwProcessId)?;
        let reader_process = process.clone();
        let reader_control = control.clone();
        let reader_retention = setup_retention.clone();
        let output_read = console
            .undrained_output
            .take()
            .expect("reader has not started");
        let reader = thread::Builder::new()
            .name("runyte-conpty-read".into())
            .spawn(move || {
                let _retention = reader_retention;
                let mut file = File::from(output_read);
                let mut bytes = vec![0; READ_CHUNK];
                loop {
                    match file.read(&mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => events(PtyEvent::Output(bytes[..count].to_vec())),
                    }
                }
                reader_control.stop();
                unsafe {
                    WaitForSingleObject(reader_process.as_raw_handle(), INFINITE);
                }
                events(PtyEvent::Exited(exit_code(&reader_process)));
            })?;
        checkpoint(SpawnCheckpoint::ReaderStarted, info.dwProcessId)?;
        let writer_control = control.clone();
        let writer_retention = setup_retention.clone();
        let writer = thread::Builder::new()
            .name("runyte-conpty-write".into())
            .spawn(move || {
                let _retention = writer_retention;
                let mut file = File::from(input_write);
                let handles = [
                    writer_control.stop.as_raw_handle(),
                    writer_control.readable.as_raw_handle(),
                ];
                loop {
                    if unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) }
                        != WAIT_OBJECT_0 + 1
                    {
                        break;
                    }
                    loop {
                        if writer_control.stopped() {
                            return;
                        }
                        let Some(input) = writer_control
                            .input
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .pop_front()
                        else {
                            break;
                        };
                        if input
                            .delivery
                            .as_ref()
                            .is_some_and(|delivery| !delivery.claim())
                        {
                            continue;
                        }
                        for chunk in input.bytes.chunks(16 * 1024) {
                            if writer_control.stopped() || file.write_all(chunk).is_err() {
                                writer_control.stop();
                                return;
                            }
                        }
                        if let Some(delivery) = &input.delivery {
                            delivery.complete();
                        }
                    }
                }
            })?;
        checkpoint(SpawnCheckpoint::WriterStarted, info.dwProcessId)?;
        let lifecycle_control = control.clone();
        let lifecycle_process = process.clone();
        let lifecycle_job = job.clone();
        let lifecycle_pid = info.dwProcessId;
        let lifecycle_retention = setup_retention.clone();
        thread::Builder::new()
            .name("runyte-conpty-lifecycle".into())
            .spawn(move || {
                let _retention = lifecycle_retention;
                let console = console;
                let handles = [
                    lifecycle_control.stop.as_raw_handle(),
                    lifecycle_process.as_raw_handle(),
                    lifecycle_control.resize.as_raw_handle(),
                ];
                loop {
                    if unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, INFINITE) }
                        != WAIT_OBJECT_0 + 2
                    {
                        break;
                    }
                    let size = lifecycle_control.dimensions.load(Ordering::Acquire);
                    if hresult(unsafe {
                        ResizePseudoConsole(
                            console.handle,
                            dimensions(size as u16, (size >> 16) as u16),
                        )
                    })
                    .is_err()
                    {
                        break;
                    }
                }
                lifecycle_control.stop();
                unsafe {
                    TerminateJobObject(lifecycle_job.as_raw_handle(), 1);
                }
                // Output continues draining while closing emits its final frame.
                drop(console);
                let _ = writer.join();
                let _ = reader.join();
                #[cfg(debug_assertions)]
                note_conpty_cleanup_fixture(lifecycle_pid);
                #[cfg(test)]
                {
                    let mut held = lifecycle_control
                        .cleanup_hold
                        .0
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    while *held {
                        held = lifecycle_control
                            .cleanup_hold
                            .1
                            .wait(held)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                }
                unsafe {
                    SetEvent(lifecycle_control.completed.as_raw_handle());
                }
            })?;
        checkpoint(SpawnCheckpoint::LifecycleStarted, info.dwProcessId)?;
        if let Some(activation) = activation {
            let activation_control = control.clone();
            let activation_job = job.clone();
            thread::Builder::new()
                .name("runyte-conpty-activate".into())
                .spawn(move || {
                    let active = (activation.wait)();
                    drop(activation.lifetime);
                    if !active
                        || unsafe { ResumeThread(primary_thread.as_raw_handle()) } == u32::MAX
                    {
                        activation_control.stop();
                        unsafe {
                            TerminateJobObject(activation_job.as_raw_handle(), 1);
                        }
                    }
                })?;
        } else if unsafe { ResumeThread(primary_thread.as_raw_handle()) } == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if let Some(retention) = &setup_retention {
            retention
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
        }
        guard.armed = false;
        Ok(Self {
            process,
            job,
            control,
            pid: info.dwProcessId,
        })
    }
    pub fn write(&self, bytes: Vec<u8>) -> bool {
        if bytes.len() > MAX_INPUT_BYTES {
            return false;
        }
        bytes.is_empty()
            || self
                .control
                .enqueue(Input {
                    bytes,
                    delivery: None,
                })
                .is_ok()
    }
    pub(super) fn enqueue_proposal(
        &self,
        text: &super::proposal::Text,
        bracketed: bool,
    ) -> io::Result<super::proposal::Delivery> {
        let mut bytes = Vec::new();
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_str().as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        let delivery = super::proposal::Delivery::queued();
        self.control.enqueue(Input {
            bytes,
            delivery: Some(delivery.clone()),
        })?;
        Ok(delivery)
    }
    pub fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        if self.control.stopped() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Terminal has exited",
            ));
        }
        self.control
            .dimensions
            .store(columns as u32 | ((rows as u32) << 16), Ordering::Release);
        if unsafe { SetEvent(self.control.resize.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn terminate(&mut self) {
        self.control.stop();
        unsafe {
            TerminateJobObject(self.job.as_raw_handle(), 1);
        }
    }
    pub(super) fn signal_unpublished(&self) {
        self.control.stop();
        unsafe {
            TerminateJobObject(self.job.as_raw_handle(), 1);
        }
    }
    pub(super) fn terminate_unpublished(&mut self) {
        self.signal_unpublished();
        unsafe {
            WaitForSingleObject(self.control.completed.as_raw_handle(), INFINITE);
        }
    }
    #[cfg(test)]
    pub(super) fn unpublished_completed(&self) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.control.completed.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected ConPTY cleanup wait result",
            )),
        }
    }
    pub fn finished(&mut self) -> Option<Option<i32>> {
        (unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } == WAIT_OBJECT_0)
            .then(|| exit_code(&self.process))
    }
}

#[cfg(debug_assertions)]
fn note_conpty_cleanup_fixture(pid: u32) {
    let Some(directory) = std::env::var_os("RUNYTE_TEST_CONPTY_CLEANUP_DIR") else {
        return;
    };
    let marker = PathBuf::from(directory).join(format!("conpty-cleanup-{pid}"));
    let pending = marker.with_extension("pending");
    if std::fs::write(&pending, b"complete").is_ok() {
        let _ = std::fs::rename(pending, marker);
    }
}
fn exit_code(process: &OwnedHandle) -> Option<i32> {
    let mut code = 0;
    (unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } != 0).then_some(code as i32)
}
pub fn default_shell() -> std::ffi::OsString {
    std::env::var_os("COMSPEC")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cmd.exe".into())
}

#[cfg(test)]
#[path = "tests/pty_windows.rs"]
mod tests;
