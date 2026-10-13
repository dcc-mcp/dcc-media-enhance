use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub exit_code: i32,
}
impl Error {
    pub fn new(code: &str, message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("IO_ERROR", e.to_string(), 3)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_JSON", e.to_string(), 2)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub threads: u32,
    pub memory_mb: u64,
    pub timeout_secs: u64,
    pub max_pixels: u64,
    pub max_input_mb: u64,
    pub max_output_mb: u64,
    pub min_free_disk_mb: u64,
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if !(1..=64).contains(&self.threads)
            || !(64..=65536).contains(&self.memory_mb)
            || !(1..=86400).contains(&self.timeout_secs)
            || !(1..=268435456).contains(&self.max_pixels)
            || !(1..=1048576).contains(&self.max_input_mb)
            || !(1..=1048576).contains(&self.max_output_mb)
            || self.min_free_disk_mb > 1048576
        {
            return Err(Error::new(
                "INVALID_LIMITS",
                "Resource limits are absent, zero or outside supported bounds",
                2,
            ));
        }
        Ok(())
    }
    pub fn inspection() -> Self {
        Self {
            threads: 1,
            memory_mb: 256,
            timeout_secs: 15,
            max_pixels: 268435456,
            max_input_mb: 1048576,
            max_output_mb: 2,
            min_free_disk_mb: 0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Passes {
    pub beauty: String,
    pub albedo: String,
    pub normal: String,
    pub layer: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub schema_version: u32,
    pub operation: String,
    pub media: String,
    pub content: String,
    pub input: PathBuf,
    pub output: PathBuf,
    pub backend: String,
    pub colorspace: String,
    pub alpha: String,
    pub limits: Limits,
    #[serde(default)]
    pub passes: Option<Passes>,
    #[serde(default)]
    pub model_dir: Option<PathBuf>,
    #[serde(default)]
    pub scale: Option<u32>,
}
impl Job {
    pub fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        if self.schema_version != 1 {
            return Err(Error::new(
                "SCHEMA_VERSION",
                "Only job schema version 1 is supported",
                2,
            ));
        }
        if !["denoise", "upscale", "resize"].contains(&self.operation.as_str())
            || !["image", "video"].contains(&self.media.as_str())
            || !["render", "photo", "illustration"].contains(&self.content.as_str())
        {
            return Err(Error::new(
                "UNSUPPORTED_OPERATION",
                "Unsupported operation/media/content combination",
                2,
            ));
        }
        if self.alpha != "opaque" {
            return Err(Error::new(
                "UNSUPPORTED_ALPHA",
                "MVP supports verified opaque inputs only; transparent inputs require a future alpha-preservation backend",
                2,
            ));
        }
        if self.input.as_os_str().is_empty() || self.output.as_os_str().is_empty() {
            return Err(Error::new(
                "INVALID_PATH",
                "Input and output are required",
                2,
            ));
        }
        if let Some(p) = &self.passes {
            if p.beauty == p.albedo || p.beauty == p.normal || p.albedo == p.normal {
                return Err(Error::new(
                    "INVALID_PASSES",
                    "Beauty, albedo and normal roles must use distinct real passes",
                    2,
                ));
            }
            for n in [&p.beauty, &p.albedo, &p.normal, &p.layer] {
                if n.is_empty() || n.len() > 128 || n.chars().any(|c| c.is_control()) {
                    return Err(Error::new(
                        "INVALID_PASSES",
                        "Pass names must be nonempty, bounded and contain no control characters",
                        2,
                    ));
                }
            }
        }
        Ok(())
    }
}
pub fn load_job(path: &Path) -> Result<(Job, PathBuf)> {
    let job: Job = serde_json::from_value(read_json_bounded(path, 65536)?)?;
    job.validate()?;
    Ok((job, fs::canonicalize(path)?.parent().unwrap().to_path_buf()))
}
pub fn read_json_bounded(path: &Path, max: u64) -> Result<Value> {
    regular_file(path, max)?;
    let mut s = String::new();
    fs::File::open(path)?.take(max + 1).read_to_string(&mut s)?;
    if s.len() as u64 > max {
        return Err(Error::new(
            "INPUT_TOO_LARGE",
            "JSON exceeds maximum size",
            2,
        ));
    }
    Ok(serde_json::from_str(&s)?)
}
pub fn regular_file(path: &Path, max: u64) -> Result<()> {
    crate::platform::validate_path(path)?;
    let md = fs::symlink_metadata(path)?;
    if !md.is_file() || md.file_type().is_symlink() || md.len() > max {
        return Err(Error::new(
            "UNSAFE_INPUT",
            "Expected a bounded regular non-symlink file",
            2,
        ));
    }
    Ok(())
}
pub fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
pub fn hash_file(path: &Path) -> Result<String> {
    hash_file_with_cancel(path, None)
}
pub fn hash_file_with_cancel(path: &Path, cancel: Option<&Path>) -> Result<String> {
    crate::process::check_cancel(cancel)?;
    regular_file(path, u64::MAX)?;
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut chunks = 0u32;
    loop {
        if chunks % 64 == 0 {
            crate::process::check_cancel(cancel)?;
        }
        chunks = chunks.wrapping_add(1);
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    f.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    f.sync_all()?;
    drop(f);
    crate::platform::replace(&temp, path)?;
    sync_parent(path)
}
pub fn sync_parent(path: &Path) -> Result<()> {
    crate::platform::sync_parent(path)
}
pub fn suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_is_sha256() {
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn reject_zero_threads() {
        let mut l = Limits::inspection();
        l.threads = 0;
        assert!(l.validate().is_err());
    }
    #[test]
    fn reject_unbounded_pixels() {
        let mut l = Limits::inspection();
        l.max_pixels = u64::MAX;
        assert!(l.validate().is_err());
    }
    #[test]
    fn rejects_same_socket_for_different_render_roles() {
        let mut job: Job = serde_json::from_str(include_str!("../examples/render.json")).unwrap();
        job.passes.as_mut().unwrap().normal = "Combined".into();
        assert!(matches!(job.validate(),Err(e) if e.code=="INVALID_PASSES"));
    }
    #[test]
    fn hashing_honors_cancellation_before_publication() {
        let dir = std::env::temp_dir().join(format!(
            "dcc-hash-cancel-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("file"), "source").unwrap();
        fs::write(dir.join("cancel"), "owned-fixture").unwrap();
        assert!(
            matches!(hash_file_with_cancel(&dir.join("file"),Some(&dir.join("cancel"))),Err(e) if e.code=="CANCELLED")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
