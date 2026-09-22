// SPDX-License-Identifier: MPL-2.0

//! Native process ownership shared by terminals and background services.
//! Service and provisional startup jobs forbid breakaway and close descendants.
//! Only an authenticated detached startup releases its own private job, allowing
//! later explicit breakaway; inherited external jobs retain their own policies.

pub(crate) mod overlapped;

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    process::{Command, ExitStatus},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::SECURITY_ATTRIBUTES,
    System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
};

pub(crate) fn new_job() -> io::Result<OwnedHandle> {
    let job = owned(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // Each successful native creation transfers one handle to this owner.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

/// CRT argument framing, with every argument quoted (also suppresses MSYS globbing).
pub(crate) fn command_line<'a>(
    program: &'a OsStr,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> io::Result<Vec<u16>> {
    let mut result = Vec::new();
    for value in std::iter::once(program).chain(args) {
        if !result.is_empty() {
            result.push(32);
        }
        result.push(34);
        let mut slashes = 0;
        for unit in value.encode_wide() {
            if unit == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "command contains NUL",
                ));
            }
            if unit == 92 {
                slashes += 1;
                continue;
            }
            result.extend(std::iter::repeat_n(
                92,
                if unit == 34 { slashes * 2 + 1 } else { slashes },
            ));
            slashes = 0;
            result.push(unit);
        }
        result.extend(std::iter::repeat_n(92, slashes * 2));
        result.push(34);
    }
    if result.len() >= 32767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command line is too long",
        ));
    }
    result.push(0);
    Ok(result)
}

struct Attributes(Vec<usize>);
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut bytes);
        }
        let mut storage = vec![0; bytes.div_ceil(size_of::<usize>())];
        if unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &mut bytes)
        } == 0
        {
            // An uninitialized list cannot be passed to DeleteProcThreadAttributeList.
            return Err(io::Error::last_os_error());
        }
        Ok(Self(storage))
    }
    /// Values are borrowed until CreateProcess completes and this list is dropped.
    unsafe fn handles(&mut self, attribute: usize, values: &[HANDLE]) -> io::Result<()> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.0.as_mut_ptr().cast(),
                0,
                attribute,
                values.as_ptr().cast(),
                size_of_val(values),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}

/// Synchronous parent pipes are consumed by the existing bounded Git I/O workers.
pub(crate) struct Child {
    pub(crate) stdin: Option<File>,
    pub(crate) stdout: Option<File>,
    pub(crate) stderr: Option<File>,
    process: OwnedHandle,
    job: OwnedHandle,
}
impl Child {
    #[cfg(test)]
    pub(crate) fn fixture_process_handle(&self) -> HANDLE {
        self.process.as_raw_handle()
    }

    #[cfg(test)]
    pub(crate) fn fixture_job_handle(&self) -> HANDLE {
        self.job.as_raw_handle()
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                // The process handle and job remain ours even after leader exit.
                self.kill()?;
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub(crate) fn kill(&mut self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 5000) } {
            WAIT_OBJECT_0 => self
                .try_wait()?
                .ok_or_else(|| io::Error::other("process exit was not observable")),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process cleanup timed out",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn pipe(parent_reads: bool) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 0,
    };
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let (read, write) = (owned(read)?, owned(write)?);
    let (parent, child) = if parent_reads {
        (read, write)
    } else {
        (write, read)
    };
    Ok((parent, child))
}

/// An isolated handle table. Its primary thread is never resumed, so no
/// application code or loader initialization executes. The job owns it from
/// creation, including if the editor exits between setup steps.
struct InheritanceParent(OwnedHandle);
impl InheritanceParent {
    fn new(job: &OwnedHandle, directory: Option<&[u16]>) -> io::Result<Self> {
        Self::with_creation_flags(job, directory, 0)
    }

    fn with_creation_flags(
        job: &OwnedHandle,
        directory: Option<&[u16]>,
        extra_flags: u32,
    ) -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        let application = crate::windows_fs::wide(&executable)?;
        let mut line = command_line(executable.as_os_str(), [])?;
        let jobs = [job.as_raw_handle()];
        let mut attributes = Attributes::new()?;
        unsafe {
            attributes.handles(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &jobs)?;
        }
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of_val(&startup) as u32;
        startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
        let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW | extra_flags,
                ptr::null(),
                directory.map_or(ptr::null(), |value| value.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return Err(if extra_flags & CREATE_BREAKAWAY_FROM_JOB != 0 {
                io::Error::new(
                    error.kind(),
                    anyhow::Error::new(error)
                        .context("CreateProcessW could not create the detached inheritance parent"),
                )
            } else {
                error
            });
        }
        let parent = Self(owned(info.hProcess)?);
        drop(owned(info.hThread)?);
        Ok(parent)
    }

    fn inherit(&self, handle: &OwnedHandle) -> io::Result<HANDLE> {
        let mut remote = ptr::null_mut();
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle.as_raw_handle(),
                self.0.as_raw_handle(),
                &mut remote,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // This value belongs to the surrogate's handle table, not ours. Its
        // termination closes every duplicate, including on partial failure.
        Ok(remote)
    }
}
impl Drop for InheritanceParent {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.0.as_raw_handle(), 0);
        }
        // The suspended surrogate remains job-owned until termination. No
        // Windows reaping is required, and launch must not wait on its exit.
    }
}

/// Spawn an inherited-environment command with captured stdout/stderr and
/// optional stdin. Only the Git builder calls this; its Stdio and environment
/// policy is reflected here explicitly, since stable Rust cannot supply native
/// process attributes to Command::spawn.
pub(crate) fn spawn(command: &Command, pipe_stdin: bool) -> io::Result<Child> {
    spawn_inner(command, pipe_stdin, |_, _| {})
}

/// Launch with caller-owned native stdin/stdout/stderr endpoints. Their peer
/// handles stay with the caller (for example an overlapped async transport).
/// The endpoints are inherited through the same isolated parent and job as Git.
pub(crate) fn spawn_with_stdio(command: &Command, stdio: [OwnedHandle; 3]) -> io::Result<Child> {
    spawn_with_stdio_inner(command, true, Some(stdio), |_, _| {})
}

fn spawn_inner(
    command: &Command,
    pipe_stdin: bool,
    before_create: impl FnOnce(HANDLE, HANDLE),
) -> io::Result<Child> {
    spawn_with_stdio_inner(command, pipe_stdin, None, before_create)
}

fn spawn_with_stdio_inner(
    command: &Command,
    pipe_stdin: bool,
    stdio: Option<[OwnedHandle; 3]>,
    before_create: impl FnOnce(HANDLE, HANDLE),
) -> io::Result<Child> {
    spawn_with_policy(command, pipe_stdin, stdio, before_create, false)
}

fn spawn_with_policy(
    command: &Command,
    pipe_stdin: bool,
    stdio: Option<[OwnedHandle; 3]>,
    before_create: impl FnOnce(HANDLE, HANDLE),
    detached: bool,
) -> io::Result<Child> {
    let program = std::path::Path::new(command.get_program());
    if !program.is_absolute()
        || !program
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "background program must be an absolute native executable",
        ));
    }
    let application = crate::windows_fs::wide(program)?;
    let directory = command
        .get_current_dir()
        .map(crate::windows_fs::ordinary_working_directory)
        .transpose()?;
    let directory = directory
        .as_deref()
        .map(crate::windows_fs::wide)
        .transpose()?;
    let mut line = command_line(command.get_program(), command.get_args())?;
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (name, value) in command.get_envs() {
        environment.retain(|(existing, _)| !same_name(existing, name));
        if let Some(value) = value {
            environment.push((name.into(), value.into()));
        }
    }
    environment.sort_by(|a, b| {
        crate::windows_fs::compare_names(
            &a.0.encode_wide().collect::<Vec<_>>(),
            &b.0.encode_wide().collect::<Vec<_>>(),
        )
    });
    let mut block = Vec::new();
    for (name, value) in environment {
        if name
            .encode_wide()
            .chain(value.encode_wide())
            .any(|c| c == 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "environment contains NUL",
            ));
        }
        block.extend(name.encode_wide());
        block.push(61);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    let job = new_job()?;
    let (stdin, stdout, stderr, stdio) = match stdio {
        Some(stdio) => (None, None, None, stdio),
        None => {
            let (stdin, child_stdin) = pipe(false)?;
            let (stdout, child_stdout) = pipe(true)?;
            let (stderr, child_stderr) = pipe(true)?;
            // A closed writer provides immediate EOF when no input was requested.
            (
                pipe_stdin.then(|| File::from(stdin)),
                Some(File::from(stdout)),
                Some(File::from(stderr)),
                [child_stdin, child_stdout, child_stderr],
            )
        }
    };
    let parent = if detached {
        // Native policy decides whether inherited jobs permit breakaway. Do not
        // retry without this flag: that would silently undo detached ownership.
        InheritanceParent::with_creation_flags(
            &job,
            directory.as_deref(),
            CREATE_BREAKAWAY_FROM_JOB,
        )?
    } else {
        InheritanceParent::new(&job, directory.as_deref())?
    };
    let handles = [
        parent.inherit(&stdio[0])?,
        parent.inherit(&stdio[1])?,
        parent.inherit(&stdio[2])?,
    ];
    before_create(stdio[1].as_raw_handle(), parent.0.as_raw_handle());
    let parents = [parent.0.as_raw_handle()];
    let mut attributes = Attributes::new()?;
    // Inheritable handles exist ONLY in the isolated parent's table. This
    // remains safe even when an embedding host uses ordinary Command::spawn.
    // PARENT_PROCESS also inherits the parent's job before execution. The
    // explicit handle list restricts the child to these three endpoints.
    unsafe {
        attributes.handles(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &handles)?;
        attributes.handles(PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize, &parents)?;
    }
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of_val(&startup) as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            block.as_ptr().cast(),
            directory
                .as_ref()
                .map_or(ptr::null(), |value| value.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let process = owned(info.hProcess)?;
    drop(owned(info.hThread)?);
    Ok(Child {
        stdin,
        stdout,
        stderr,
        process,
        job,
    })
}

fn same_name(left: &OsStr, right: &OsStr) -> bool {
    crate::windows_fs::compare_names(
        &left.encode_wide().collect::<Vec<_>>(),
        &right.encode_wide().collect::<Vec<_>>(),
    )
    .is_eq()
}

/// Owns a provisional detached host and all its descendants until the caller
/// authenticates readiness. This is deliberately separate from Git/LSP Child:
/// observing leader exit does not release or kill the provisional job.
pub(crate) struct StartupChild {
    child: Child,
    armed: bool,
}

impl StartupChild {
    pub(crate) fn spawn(command: &Command) -> io::Result<Self> {
        // No launch-lifetime pipe reader remains after successful handoff.
        // Log configuration is passed explicitly in argv by the lifecycle.
        let input: OwnedHandle = std::fs::OpenOptions::new().read(true).open("NUL")?.into();
        let output: OwnedHandle = std::fs::OpenOptions::new().write(true).open("NUL")?.into();
        let error: OwnedHandle = std::fs::OpenOptions::new().write(true).open("NUL")?.into();
        Ok(Self {
            child: spawn_with_policy(
                command,
                false,
                Some([input, output, error]),
                |_, _| {},
                true,
            )?,
            armed: true,
        })
    }

    pub(crate) fn handle(&self) -> HANDLE {
        self.child.process.as_raw_handle()
    }

    #[cfg(test)]
    pub(crate) fn job_handle(&self) -> HANDLE {
        self.child.job.as_raw_handle()
    }

    pub(crate) fn exit_status(&self) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(crate) fn contains_process(&self, process: HANDLE) -> io::Result<bool> {
        let mut value = 0;
        if unsafe { IsProcessInJob(process, self.child.job.as_raw_handle(), &mut value) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(value != 0)
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.child.job.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn job_is_empty(&self) -> io::Result<bool> {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                self.child.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&info) as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses == 0)
    }

    /// Caller must have authenticated the actual created host immediately
    /// before this synchronous commit. There must be no await before returning
    /// successful readiness after the private job's kill flag is cleared.
    pub(crate) fn release(&mut self) -> io::Result<()> {
        if self.exit_status()?.is_some() || !self.contains_process(self.handle())? {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "startup host exited before ownership handoff",
            ));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                self.child.job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        limits.BasicLimitInformation.LimitFlags &= !JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // Only this authenticated startup job becomes breakaway-capable. The
        // released host may later start another independently detached host;
        // terminal/background jobs retain their existing no-breakaway policy.
        limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        if unsafe {
            SetInformationJobObject(
                self.child.job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        self.armed = false;
        Ok(())
    }
}

impl Drop for StartupChild {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.terminate();
        }
        // Kill-on-close remains armed on all failure/cancellation paths. Native
        // process teardown completes asynchronously; no worker is detached.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;
    use std::{
        io::{Read, Write},
        path::Path,
        time::{Duration, Instant},
    };

    const FIXTURE: &str = "windows_process::tests::child_fixture";

    fn command(root: &Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                FIXTURE,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("RUNYTE_PROCESS_FIXTURE", mode);
        command
    }

    #[test]
    #[ignore = "compiled subprocess fixture invoked by native process tests"]
    fn child_fixture() {
        let Ok(mode) = std::env::var("RUNYTE_PROCESS_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "echo" => {
                let expected: Vec<String> =
                    serde_json::from_str(&std::env::var("RUNYTE_EXPECTED_ARGS").unwrap()).unwrap();
                assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), expected);
                assert_eq!(std::env::var("RUNYTE_CASE_CHECK").unwrap(), "updated");
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).unwrap();
                print!("NATIVE_INPUT:{input}:END");
                eprint!("NATIVE_STDERR");
            }
            "exit" => {
                std::process::exit(17);
            }
            "probe-inheritance" => {
                let handle: usize = std::env::var("RUNYTE_PROBE_HANDLE")
                    .unwrap()
                    .parse()
                    .unwrap();
                let address = std::env::var("RUNYTE_PROBE_ADDRESS").unwrap();
                let mut control = std::net::TcpStream::connect(address).unwrap();
                control
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                control.write_all(b"ready").unwrap();
                let mut release = [0];
                control.read_exact(&mut release).unwrap();
                let marker = b"UNRELATED_CHILD_RETAINED_THIS_PIPE";
                let mut written = 0;
                let success = unsafe {
                    windows_sys::Win32::Storage::FileSystem::WriteFile(
                        handle as HANDLE,
                        marker.as_ptr(),
                        marker.len() as u32,
                        &mut written,
                        ptr::null_mut(),
                    )
                };
                control
                    .write_all(&[u8::from(success != 0 && written == marker.len() as u32)])
                    .unwrap();
                control.read_exact(&mut release).unwrap();
            }
            "handle" => {
                let value: usize = std::env::var("RUNYTE_SENTINEL_HANDLE")
                    .unwrap()
                    .parse()
                    .unwrap();
                // Only an inherited handle to the parent's event can signal it.
                unsafe {
                    SetEvent(value as HANDLE);
                }
            }
            "tree-exit" | "tree-hold" => {
                use std::os::windows::process::CommandExt;
                let root = std::env::current_dir().unwrap();
                // This fixture intentionally leaves the descendant alive so
                // the parent test can verify cleanup after its leader exits.
                #[allow(clippy::zombie_processes)]
                let child = command(&root, "hold")
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(root.join("child.pid"), child.id().to_string()).unwrap();
                while !root.join("release").is_file() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if mode == "tree-hold" {
                    loop {
                        std::thread::park();
                    }
                }
                print!("TREE_FINISHED");
            }
            "hold" => loop {
                std::thread::park();
            },
            _ => panic!("unknown fixture mode"),
        }
    }

    fn wait(child: &mut Child) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "native fixture did not exit"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    struct FixtureChild(std::process::Child);
    impl Drop for FixtureChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            unsafe {
                WaitForSingleObject(self.0.as_raw_handle(), 5000);
            }
        }
    }

    #[test]
    fn mixed_standard_spawn_cannot_retain_the_native_output_pipe() {
        use std::os::windows::process::CommandExt;
        let root = TestRuntimeRoot::new("mixed-spawn-inheritance").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut sibling = None;
        let mut control = None;
        let mut child = spawn_inner(&command(root.path(), "exit"), false, |handle, _| {
            let mut probe = command(root.path(), "probe-inheritance");
            probe
                .current_dir(crate::windows_fs::ordinary_working_directory(root.path()).unwrap())
                .env("RUNYTE_PROBE_HANDLE", (handle as usize).to_string())
                .env(
                    "RUNYTE_PROBE_ADDRESS",
                    listener.local_addr().unwrap().to_string(),
                )
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            sibling = Some(FixtureChild(probe.spawn().unwrap()));
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "probe did not acknowledge startup"
                        );
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("probe connection: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut ready = [0; 5];
            stream.read_exact(&mut ready).unwrap();
            assert_eq!(&ready, b"ready");
            control = Some(stream);
        })
        .unwrap();
        assert_eq!(wait(&mut child).code(), Some(17));
        // The intended child and its job have ended; only the unrelated child
        // can now write this marker into the exact pipe owned by this launcher.
        let mut control = control.unwrap();
        control.write_all(b"g").unwrap();
        let mut inherited = [0];
        control.read_exact(&mut inherited).unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut bytes = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut available = 0;
            let ready = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    stdout.as_raw_handle(),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut available,
                    ptr::null_mut(),
                )
            };
            if ready == 0 {
                assert_eq!(io::Error::last_os_error().raw_os_error(), Some(109));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "unrelated live child retained output pipe"
            );
            let mut chunk = vec![0; available as usize];
            stdout.read_exact(&mut chunk).unwrap();
            bytes.extend(chunk);
            std::thread::yield_now();
        }
        assert!(
            sibling.as_mut().unwrap().0.try_wait().unwrap().is_none(),
            "the unrelated child must remain alive when EOF is observed"
        );
        control.write_all(b"q").unwrap();
        drop(sibling);
        let output = String::from_utf8(bytes).unwrap();
        assert!(
            !output.contains("UNRELATED_CHILD_RETAINED_THIS_PIPE"),
            "{output}"
        );
    }

    #[test]
    fn native_arguments_environment_and_pipes_round_trip() {
        let root = TestRuntimeRoot::new("native-process-arguments").unwrap();
        let mut command = command(root.path(), "echo");
        // Additional libtest filters use OR matching, so the exact fixture
        // still runs and verifies these otherwise uninterpreted argument values.
        command.args([
            "",
            "café 😀",
            r"C:\folder with spaces\",
            "a\"b",
            "a&b",
            "*.txt",
        ]);
        command
            .env("runyte_case_check", "stale")
            .env("RUNYTE_CASE_CHECK", "updated");
        let expected: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        command.env(
            "RUNYTE_EXPECTED_ARGS",
            serde_json::to_string(&expected).unwrap(),
        );
        let mut child = spawn(&command, true).unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let output = std::thread::spawn(move || {
            let mut text = String::new();
            stdout.read_to_string(&mut text).unwrap();
            text
        });
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            stderr.read_to_string(&mut text).unwrap();
            text
        });
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all("café 😀\r\nsecond-line\n".as_bytes())
            .unwrap();
        drop(stdin);
        let status = wait(&mut child);
        let output = output.join().unwrap();
        let errors = errors.join().unwrap();
        assert!(status.success(), "{status}: {output} {errors}");
        assert!(
            output.contains("NATIVE_INPUT:café 😀\r\nsecond-line\n:END"),
            "{output}"
        );
        assert!(errors.contains("NATIVE_STDERR"));
    }

    #[test]
    fn native_exit_status_and_null_stdin_are_observed() {
        let root = TestRuntimeRoot::new("native-process-status").unwrap();
        let mut child = spawn(&command(root.path(), "exit"), false).unwrap();
        assert!(child.stdin.is_none());
        assert_eq!(wait(&mut child).code(), Some(17));
    }

    #[test]
    fn leader_exit_cancellation_and_drop_stop_owned_descendants() {
        for action in ["exit", "cancel", "drop"] {
            let root = TestRuntimeRoot::new("native-process-tree").unwrap();
            let mut child = spawn(
                &command(
                    root.path(),
                    if action == "exit" {
                        "tree-exit"
                    } else {
                        "tree-hold"
                    },
                ),
                false,
            )
            .unwrap();
            let started = Instant::now();
            let pid = loop {
                if let Ok(pid) = std::fs::read_to_string(root.join("child.pid"))
                    && let Ok(pid) = pid.parse::<u32>()
                {
                    break pid;
                }
                assert!(started.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(5));
            };
            let descendant = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }).unwrap();
            std::fs::write(root.join("release"), "continue").unwrap();
            match action {
                "exit" => {
                    assert!(wait(&mut child).success());
                }
                "cancel" => {
                    child.kill().unwrap();
                    child.wait().unwrap();
                }
                "drop" => {}
                _ => unreachable!(),
            }
            drop(child);
            assert_eq!(
                unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
                WAIT_OBJECT_0,
                "{action} left a descendant"
            );
        }
    }

    #[test]
    fn native_spawn_refuses_relative_programs_and_invalid_framing() {
        let root = TestRuntimeRoot::new("native-process-refusal").unwrap();
        assert!(spawn(&Command::new("git"), false).is_err());
        let mut command = command(root.path(), "exit");
        command.arg("bad\0argument");
        assert!(spawn(&command, false).is_err());
        assert!(command_line(OsStr::new("tool.exe"), [OsStr::new(&"x".repeat(32767))]).is_err());
        assert!(spawn(&Command::new(root.join("missing.exe")), false).is_err());
    }

    #[test]
    fn failed_launch_and_unwind_close_the_isolated_parent() {
        let root = TestRuntimeRoot::new("native-parent-cleanup").unwrap();
        for unwind in [false, true] {
            let mut observed = None;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                spawn_inner(
                    &Command::new(root.join("missing.exe")),
                    false,
                    |_, parent| {
                        observed = Some(
                            owned(unsafe {
                                OpenProcess(PROCESS_SYNCHRONIZE, 0, GetProcessId(parent))
                            })
                            .unwrap(),
                        );
                        assert!(!unwind, "injected setup unwind");
                    },
                )
            }));
            assert!(result.is_err() || result.unwrap().is_err());
            assert_eq!(
                unsafe { WaitForSingleObject(observed.unwrap().as_raw_handle(), 5000) },
                WAIT_OBJECT_0
            );
        }
        // Closing the job is also sufficient if normal parent cleanup is
        // interrupted by editor termination before the Git child exists.
        let job = new_job().unwrap();
        let parent = InheritanceParent::new(&job, None).unwrap();
        drop(job);
        assert_eq!(
            unsafe { WaitForSingleObject(parent.0.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn unrelated_inheritable_handle_is_excluded_from_child() {
        let root = TestRuntimeRoot::new("native-process-inheritance").unwrap();
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let event = owned(unsafe { CreateEventW(&security, 1, 0, ptr::null()) }).unwrap();
        let mut command = command(root.path(), "handle");
        command.env(
            "RUNYTE_SENTINEL_HANDLE",
            (event.as_raw_handle() as usize).to_string(),
        );
        let mut child = spawn(&command, false).unwrap();
        assert!(wait(&mut child).success());
        assert_eq!(
            unsafe { WaitForSingleObject(event.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
    }

    #[test]
    fn cancellation_releases_blocked_pipe_workers() {
        let root = TestRuntimeRoot::new("native-process-blocked-pipes").unwrap();
        let mut child = spawn(&command(root.path(), "hold"), true).unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (started, starting) = std::sync::mpsc::channel();
        let (finished, finishing) = std::sync::mpsc::channel();
        let writer_finished = finished.clone();
        let writer = std::thread::spawn(move || {
            let input = vec![b'x'; 16 * 1024 * 1024];
            started.send(()).unwrap();
            let result = stdin.write_all(&input);
            writer_finished.send(()).unwrap();
            result
        });
        let reader = std::thread::spawn(move || {
            // Drain the small libtest heading, then wait for output or EOF.
            let result = io::copy(&mut stdout, &mut io::sink());
            finished.send(()).unwrap();
            result
        });
        starting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            finishing.recv_timeout(Duration::from_millis(20)).is_err(),
            "pipes should be waiting on the live child"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        finishing.recv_timeout(Duration::from_secs(5)).unwrap();
        finishing.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(writer.join().unwrap().is_err());
        assert!(reader.join().unwrap().is_ok());
    }
}
