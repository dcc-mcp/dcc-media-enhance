//! Small Windows SDK ABI surface. No shell, taskkill, registry or global PATH.
use crate::contract::{Error, Result};
use std::{ffi::c_void, fs, mem::{size_of, zeroed}, os::windows::{ffi::OsStrExt, io::AsRawHandle}, path::Path, process::Child, ptr::null_mut};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Default)]
struct BasicLimits { process_time: i64, job_time: i64, flags: u32, min_ws: usize, max_ws: usize, active: u32, affinity: usize, priority: u32, scheduling: u32 }
#[repr(C)]
#[derive(Default)]
struct ExtendedLimits { basic: BasicLimits, io: [u64; 6], process_memory: usize, job_memory: usize, peak_process: usize, peak_job: usize }
#[repr(C)]
struct ThreadEntry { size: u32, usage: u32, tid: u32, pid: u32, priority: i32, delta: i32, flags: u32 }
#[repr(C)]
struct ProcessMemory { size: u32, faults: u32, peak_ws: usize, ws: usize, peak_paged: usize, paged: usize, peak_nonpaged: usize, nonpaged: usize, pagefile: usize, peak_pagefile: usize }
#[repr(C)]
struct JobProcesses { assigned: u32, count: u32, ids: [usize; 256] }
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDiskFreeSpaceExW(path: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn MoveFileExW(source: *const u16, dest: *const u16, flags: u32) -> i32;
    fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: u32, data: *const c_void, size: u32) -> i32;
    fn QueryInformationJobObject(job: Handle, class: u32, data: *mut c_void, size: u32, returned: *mut u32) -> i32;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn TerminateJobObject(job: Handle, code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> Handle;
    fn ResumeThread(thread: Handle) -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn GetProcessTimes(process: Handle, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn K32GetProcessMemoryInfo(process: Handle, memory: *mut ProcessMemory, size: u32) -> i32;
    fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
}
fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut s: Vec<u16> = path.as_os_str().encode_wide().collect();
    if s.contains(&0) { return Err(Error::new("INVALID_PATH", "NUL in path", 2)); }
    s.push(0); Ok(s)
}
fn last() -> Error { std::io::Error::last_os_error().into() }
pub fn disk_free(path: &Path) -> Result<u64> {
    let p=wide(path)?; let mut available=0;
    if unsafe { GetDiskFreeSpaceExW(p.as_ptr(), &mut available, null_mut(), null_mut()) } == 0 { return Err(last()); }
    Ok(available)
}
pub fn replace(source: &Path, dest: &Path) -> Result<()> {
    let source=wide(source)?; let dest=wide(dest)?;
    if unsafe { MoveFileExW(source.as_ptr(), dest.as_ptr(), 1 | 8) } == 0 { return Err(last()); }
    Ok(())
}
pub fn install_handler(handler: unsafe extern "system" fn(u32)->i32) {
    unsafe { SetConsoleCtrlHandler(Some(handler), 1); }
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle { fn drop(&mut self) { if !self.0.is_null() { unsafe { CloseHandle(self.0); } } } }
pub struct Job(OwnedHandle);
impl Job {
    pub fn new(memory_mb: u64) -> Result<Self> {
        let raw=unsafe { CreateJobObjectW(null_mut(), std::ptr::null()) };
        if raw.is_null() { return Err(last()); }
        let job=Self(OwnedHandle(raw));
        let mut info=ExtendedLimits::default();
        // Kill all descendants on handle close and cap aggregate committed memory.
        info.basic.flags=0x2000 | 0x200;
        info.job_memory=(memory_mb * 1024 * 1024) as usize;
        if unsafe { SetInformationJobObject(raw, 9, &info as *const _ as _, size_of::<ExtendedLimits>() as _) } == 0 { return Err(last()); }
        Ok(job)
    }
    pub fn attach_and_resume(&self, child: &Child) -> Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.0, child.as_raw_handle() as _) } == 0 { return Err(last()); }
        // Child was created suspended: it cannot fork before job assignment.
        // Locate its only initial thread using documented Toolhelp APIs.
        let raw=unsafe { CreateToolhelp32Snapshot(4, 0) };
        if raw as isize == -1 { return Err(last()); }
        let snapshot=OwnedHandle(raw);
        let mut entry: ThreadEntry=unsafe { zeroed() }; entry.size=size_of::<ThreadEntry>() as _;
        let mut more=unsafe { Thread32First(snapshot.0, &mut entry) };
        while more != 0 {
            if entry.pid == child.id() {
                let raw=unsafe { OpenThread(2, 0, entry.tid) };
                if raw.is_null() { return Err(last()); }
                let thread=OwnedHandle(raw);
                if unsafe { ResumeThread(thread.0) } == u32::MAX { return Err(last()); }
                return Ok(());
            }
            more=unsafe { Thread32Next(snapshot.0, &mut entry) };
        }
        Err(Error::new("SUPERVISOR_SETUP", "Cannot locate suspended backend thread", 3))
    }
    pub fn stop(&self) { unsafe { TerminateJobObject(self.0.0, 1); } }
    pub fn peak_commit_kib(&self) -> u64 {
        let mut info=ExtendedLimits::default();
        if unsafe { QueryInformationJobObject(self.0.0, 9, &mut info as *mut _ as _, size_of::<ExtendedLimits>() as _, null_mut()) } == 0 { return 0; }
        (info.peak_job / 1024) as u64
    }
    pub fn rss_kib(&self) -> Result<u64> {
        let mut ids: JobProcesses=unsafe { zeroed() };
        if unsafe { QueryInformationJobObject(self.0.0, 3, &mut ids as *mut _ as _, size_of::<JobProcesses>() as _, null_mut()) } == 0 { return Err(last()); }
        if ids.assigned > ids.ids.len() as u32 { return Err(Error::new("PROCESS_LIMIT", "Backend exceeds 256 tracked processes", 4)); }
        let mut rss=0;
        for pid in &ids.ids[..ids.count.min(256) as usize] {
            let raw=unsafe { OpenProcess(0x0400 | 0x0010, 0, *pid as u32) };
            if raw.is_null() { continue; } // It may have exited between samples.
            let process=OwnedHandle(raw);
            let mut memory: ProcessMemory=unsafe { zeroed() }; memory.size=size_of::<ProcessMemory>() as _;
            if unsafe { K32GetProcessMemoryInfo(process.0, &mut memory, size_of::<ProcessMemory>() as _) } != 0 { rss += memory.ws as u64 / 1024; }
        }
        Ok(rss)
    }
}
pub fn process_identity(pid: u32) -> Option<String> {
    let raw=unsafe { OpenProcess(0x1000, 0, pid) }; if raw.is_null() { return None; }
    let process=OwnedHandle(raw); let mut status=0;
    if unsafe { GetExitCodeProcess(process.0, &mut status) } == 0 || status != 259 { return None; }
    let (mut created,mut exited,mut kernel,mut user)=(0,0,0,0);
    if unsafe { GetProcessTimes(process.0,&mut created,&mut exited,&mut kernel,&mut user) } == 0 { return None; }
    Some(created.to_string())
}
pub fn runtime_libraries(program: &Path) -> Result<serde_json::Map<String,serde_json::Value>> {
    let mut result=serde_json::Map::new();
    // Only explicitly installed colocated runtime DLLs are fingerprinted.
    // System DLLs and DLLs found elsewhere in PATH are outside cache coverage;
    // the portable runner disables cache reuse rather than claiming identity.
    for entry in fs::read_dir(program.parent().unwrap())? {
        let p=entry?.path();
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) && p.is_file() {
            let p=fs::canonicalize(p)?;
            result.insert(p.to_string_lossy().into_owned(), serde_json::json!(crate::contract::hash_file(&p)?));
        }
    }
    Ok(result)
}
