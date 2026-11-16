//! A cancellable helper owns its process tree, including package-manager launchers.
use std::{
    io,
    process::{Child, Command},
};

#[cfg(not(windows))]
pub struct Group;
#[cfg(not(windows))]
pub fn spawn(command: &mut Command) -> io::Result<(Child, Group)> {
    command.spawn().map(|child| (child, Group))
}
#[cfg(not(windows))]
impl Group {
    pub fn terminate(&self, child: &mut Child) {
        let _ = child.kill();
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{
                OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub struct Group(Handle);
    impl Group {
        fn new() -> io::Result<Self> {
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let group = Self(Handle(job));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(group)
        }
        pub fn terminate(&self, child: &mut Child) {
            // Killing only the launcher leaves its encoder and inherited pipe handles alive.
            unsafe {
                TerminateJobObject(self.0 .0, 1);
            }
            let _ = child.kill();
        }
    }

    fn resume_initial_thread(pid: u32) -> io::Result<()> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Handle(snapshot);
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                // The process is still suspended and has not created any worker threads.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = Handle(thread);
                if unsafe { ResumeThread(thread.0) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            found = unsafe { Thread32Next(snapshot.0, &mut entry) };
        }
        Err(io::Error::other(
            "Der Startthread von FFmpeg wurde nicht gefunden.",
        ))
    }

    pub fn spawn(command: &mut Command) -> io::Result<(Child, Group)> {
        let group = Group::new()?;
        // Assign before executing any user code, so even a fast launcher cannot escape the job.
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let setup = if unsafe {
            AssignProcessToJobObject(group.0 .0, child.as_raw_handle() as HANDLE)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            resume_initial_thread(child.id())
        };
        if let Err(error) = setup {
            group.terminate(&mut child);
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, group))
    }
}
#[cfg(windows)]
pub use platform::spawn;
