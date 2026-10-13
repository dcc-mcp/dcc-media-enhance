//! Filesystem primitives whose semantics differ across supported OSes.
use crate::contract::{Error, Result};
use std::{fs, path::Path};

pub fn lock_file(path: &Path, create: bool) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Deny write/delete sharing while the handle lives; OS releases on exit.
        // OPEN_REPARSE_POINT opens the link itself, never its target.
        options.share_mode(1).custom_flags(0x00200000);
    }
    options.open(path).map_err(|e| {
        #[cfg(windows)]
        if e.raw_os_error() == Some(32) {
            return Error::new("OUTPUT_LOCKED", "Another supervisor owns this output", 3);
        }
        e.into()
    })
}
pub fn lock(file: &fs::File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::new("OUTPUT_LOCKED", "Another supervisor owns this output", 3));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(Error::new("LOCK_NOT_OWNED", "Reparse-point locks are unsupported", 3));
        }
    }
    Ok(())
}
pub fn private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    // On Windows retain the enclosing directory's ACL. Never broaden it.
    #[cfg(windows)]
    let _ = path;
    Ok(())
}
pub fn replace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::rename(source, destination)?;
    #[cfg(windows)]
    crate::windows::replace(source, destination)?;
    Ok(())
}
pub fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    // MoveFileExW(WRITE_THROUGH) handles sidecars on Windows. File data is
    // flushed separately; Windows offers no portable directory-fsync contract.
    #[cfg(windows)]
    let _ = path;
    Ok(())
}
pub fn validate_path(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::path::Component;
        for component in path.components() {
            if let Component::Normal(name) = component {
                let value = name.to_string_lossy();
                let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
                let device = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
                    || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && stem.as_bytes()[3].is_ascii_digit());
                if device || value.contains(':') || value.ends_with(['.', ' ']) {
                    return Err(Error::new("INVALID_PATH", "Windows device names, alternate streams and trailing dots/spaces are unsupported", 2));
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}
