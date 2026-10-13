use crate::contract::{Error, Limits, Result};
use serde::Serialize;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::{
    ffi::CString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn on_signal(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}
pub fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
    }
}

pub fn disk_free(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::new("INVALID_PATH", "NUL in path", 2))?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let s = unsafe { s.assume_init() };
    Ok(s.f_bavail.saturating_mul(s.f_frsize))
}
pub fn executable(name: &str) -> Result<PathBuf> {
    for d in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if !d.is_absolute() {
            continue;
        }
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
}
struct ChildGuard {
    child: Child,
    active: bool,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
    }
}

fn group_rss(group: i32) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir("/proc") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let stat = fs::read_to_string(e.path().join("stat")).unwrap_or_default();
            if let Some((_, tail)) = stat.rsplit_once(") ") {
                let fields: Vec<&str> = tail.split_whitespace().collect();
                if fields.get(2).and_then(|x| x.parse::<i32>().ok()) != Some(group) {
                    continue;
                }
                let status = fs::read_to_string(e.path().join("status")).unwrap_or_default();
                for l in status.lines() {
                    if l.starts_with("VmRSS:") {
                        total += l
                            .split_whitespace()
                            .nth(1)
                            .and_then(|x| x.parse::<u64>().ok())
                            .unwrap_or(0);
                    }
                }
            }
        }
    }
    total
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
        .env("OPENBLAS_NUM_THREADS", limits.threads.to_string())
        .process_group(0);
    // Kill the child if its supervisor dies, including SIGKILL. Linux MVP only.
    let supervisor = std::process::id() as libc::pid_t;
    unsafe {
        cmd.pre_exec(move || {
            let no_core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::setrlimit(libc::RLIMIT_CORE, &no_core) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != supervisor {
                return Err(std::io::Error::other(
                    "Supervisor exited before child startup",
                ));
            }
            Ok(())
        });
    }
    let mut owned = ChildGuard {
        child: cmd.spawn()?,
        active: true,
    };
    let child = &mut owned.child;
    let pid = child.id() as i32;
    let started = Instant::now();
    let mut peak = 0;
    let mut failure: Option<Error> = None;
    let status = loop {
        if exited_unreaped(pid)? {
            // Keep the leader unreaped until its whole owned process group is
            // stopped: this prevents the PID/group ID being reused before kill.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            break child.wait()?;
        }
        peak = peak.max(group_rss(pid));
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
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && !exited_unreaped(pid)? {
                thread::sleep(Duration::from_millis(20));
            }
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            break child.wait()?;
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
                status.signal(),
                log.display()
            ),
            3,
        ));
    }
    Ok(Receipt {
        elapsed_ms: started.elapsed().as_millis(),
        sampled_peak_rss_kib: peak,
        memory_enforcement: "sampled_process_group_watchdog_not_hard_limit",
        exit_code: status.code(),
    })
}

pub fn pid_namespace() -> Option<String> {
    fs::read_link("/proc/self/ns/pid")
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn process_identity(pid: u32) -> Option<String> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

pub fn runtime_fingerprint(program: &Path) -> Result<serde_json::Value> {
    let mut libraries = serde_json::Map::new();
    let ldd = executable("ldd")?;
    let listing = capture(&ldd, &[program.to_string_lossy().into_owned()])?;
    for line in listing.lines() {
        for word in line.split_whitespace() {
            if word.starts_with('/') {
                let p = fs::canonicalize(word)?;
                if p.is_file() {
                    libraries.insert(
                        p.to_string_lossy().into_owned(),
                        serde_json::json!(crate::contract::hash_file(&p)?),
                    );
                }
            }
        }
    }
    // OIDN loads its CPU device library dynamically; ldd alone misses it in
    // Blender's official portable bundle. Hash these known runtime components.
    if program.file_name().is_some_and(|n| n == "blender") {
        let bundled = program.parent().unwrap().join("lib");
        if bundled.is_dir() {
            for entry in fs::read_dir(bundled)? {
                let p = entry?.path();
                let name = p.file_name().unwrap().to_string_lossy();
                if name.starts_with("libOpenImageDenoise")
                    && (name.contains("device_cpu") || !name.contains("device_"))
                {
                    let real = fs::canonicalize(p)?;
                    if real.is_file() && !libraries.contains_key(real.to_string_lossy().as_ref()) {
                        libraries.insert(
                            real.to_string_lossy().into_owned(),
                            serde_json::json!(crate::contract::hash_file(&real)?),
                        );
                    }
                }
            }
        }
    }
    if libraries.is_empty() {
        return Err(Error::new(
            "RUNTIME_IDENTITY",
            "Cannot fingerprint native backend dependencies",
            3,
        ));
    }
    let version_flag = if program.file_name().is_some_and(|n| n == "blender") {
        "--version"
    } else {
        "-version"
    };
    let version =
        capture(program, &[version_flag.into()]).unwrap_or_else(|_| "not_reported".into());
    Ok(
        serde_json::json!({"version":version.lines().take(3).collect::<Vec<_>>().join("\n"),"linked_libraries":libraries,
        "dynamic_loader_environment_sha256":crate::contract::hash_bytes(format!("{}\0{}",std::env::var("LD_LIBRARY_PATH").unwrap_or_default(),std::env::var("LD_PRELOAD").unwrap_or_default()).as_bytes())}),
    )
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
    if result.is_ok() { let _ = fs::remove_file(&log); }
    check_cancel(cancel)?;
    result?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "dcc-process-test-{label}-{}-{}",
            std::process::id(),
            crate::contract::now_ms()
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn kills_owned_child_on_timeout() {
        let dir = directory("timeout");
        let mut limits = Limits::inspection();
        limits.timeout_secs = 1;
        let start = Instant::now();
        let err = run(
            Path::new("/bin/sleep"),
            &["30".into()],
            &limits,
            &dir.join("log"),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, "TIMEOUT");
        assert!(start.elapsed() < Duration::from_secs(5));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cancellation_sentinel_stops_owned_child() {
        let dir = directory("cancel");
        fs::write(dir.join("cancel"), "fixture").unwrap();
        let err = run(
            Path::new("/bin/sleep"),
            &["30".into()],
            &Limits::inspection(),
            &dir.join("log"),
            None,
            Some(&dir.join("cancel")),
        )
        .unwrap_err();
        assert_eq!(err.code, "CANCELLED");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn leader_exit_does_not_leave_running_grandchild() {
        let dir = directory("grandchild");
        let pidfile = dir.join("grandchild.pid");
        let args = vec![
            "-c".into(),
            "sleep 30 & echo $! > \"$1\"; exit 0".into(),
            "fixture".into(),
            pidfile.to_string_lossy().into_owned(),
        ];
        run(
            Path::new("/bin/sh"),
            &args,
            &Limits::inspection(),
            &dir.join("log"),
            None,
            None,
        )
        .unwrap();
        let pid: u32 = fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let state = fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|s| {
                    s.rsplit_once(") ")
                        .and_then(|(_, tail)| tail.split_whitespace().next())
                        .map(str::to_owned)
                });
            if state.is_none() || state.as_deref() == Some("Z") {
                break;
            }
            assert!(Instant::now() < deadline, "Grandchild remains live: {pid}");
            thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir_all(dir).unwrap();
    }
}

pub fn memory_enforcement() -> &'static str { "sampled_process_group_watchdog_not_hard_limit" }
pub fn cache_reuse_supported() -> bool { true }
