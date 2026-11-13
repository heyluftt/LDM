//! Child processes (yt-dlp, FFmpeg, and whatever they start) that end with LDM.
//!
//! Windows neither ends a child when its parent exits nor ends a child's own children when the
//! child is killed. A job object with `KILL_ON_JOB_CLOSE` does both: every process in the job
//! ends when the last handle to the job closes, which is when the `ProcessTree` is dropped or
//! when LDM exits or crashes. The child is created suspended and resumed only once it is in
//! the job, so nothing it starts can escape.

use std::{
    io,
    os::windows::{
        io::{AsRawHandle, RawHandle},
        process::CommandExt,
    },
    process::{Child, Command, Output, Stdio},
    ptr,
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

const CREATE_SUSPENDED: u32 = 0x0000_0004;
/// Console tools must not open a console window next to a GUI app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A running child process together with everything it starts.
pub struct ProcessTree {
    /// `None` when the job could not be set up; `terminate` then falls back to `taskkill`.
    job: Option<Job>,
    root_process_id: u32,
}

impl ProcessTree {
    /// Starts `command` without a console window, inside a new job. Keep the returned tree
    /// alive for as long as the child should run: dropping it ends the processes still in it.
    pub fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .spawn()?;
        match Self::adopt(child.id(), child.as_raw_handle()) {
            Ok(tree) => Ok((child, tree)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    /// `spawn` for a child that is awaited instead of blocking a thread. Dropping the returned
    /// child ends its process as well.
    pub fn spawn_async(
        command: &mut tokio::process::Command,
    ) -> io::Result<(tokio::process::Child, Self)> {
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .kill_on_drop(true)
            .spawn()?;
        let adopted = match (child.id(), child.raw_handle()) {
            (Some(process_id), Some(process)) => Self::adopt(process_id, process),
            _ => Err(io::Error::other(
                "the child process ended before it was started",
            )),
        };
        match adopted {
            Ok(tree) => Ok((child, tree)),
            Err(error) => {
                let _ = child.start_kill();
                Err(error)
            }
        }
    }

    /// Puts a suspended process into a new job and lets it run.
    fn adopt(root_process_id: u32, process: RawHandle) -> io::Result<Self> {
        let job = match Job::containing(process) {
            Ok(job) => Some(job),
            Err(error) => {
                tracing::warn!("child process {root_process_id} runs outside a job: {error}");
                None
            }
        };
        resume_process(root_process_id)?;
        Ok(Self {
            job,
            root_process_id,
        })
    }

    /// Runs `command` to completion inside its own job and collects what it printed.
    pub fn output(command: &mut Command) -> io::Result<Output> {
        let (child, _tree) = Self::spawn(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )?;
        child.wait_with_output()
    }

    /// Ends every process of the tree now.
    pub fn terminate(&self) {
        match &self.job {
            // SAFETY: the handle is a valid job for as long as `self` lives.
            Some(job) => unsafe {
                TerminateJobObject(job.0, 1);
            },
            None => taskkill_tree(self.root_process_id),
        }
    }
}

/// A job object handle; closing it ends the processes that are still in the job.
struct Job(HANDLE);

// SAFETY: the handle refers to a kernel object that belongs to the process, not to the thread
// that created it, and TerminateJobObject and CloseHandle may be called from any thread.
unsafe impl Send for Job {}
// SAFETY: as above; the only call made through a shared reference is TerminateJobObject.
unsafe impl Sync for Job {}

impl Job {
    fn containing(process: RawHandle) -> io::Result<Self> {
        // SAFETY: null attributes and a null name create an unnamed job with default security.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // From here on the handle is closed on every early return.
        let job = Self(handle);

        // SAFETY: the structure is plain data for which all zeroes means "no limits".
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is a valid structure of the class and size passed with it.
        let configured = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: both handles are valid; the process handle stays owned by the caller.
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Lets a process created with `CREATE_SUSPENDED` run. `std` keeps the handle of its main
/// thread to itself, so the thread is looked up in a snapshot of all threads.
fn resume_process(process_id: u32) -> io::Result<()> {
    // SAFETY: a thread snapshot takes no pointers; the result is checked before use.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut resumed = false;
    // SAFETY: the structure is plain data; `dwSize` is set as the API requires.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    // SAFETY: `snapshot` is valid and `entry` is a correctly sized THREADENTRY32.
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32OwnerProcessID == process_id {
            // SAFETY: OpenThread takes no pointers; the handle is checked and closed below.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                // SAFETY: `thread` is a valid thread handle with the resume right.
                resumed |= unsafe { ResumeThread(thread) } != u32::MAX;
                // SAFETY: the handle came from OpenThread and is closed exactly once.
                unsafe {
                    CloseHandle(thread);
                }
            }
        }
        // SAFETY: as for Thread32First.
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot handle is valid and closed exactly once.
    unsafe {
        CloseHandle(snapshot);
    }

    if resumed {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "could not resume the main thread of process {process_id}"
        )))
    }
}

/// Kills a process and its descendants by parent-child relationship. Only the fallback for a
/// child that is in no job.
fn taskkill_tree(process_id: u32) {
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    use super::*;

    /// `cmd` starting `ping` as its own child: a tree of two that would run for half a minute.
    /// Both inherit the stdout pipe, so reading it to the end returns only once both are gone.
    fn spawn_slow_tree() -> (Child, ProcessTree) {
        ProcessTree::spawn(
            Command::new("cmd")
                .args(["/C", "ping -n 30 127.0.0.1"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .expect("cmd should start")
    }

    fn wait_for_the_whole_tree(mut child: Child) -> Duration {
        let started = Instant::now();
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut output)
            .expect("stdout should close");
        child.wait().expect("child should exit");
        started.elapsed()
    }

    #[test]
    fn terminate_ends_the_child_and_what_it_started() {
        let (child, tree) = spawn_slow_tree();
        std::thread::sleep(Duration::from_millis(500));

        tree.terminate();

        let waited = wait_for_the_whole_tree(child);
        assert!(waited < Duration::from_secs(10), "{waited:?}");
    }

    #[test]
    fn dropping_the_tree_ends_the_child_and_what_it_started() {
        let (child, tree) = spawn_slow_tree();
        std::thread::sleep(Duration::from_millis(500));

        drop(tree);

        let waited = wait_for_the_whole_tree(child);
        assert!(waited < Duration::from_secs(10), "{waited:?}");
    }

    #[tokio::test]
    async fn an_awaited_child_runs_in_a_tree_of_its_own() {
        let (mut child, tree) = ProcessTree::spawn_async(
            tokio::process::Command::new("cmd")
                .args(["/C", "ping -n 30 127.0.0.1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .expect("cmd should start");
        let started = Instant::now();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(child.try_wait().unwrap().is_none(), "the child should run");

        tree.terminate();

        let status = child.wait().await.expect("child should exit");
        assert!(!status.success());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn an_awaited_child_runs_to_completion() {
        let (child, _tree) = ProcessTree::spawn_async(
            tokio::process::Command::new("cmd")
                .args(["/C", "echo hello"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .expect("cmd should start");

        let output = child.wait_with_output().await.expect("cmd should run");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn a_resumed_child_runs_to_completion_and_its_output_is_collected() {
        let output = ProcessTree::output(Command::new("cmd").args(["/C", "echo hello"]))
            .expect("cmd should run");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }
}
