use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::sync::OnceLock;

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;

#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(
        job: *mut c_void,
        info_class: i32,
        info: *const c_void,
        length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

fn overlay_job() -> Option<*mut c_void> {
    static JOB: OnceLock<usize> = OnceLock::new();
    let handle = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info = JobObjectExtendedLimitInformation::default();
        info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        );
        if configured == 0 {
            let _ = CloseHandle(job);
            return 0;
        }
        job as usize
    });
    if handle == 0 { None } else { Some(handle as *mut c_void) }
}

/// Assigns a freshly spawned overlay to the daemon's job. The handle is
/// deliberately kept open for the process lifetime; closing it (process
/// exit) is what triggers the kill-on-close contract.
pub fn assign_overlay_child(child: &Child) {
    if let Some(job) = overlay_job() {
        unsafe {
            let _ = AssignProcessToJobObject(job, child.as_raw_handle());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_job_exists_and_is_shared() {
        let job = overlay_job().expect("kill-on-close job object must be created");
        assert!(!job.is_null());
        assert_eq!(overlay_job(), Some(job), "one job object per daemon process");
    }
}
