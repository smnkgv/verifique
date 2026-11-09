//! What macOS and Linux share, because POSIX gives it to both.

use std::collections::BTreeMap;
use std::env;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::Pid;

const TICK: Duration = Duration::from_millis(100);
const MARKER: &str = "__VERIFIQUE_ENV__";

fn sh() -> PathBuf {
    env::var_os("VERIFIQUE_SH").map_or_else(|| PathBuf::from("sh"), PathBuf::from)
}

pub fn shell(script: &str) -> Result<Command> {
    let mut command = Command::new(sh());
    command.arg("-c").arg(script);
    Ok(command)
}

pub fn shell_program() -> Result<PathBuf> {
    let sh = sh();
    if sh.is_absolute() {
        return Ok(sh);
    }
    let path = env::var_os("PATH").unwrap_or_default();
    super::find_program(&sh.to_string_lossy(), &path).context("no sh on PATH")
}

/// A process group: the first process leads it, and its children join unless they leave on purpose.
pub struct Tree {
    child: Child,
}

fn signal_group(group: Pid, signal: i32) {
    // SAFETY: killpg takes plain integers and touches no memory. A group that has already gone
    // reports ESRCH, which is the outcome we want.
    unsafe {
        libc::killpg(group as i32, signal);
    }
}

impl Tree {
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        command.process_group(0);
        Ok(Self { child: command.spawn()? })
    }

    pub fn id(&self) -> Pid {
        self.child.id()
    }

    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    pub fn stop(&mut self, grace: Duration) {
        signal_group(self.id(), libc::SIGTERM);
        let deadline = Instant::now() + grace;
        while self.exited().is_none() && Instant::now() < deadline {
            sleep(TICK);
        }
        // A stack's children can outlive its leader, so the group always gets the final word.
        self.kill();
        let _ = self.child.wait();
    }

    pub fn kill(&mut self) {
        signal_group(self.id(), libc::SIGKILL);
    }
}

pub fn end_orphaned_tree(id: Pid) {
    signal_group(id, libc::SIGKILL);
}

pub fn alive(pid: Pid) -> bool {
    // SAFETY: signal 0 only asks whether the process exists, and touches no memory.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn stop_process(pid: Pid, grace: Duration) {
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        if !alive(pid) {
            return;
        }
        // SAFETY: as above.
        unsafe {
            libc::kill(pid as i32, signal);
        }
        let deadline = Instant::now() + grace;
        while alive(pid) && Instant::now() < deadline {
            sleep(TICK);
        }
    }
}

pub fn ask_to_stop(pid: Pid) -> Result<()> {
    // SAFETY: as above.
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        bail!("can't signal pid {pid}: {}", io::Error::last_os_error());
    }
    Ok(())
}

pub fn on_stop_request(flag: Arc<AtomicBool>) -> Result<()> {
    signal_hook::flag::register(libc::SIGTERM, Arc::clone(&flag))?;
    signal_hook::flag::register(libc::SIGINT, flag)?;
    Ok(())
}

pub fn end_quietly_on_closed_output() {
    // SAFETY: restores the default disposition, before any other thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
}

pub fn free_bytes(path: &Path) -> Result<u64> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: statvfs fills the zeroed struct and reads a valid C string.
    let (code, stats) = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(path.as_ptr(), &mut stats), stats)
    };
    if code != 0 {
        bail!("can't read free disk: {}", io::Error::last_os_error());
    }
    Ok(stats.f_bavail as u64 * stats.f_frsize as u64)
}

pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(contents)?;
    Ok(())
}

pub fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

pub fn program_file_names(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

/// What the user's interactive login shell really has, which is where people set PATH up.
pub fn session_environment(names: &[&str]) -> Result<BTreeMap<String, String>> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let script = format!(
        "printf '{MARKER}'; for name in {}; do eval \"value=\\${{$name:-}}\"; printf '%s=%s\\n' \"$name\" \"$value\"; done",
        names.join(" ")
    );
    let out = Command::new(&shell).args(["-ilc", &script]).output().context("reading the shell's environment")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (_, values) = text.rsplit_once(MARKER).context("the shell printed no environment")?;
    Ok(values
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect())
}

pub fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program).args(args).output().with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
