use crate::contract::{Error, Limits, Result};
use serde::Serialize;
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
extern "C" fn on_signal(_: i32) { INTERRUPTED.store(true, Ordering::SeqCst); }
#[cfg(windows)]
unsafe extern "system" fn on_console(event: u32) -> i32 {
    if event <= 2 { INTERRUPTED.store(true, Ordering::SeqCst); 1 } else { 0 }
}
pub fn install_signal_handlers() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
    }
    #[cfg(windows)]
    crate::windows::install_handler(on_console);
}
pub fn disk_free(path: &Path) -> Result<u64> {
    #[cfg(windows)]
    { crate::windows::disk_free(path) }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c=std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_|Error::new("INVALID_PATH", "NUL in path", 2))?;
        let mut data=std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(c.as_ptr(),data.as_mut_ptr()) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        let data=unsafe { data.assume_init() };
        Ok((data.f_bavail as u64).saturating_mul(data.f_frsize as u64))
    }
}
pub fn executable(name: &str) -> Result<PathBuf> {
    for d in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if !d.is_absolute() {
            continue;
        }
        #[cfg(windows)]
        let p = d.join(format!("{name}.exe"));
        #[cfg(not(windows))]
        let p = d.join(name);
        if p.is_file() {
            return Ok(fs::canonicalize(p)?);
        }
    }
    Err(Error::new(
        "BACKEND_MISSING",
        format!("Required executable {name} is not installed; no automatic download was attempted"),
        3,
    ))
}
#[derive(Debug, Serialize)]
pub struct Receipt {
    pub elapsed_ms: u128,
    pub sampled_peak_rss_kib: u64,
    pub memory_enforcement: &'static str,
    pub exit_code: Option<i32>,
    pub peak_job_commit_kib: Option<u64>,
}
struct ChildGuard {
    child: Child,
    active: bool,
    #[cfg(windows)]
    job: crate::windows::Job,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.active {
            self.stop();
            let _ = self.child.wait();
        }
    }
}

impl ChildGuard {
    fn stop(&self) {
        #[cfg(unix)]
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL); }
        #[cfg(windows)]
        self.job.stop();
    }
    fn rss(&self) -> Result<u64> {
        #[cfg(target_os="macos")]
        { crate::macos::group_rss(self.child.id() as i32) }
        #[cfg(windows)]
        { self.job.rss_kib() }
    }
}
pub fn check_cancel(cancel: Option<&Path>) -> Result<()> {
    if INTERRUPTED.load(Ordering::SeqCst) || cancel.is_some_and(|p| fs::symlink_metadata(p).is_ok())
    {
        return Err(Error::new(
            "CANCELLED",
            "Operation cancelled; source was not changed",
            5,
        ));
    }
    Ok(())
}
#[cfg(unix)]
fn exited_unreaped(pid: i32) -> std::io::Result<bool> {
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.si_pid() } != 0)
}

pub fn run(
    program: &Path,
    args: &[String],
    limits: &Limits,
    log: &Path,
    staging: Option<&Path>,
    cancel: Option<&Path>,
) -> Result<Receipt> {
    check_cancel(cancel)?;
    let f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(log)?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(f.try_clone()?))
        .stderr(Stdio::from(f));
    cmd.env("OMP_NUM_THREADS", limits.threads.to_string())
        .env("OPENBLAS_NUM_THREADS", limits.threads.to_string());
    #[cfg(unix)]
    {
        cmd.process_group(0);
        unsafe { cmd.pre_exec(|| {
            let limit=libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 { return Err(std::io::Error::last_os_error()); }
            Ok(())
        }); }
    }
    #[cfg(windows)]
    cmd.creation_flags(0x00000004); // CREATE_SUSPENDED: assign Job before running.
    #[cfg(windows)]
    let job=crate::windows::Job::new(limits.memory_mb)?;
    let mut child=cmd.spawn()?;
    #[cfg(windows)]
    if let Err(e)=job.attach_and_resume(&child) { let _=child.kill(); let _=child.wait(); return Err(e); }
    let mut owned=ChildGuard { child, active:true, #[cfg(windows)] job };
    #[cfg(unix)]
    let pid=owned.child.id() as i32;
    let started = Instant::now();
    let mut peak = 0;
    let mut failure: Option<Error> = None;
    let status = loop {
        #[cfg(unix)]
        if exited_unreaped(pid)? {
            owned.stop();
            break owned.child.wait()?;
        }
        #[cfg(windows)]
        if let Some(status)=owned.child.try_wait()? {
            owned.stop(); // Job handle ownership is independent of PID reuse.
            break status;
        }
        peak = peak.max(owned.rss()?);
        if check_cancel(cancel).is_err() {
            failure = Some(Error::new(
                "CANCELLED",
                "Operation cancelled; source was not changed",
                5,
            ));
        } else if started.elapsed().as_secs() >= limits.timeout_secs {
            failure = Some(Error::new(
                "TIMEOUT",
                "Backend exceeded wall-clock budget",
                4,
            ));
        } else if peak > limits.memory_mb * 1024 {
            failure = Some(Error::new(
                "MEMORY_LIMIT",
                "Backend exceeded sampled process-group RSS budget",
                4,
            ));
        } else if fs::metadata(log).map(|x| x.len()).unwrap_or(0) > 4 * 1024 * 1024 {
            failure = Some(Error::new(
                "LOG_LIMIT",
                "Backend diagnostic log exceeded 4 MiB",
                4,
            ));
        } else if let Some(s) = staging {
            if fs::metadata(s).map(|m| m.len()).unwrap_or(0) > limits.max_output_mb * 1024 * 1024 {
                failure = Some(Error::new(
                    "OUTPUT_LIMIT",
                    "Staged output exceeded byte budget",
                    4,
                ));
            } else if disk_free(s.parent().unwrap())? < limits.min_free_disk_mb * 1024 * 1024 {
                failure = Some(Error::new(
                    "DISK_LIMIT",
                    "Free disk fell below required reserve",
                    4,
                ));
            }
        }
        if failure.is_some() {
            owned.stop();
            break owned.child.wait()?;
        }
        thread::sleep(Duration::from_millis(50));
    };
    owned.active = false;
    if fs::metadata(log)?.len() > 4 * 1024 * 1024 {
        fs::OpenOptions::new()
            .write(true)
            .open(log)?
            .set_len(4 * 1024 * 1024)?;
        if failure.is_none() {
            failure = Some(Error::new(
                "LOG_LIMIT",
                "Backend log exceeded 4 MiB and was truncated",
                4,
            ));
        }
    }
    check_cancel(cancel)?;
    if let Some(e) = failure {
        return Err(e);
    }
    if !status.success() {
        return Err(Error::new(
            "BACKEND_FAILED",
            format!(
                "Backend exited code={:?}, signal={:?}; bounded diagnostics: {}",
                status.code(),
                signal(&status),
                log.display()
            ),
            3,
        ));
    }
    Ok(Receipt {
        elapsed_ms: started.elapsed().as_millis(),
        sampled_peak_rss_kib: peak,
        memory_enforcement: memory_enforcement(),
        exit_code: status.code(),
        #[cfg(windows)]
        peak_job_commit_kib: Some(owned.job.peak_commit_kib()),
        #[cfg(not(windows))]
        peak_job_commit_kib: None,
    })
}

fn signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    { status.signal() }
    #[cfg(windows)]
    { let _=status; None }
}
pub fn memory_enforcement() -> &'static str {
    if cfg!(windows) { "windows_job_commit_limit_and_sampled_rss_watchdog" }
    else { "sampled_process_group_watchdog_not_hard_limit" }
}
pub fn cache_reuse_supported() -> bool { false }
pub fn pid_namespace() -> Option<String> { Some(std::env::consts::OS.into()) }
pub fn process_identity(pid: u32) -> Option<String> {
    #[cfg(windows)]
    { crate::windows::process_identity(pid) }
    #[cfg(target_os="macos")]
    { crate::macos::process_identity(pid) }
}
pub fn runtime_fingerprint(program: &Path) -> Result<serde_json::Value> {
    #[cfg(windows)]
    let libraries=crate::windows::runtime_libraries(program)?;
    #[cfg(not(windows))]
    let libraries=serde_json::Map::<String,serde_json::Value>::new();
    let blender=program.file_stem().is_some_and(|n| n=="blender");
    let version=capture(program, &[if blender {"--version".into()} else {"-version".into()}])?;
    Ok(serde_json::json!({"version":version.lines().take(3).collect::<Vec<_>>().join("\n"),
        "linked_libraries":libraries,"cache_reuse_supported":false,
        "identity_scope":"backend binary and explicit colocated DLLs only; full loader closure not verified; cache reuse disabled",
        "os":std::env::consts::OS,"architecture":std::env::consts::ARCH}))
}

pub fn capture(program: &Path, args: &[String]) -> Result<String> {
    capture_with_cancel(program, args, None)
}

pub fn capture_with_cancel(
    program: &Path,
    args: &[String],
    cancel: Option<&Path>,
) -> Result<String> {
    let log = std::env::temp_dir().join(format!(
        "dcc-media-probe-{}-{}.log",
        std::process::id(),
        crate::contract::now_ms()
    ));
    let result = run(program, args, &Limits::inspection(), &log, None, cancel);
    let mut text = String::new();
    if result.is_ok() {
        fs::File::open(&log)?
            .take(2 * 1024 * 1024)
            .read_to_string(&mut text)?;
    }
    let _ = fs::remove_file(&log);
    check_cancel(cancel)?;
    result?;
    Ok(text)
}

