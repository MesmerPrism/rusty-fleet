//! Narrow Windows kernel adapter for exact retained-handle deletion.
//!
//! The public onboarding engine keeps `unsafe_code = "forbid"`. This crate is
//! the only exception: one audited Win32 call marks the already-open,
//! identity-validated object for deletion. It never accepts a path.

#![deny(unsafe_op_in_unsafe_fn)]

use std::io;

/// A Windows Job Object that kills every assigned process when closed.
#[cfg(windows)]
pub struct ContainedJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ContainedJob {
    /// Assign a newly spawned, suspended child to a kill-on-close job and
    /// resume its initial thread only after assignment succeeds.
    pub fn assign_and_resume(child: &std::process::Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: null attributes/name request a private unnamed job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self { handle };
        // SAFETY: this Windows information structure is valid when zeroed.
        let mut limits = unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the job and initialized, correctly sized limits are live.
        let configured = unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(std::mem::size_of_val(&limits))
                    .map_err(|_| io::Error::other("invalid job limit size"))?,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the child is live and suspended, so it cannot execute before
        // assignment to the configured job.
        let assigned = unsafe { AssignProcessToJobObject(job.handle, child.as_raw_handle() as _) };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        resume_process_threads(child.as_raw_handle() as _)?;
        Ok(job)
    }

    /// Terminate every process currently associated with the job.
    pub fn terminate(&self) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        // SAFETY: `self.handle` is a live job handle.
        if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for ContainedJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: this is the sole owned job handle. Kill-on-close is the
        // intentional containment backstop.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn resume_process_threads(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessId, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    // SAFETY: `process` is a live child process handle.
    let process_id = unsafe { GetProcessId(process) };
    if process_id == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a thread-only system snapshot takes no process-specific input.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        // SAFETY: `THREADENTRY32` is a C data structure valid when zeroed.
        let mut entry = unsafe { std::mem::zeroed::<THREADENTRY32>() };
        entry.dwSize = u32::try_from(std::mem::size_of::<THREADENTRY32>())
            .map_err(|_| io::Error::other("invalid thread entry size"))?;
        // SAFETY: `snapshot` is live and `entry` points to writable storage.
        let mut present = unsafe { Thread32First(snapshot, &mut entry) };
        while present != 0 {
            if entry.th32OwnerProcessID == process_id {
                // SAFETY: the thread id came from the live snapshot.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: `thread` is live and opened with resume access.
                let previous = unsafe { ResumeThread(thread) };
                // SAFETY: this scope owns the thread handle.
                unsafe {
                    CloseHandle(thread);
                }
                if previous == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
            }
            // SAFETY: the snapshot and writable entry remain valid.
            present = unsafe { Thread32Next(snapshot, &mut entry) };
        }
        Ok(())
    })();
    // SAFETY: this scope owns the snapshot handle.
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

/// Require a local fixed or RAM-disk root. Network, removable, optical,
/// unknown, and namespace paths are rejected before any private mutation.
#[cfg(windows)]
pub fn require_local_drive(root: &std::path::Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_RAMDISK};

    let mut wide: Vec<u16> = root.as_os_str().encode_wide().collect();
    wide.push(0);
    // SAFETY: `wide` is NUL-terminated and remains alive for this synchronous
    // read-only call.
    let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
    if matches!(drive_type, DRIVE_FIXED | DRIVE_RAMDISK) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local fixed or RAM drive required",
        ))
    }
}

/// Local-drive classification is intentionally unavailable off Windows.
#[cfg(not(windows))]
pub fn require_local_drive(_root: &std::path::Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows local drive required",
    ))
}

/// Mark the exact retained Windows file or directory handle for deletion.
///
/// The caller must open the handle with `DELETE` access, validate its
/// volume/file identity and expected content, and ensure a directory is empty.
#[cfg(windows)]
pub fn delete_retained_handle<T: std::os::windows::io::AsRawHandle>(handle: &T) -> io::Result<()> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX, FileDispositionInfoEx,
        SetFileInformationByHandle,
    };

    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    // SAFETY: `handle` owns a live Windows handle. `disposition` is fully
    // initialized, correctly sized, and remains valid for this synchronous
    // call. The API mutates only kernel state associated with that handle.
    let success = unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle() as HANDLE,
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO_EX>())
                .map_err(|_| io::Error::other("invalid disposition size"))?,
        )
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Retained-handle deletion is intentionally unavailable off Windows.
#[cfg(not(windows))]
pub fn delete_retained_handle<T>(_handle: &T) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows retained-handle deletion is required",
    ))
}

/// Return the numeric value of a retained test handle.
#[cfg(all(windows, feature = "test-hooks"))]
#[doc(hidden)]
pub fn test_raw_handle_value<T: std::os::windows::io::AsRawHandle>(handle: &T) -> usize {
    handle.as_raw_handle() as usize
}

/// Resolve the exact file identity behind a raw test handle.
#[cfg(all(windows, feature = "test-hooks"))]
#[doc(hidden)]
pub fn test_file_identity(raw: usize) -> io::Result<(u32, u32, u32)> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the API validates the untrusted numeric value and initializes
    // `information` only on success.
    let success = unsafe { GetFileInformationByHandle(raw as HANDLE, information.as_mut_ptr()) };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: success guarantees complete structure initialization.
        let information = unsafe { information.assume_init() };
        Ok((
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
        ))
    }
}
