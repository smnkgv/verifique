//! Windows Job Objects: a process and every process it starts, held so they can be ended together.
//! Each job here is set to end its processes when its last handle closes, so nothing it holds
//! outlives verifique, even when verifique itself is killed.
//!
//! Also used by the windowless service launcher, which has no access to the rest of the crate.

use std::io;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME};

pub struct Job(HANDLE);

// SAFETY: a job handle may be used and closed from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

fn os_error(error: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(error.code().0 & 0xFFFF)
}

impl Job {
    pub fn new() -> io::Result<Self> {
        // SAFETY: plain Win32 calls on a handle this function owns; the struct outlives the call.
        unsafe {
            let job = Job(CreateJobObjectW(None, None).map_err(os_error)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(os_error)?;
            Ok(job)
        }
    }

    /// Start `command` inside this job, with no console window. It starts suspended and runs only
    /// once it is in the job, so not even its first child can be born outside it.
    pub fn spawn(&self, mut command: Command) -> io::Result<Child> {
        command.creation_flags(CREATE_SUSPENDED.0 | CREATE_NO_WINDOW.0);
        let mut child = command.spawn()?;
        // SAFETY: the child's handle stays valid while `child` lives.
        let assigned = unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())) };
        if let Err(error) = assigned {
            let _ = child.kill();
            return Err(os_error(error));
        }
        resume(child.id())?;
        Ok(child)
    }

    pub fn terminate(&self) {
        // SAFETY: ends every process in a job this struct owns. An empty job is fine.
        unsafe {
            let _ = TerminateJobObject(self.0, 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this struct and closed once. Closing the last handle ends
        // whatever is still in the job.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Every thread of a suspended process. A new process has one, but nothing promises that.
fn resume(pid: u32) -> io::Result<()> {
    // SAFETY: a snapshot of threads, walked with a correctly sized entry, and handles closed after use.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(os_error)?;
        let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
        let mut more = Thread32First(snapshot, &mut entry).is_ok();
        while more {
            if entry.th32OwnerProcessID == pid {
                if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) {
                    ResumeThread(thread);
                    let _ = CloseHandle(thread);
                }
            }
            more = Thread32Next(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    Ok(())
}
