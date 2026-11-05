use std::env;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

pub const NAME: &str = "verifique";
pub const CONFIG_FILE: &str = "verifique.yaml";

pub fn home() -> PathBuf {
    match env::var_os("VERIFIQUE_HOME") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(format!(".{NAME}")),
    }
}

pub fn job_dir(id: i64) -> PathBuf {
    home().join("jobs").join(id.to_string())
}

pub fn verdict_file(tree: &str, check: &str) -> PathBuf {
    home().join("verdicts").join(format!("{tree}.{check}.json"))
}

/// Each repository has its own directory of slot worktrees, recorded so `clean` can find its way
/// back to the repository that owns them.
pub fn repo_slots(repo: &str) -> PathBuf {
    let key = hex(&Sha256::digest(repo.as_bytes()))[..12].to_string();
    home().join("worktrees").join(key)
}

pub fn slot_worktree(repo: &str, slot_index: usize) -> PathBuf {
    repo_slots(repo).join(slot_index.to_string())
}

/// Whole gigabytes free on the disk that holds the slot worktrees.
pub fn free_gb() -> Result<u64> {
    let path = CString::new(home().as_os_str().as_bytes())?;
    // SAFETY: statvfs fills the zeroed struct and reads a valid C string.
    let (code, stats) = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(path.as_ptr(), &mut stats), stats)
    };
    if code != 0 {
        bail!("can't read free disk: {}", std::io::Error::last_os_error());
    }
    Ok(stats.f_bavail as u64 * stats.f_frsize / (1024 * 1024 * 1024))
}

pub fn repo_label(repo: &str) -> String {
    Path::new(repo)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
