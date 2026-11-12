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

pub fn open_log(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new().create(true).append(true).open(path)
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

pub fn is_within(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
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

/// The ending of verifique's own release archive for macOS and Linux.
pub const RELEASE_ARCHIVE: &str = ".tar.gz";

/// Homebrew keeps each formula's program in `Cellar/<formula>/<version>`, on macOS and Linux alike,
/// and links it from its `bin`, which `program` has already been followed through.
pub fn from_homebrew(program: &Path) -> bool {
    let parts: Vec<&std::ffi::OsStr> = program.components().map(|part| part.as_os_str()).collect();
    parts.windows(2).any(|pair| pair[0] == "Cellar" && pair[1] == crate::paths::NAME)
}

/// Installed by its full name, as the user guide does, so Homebrew trusts that formula alone.
pub const HOMEBREW_UPDATE: &str = "brew upgrade smnkgv/verifique/verifique";

/// Homebrew's own upgrade, run by the Homebrew `program` belongs to, which a panel started from the
/// desktop wouldn't find on its short PATH. Returns where Homebrew links the new program.
pub fn homebrew_upgrade(program: &Path) -> Result<PathBuf> {
    let prefix: PathBuf = program.components().take_while(|part| part.as_os_str() != "Cellar").collect();
    let brew = prefix.join("bin/brew");
    let status = Command::new(&brew)
        .args(["upgrade", "smnkgv/verifique/verifique"])
        .status()
        .with_context(|| format!("running {}", brew.display()))?;
    if !status.success() {
        bail!("Homebrew couldn't upgrade {}: run `{HOMEBREW_UPDATE}` to see why", crate::paths::NAME);
    }
    Ok(prefix.join("bin").join(crate::paths::NAME))
}

/// Start `command` on its own, with nothing to read or write, so it carries on after this process.
pub fn spawn_detached(mut command: Command) -> io::Result<()> {
    use std::process::Stdio;
    command.process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop)
}

/// install.sh again, into the folder this copy is in. It puts it in `~/.local/bin` unless
/// `VERIFIQUE_INSTALL_DIR` says otherwise, so only another folder needs naming.
pub fn script_update(program: &Path) -> String {
    let script = "curl -fsSL https://raw.githubusercontent.com/smnkgv/verifique/main/install.sh";
    let folder = program.parent().unwrap_or(Path::new("."));
    if folder == crate::paths::user_home().join(".local/bin") {
        format!("{script} | sh")
    } else {
        format!("{script} | VERIFIQUE_INSTALL_DIR={} sh", sh_quote(&folder.to_string_lossy()))
    }
}

/// `text` as one word to sh, quoted only when it has to be.
fn sh_quote(text: &str) -> String {
    let plain = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~+:@%".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homebrew_is_told_by_its_cellar() {
        assert!(from_homebrew(Path::new("/opt/homebrew/Cellar/verifique/0.3.3/bin/verifique")));
        assert!(from_homebrew(Path::new("/home/linuxbrew/.linuxbrew/Cellar/verifique/0.3.3/bin/verifique")));
        assert!(!from_homebrew(Path::new("/opt/homebrew/Cellar/other/1.0/bin/verifique")));
        assert!(!from_homebrew(Path::new("/Users/me/.local/bin/verifique")));
    }

    #[test]
    fn the_install_script_names_only_a_folder_of_its_own() {
        let home = crate::paths::user_home();
        let default = script_update(&home.join(".local/bin/verifique"));
        assert!(default.ends_with("install.sh | sh"), "{default}");
        assert!(script_update(Path::new("/opt/tools/verifique")).ends_with("| VERIFIQUE_INSTALL_DIR=/opt/tools sh"));
        assert!(script_update(Path::new("/opt/my tools/verifique")).ends_with("| VERIFIQUE_INSTALL_DIR='/opt/my tools' sh"));
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
