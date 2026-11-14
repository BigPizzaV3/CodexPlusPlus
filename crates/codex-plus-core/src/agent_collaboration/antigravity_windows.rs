//! 只绑定本次启动的 CLI；不按进程名寻找或停止现有桌面实例。

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::System::JobObjects::{
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
};

pub(super) struct OwnedJob(OwnedHandle);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
    fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, size: u32)
    -> i32;
}

impl OwnedJob {
    pub(super) fn attach(child: &tokio::process::Child) -> std::io::Result<Self> {
        let pid = child
            .id()
            .ok_or_else(|| std::io::Error::other("CLI 已退出"))?;
        // SAFETY: null attributes/name create a private unnamed job.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a new owned handle.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(job) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // Also clean up if the host process crashes and cannot execute Drop.
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                9,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // PROCESS_SET_QUOTA | PROCESS_TERMINATE: the rights required for job assignment.
        let process = unsafe { OpenProcess(0x0100 | 0x0001, 0, pid) };
        if process.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: OpenProcess returned a new owned handle for this child PID.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process.as_raw_handle()) } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    pub(super) fn terminate(&self) {
        // SAFETY: this job contains only our CLI and descendants, never a desktop process.
        unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }
}

impl Drop for OwnedJob {
    fn drop(&mut self) {
        self.terminate();
    }
}
