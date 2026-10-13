//! Documented libproc inspection; never signals a PID read from a sidecar.
//! ABI reference: apple-oss-distributions/xnu, bsd/sys/proc_info.h.
//! Struct layout declarations describe the public SDK ABI, not its implementation.
use crate::contract::{Error, Result};
use std::{ffi::c_void, mem::{size_of, size_of_val, zeroed}};
#[repr(C)]
struct TaskInfo { sizes_times: [u64; 6], counters: [i32; 12] }
#[repr(C)]
struct BsdInfo { ids: [u32; 12], comm: [u8; 16], name: [u8; 32], fields: [u32; 6], started_sec: u64, started_usec: u64 }
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listpids(kind: u32, value: u32, buffer: *mut c_void, size: i32) -> i32;
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
}
pub fn group_rss(group: i32) -> Result<u64> {
    let mut pids=[0i32; 256];
    let size=unsafe { proc_listpids(2, group as u32, pids.as_mut_ptr() as _, size_of_val(&pids) as _) };
    if size <= 0 { return Err(Error::new("RESOURCE_OBSERVATION", "Cannot enumerate the owned macOS process group", 4)); }
    if size as usize >= size_of_val(&pids) { return Err(Error::new("PROCESS_LIMIT", "Backend exceeds 255 tracked processes", 4)); }
    let mut rss=0;
    for pid in pids.into_iter().take(size as usize / size_of::<i32>()).filter(|p| *p > 0) {
        let mut info: TaskInfo=unsafe { zeroed() };
        unsafe { *libc::__error()=0; }
        let read=unsafe { proc_pidinfo(pid, 4, 0, &mut info as *mut _ as _, size_of::<TaskInfo>() as _) };
        if valid_member_sample(read, std::io::Error::last_os_error().raw_os_error())? {
            rss += info.sizes_times[1] / 1024;
        }
    }
    Ok(rss)
}
pub fn process_identity(pid: u32) -> Option<String> {
    let mut info: BsdInfo=unsafe { zeroed() };
    if unsafe { proc_pidinfo(pid as i32, 3, 0, &mut info as *mut _ as _, size_of::<BsdInfo>() as _) } != size_of::<BsdInfo>() as i32 { return None; }
    if info.ids[1] == 5 { return None; } // SZOMB
    Some(format!("{}:{}", info.started_sec, info.started_usec))
}

fn valid_member_sample(size: i32, errno: Option<i32>) -> Result<bool> {
    if size == size_of::<TaskInfo>() as i32 { return Ok(true); }
    if size == 0 && errno == Some(libc::ESRCH) { return Ok(false); }
    Err(Error::new("RESOURCE_OBSERVATION", "Cannot inspect owned macOS process RSS", 4))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn libproc_sampling_fails_closed_except_exited_members() {
        assert!(valid_member_sample(size_of::<TaskInfo>() as i32,None).unwrap());
        assert!(!valid_member_sample(0,Some(libc::ESRCH)).unwrap());
        for (size, errno) in [(0,Some(libc::EPERM)),(0,None),(-1,None),(16,None)] {
            assert_eq!(valid_member_sample(size,errno).unwrap_err().code,"RESOURCE_OBSERVATION");
        }
    }
}
