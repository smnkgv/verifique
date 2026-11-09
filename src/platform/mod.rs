//! Everything verifique needs from the operating system, as one interface.
//!
//! macOS, Linux and Windows are equals here. Each provides every item below in a module of its
//! own, `macos.rs`, `linux.rs` and `windows.rs`, and nothing outside this module knows which one it
//! runs on. macOS and Linux share what POSIX gives them both through `posix.rs`.
//!
//! | Need                         | macOS                | Linux             | Windows                        |
//! |------------------------------|----------------------|-------------------|--------------------------------|
//! | Run a command                | `/bin/sh -c`         | `/bin/sh -c`      | Git for Windows' `sh -c`       |
//! | Hold a step and its children | process group        | process group     | Job Object                     |
//! | Stop them                    | SIGTERM, wait, SIGKILL | SIGTERM, wait, SIGKILL | end the job at once     |
//! | Ask the queue to stop        | SIGTERM, SIGINT      | SIGTERM, SIGINT   | named event, Ctrl+C            |
//! | Start the queue at logon     | LaunchAgent          | systemd user unit | scheduled task                 |
//! | Tell the user                | `osascript`          | `notify-send`     | toast                          |
//! | Owner-only file              | mode 600             | mode 600          | the folder's own permissions   |
//!
//! Who holds a port, a process's name and folder, and free disk come from libraries that already
//! do this per OS, so they are answered once, here.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

#[cfg(unix)]
mod posix;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(windows)]
pub mod job_object;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
compile_error!("verifique runs on macOS, Linux and Windows");

pub type Pid = u32;

/// A command that runs `script` as POSIX sh, the language of every command in verifique.yaml.
pub fn shell(script: &str) -> Result<Command> {
    os::shell(script)
}

/// The program the shell commands run with, for `doctor` to name.
pub fn shell_program() -> Result<PathBuf> {
    os::shell_program()
}

/// A command and everything it starts, held together so that stopping it reaches them all.
pub struct Tree(os::Tree);

impl Tree {
    /// Starts `command`, with no window of its own, in a group or job of its own.
    pub fn spawn(command: Command) -> io::Result<Self> {
        os::Tree::spawn(command).map(Self)
    }

    /// What a later server needs to end this tree if this one dies before it can.
    pub fn id(&self) -> Pid {
        self.0.id()
    }

    /// The first process's exit, once it has one. Its children may still be running.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.0.exited()
    }

    /// Ask the whole tree to stop, give it `grace`, then end whatever is left.
    pub fn stop(&mut self, grace: Duration) {
        self.0.stop(grace)
    }

    /// End every process in the tree now.
    pub fn kill(&mut self) {
        self.0.kill()
    }
}

/// End a tree a server that has since died left behind. Where trees end with their server, as
/// Windows' jobs do, there is nothing left to end.
pub fn end_orphaned_tree(id: Pid) {
    os::end_orphaned_tree(id)
}

pub fn alive(pid: Pid) -> bool {
    os::alive(pid)
}

/// Stop one process that is no tree of ours, such as an app an earlier job left on a port.
pub fn stop_process(pid: Pid, grace: Duration) {
    os::stop_process(pid, grace)
}

/// Ask a running queue to finish what it holds and exit.
pub fn ask_to_stop(pid: Pid) -> Result<()> {
    os::ask_to_stop(pid)
}

/// Set `flag` when someone asks this process to stop: Ctrl+C, the service manager, `ask_to_stop`.
pub fn on_stop_request(flag: Arc<AtomicBool>) -> Result<()> {
    os::on_stop_request(flag)
}

/// End quietly when whoever reads the output stops early, as `head` and `grep -q` do.
pub fn end_quietly_on_closed_output() {
    os::end_quietly_on_closed_output()
}

/// The processes listening for TCP connections on each of `ports`, from one look at the machine.
/// The library lists every socket, so connections are left out: one still closing on a port
/// belongs to no process, and Windows reports it as pid 0.
pub fn port_holders(ports: &[u32]) -> Result<BTreeMap<u32, Vec<Pid>>> {
    let all = listeners::get_all().map_err(|error| anyhow::anyhow!("listing listening ports: {error}"))?;
    let mut holders: BTreeMap<u32, Vec<Pid>> = ports.iter().map(|port| (*port, Vec::new())).collect();
    for listener in all {
        let listening = listener.protocol == listeners::Protocol::TCP && listener.state == listeners::SocketState::Listen;
        if let (true, Some(pids)) = (listening, holders.get_mut(&u32::from(listener.socket.port()))) {
            pids.push(listener.process.pid);
        }
    }
    for pids in holders.values_mut() {
        pids.sort_unstable();
        pids.dedup();
    }
    Ok(holders)
}

/// A process's name, and the folder it runs in when this user may see it.
pub fn process_info(pid: Pid) -> (String, Option<PathBuf>) {
    let Ok(mut system) = sysinfo::System::new() else { return (String::new(), None) };
    let pid = sysinfo::Pid::from_u32(pid);
    let refresh = sysinfo::ProcessRefreshKind::nothing().with_cwd(sysinfo::UpdateKind::Always);
    system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true, refresh);
    match system.process(pid) {
        Some(process) => (process.name().to_string_lossy().into_owned(), process.cwd().map(Path::to_path_buf)),
        None => (String::new(), None),
    }
}

/// Whether `path` is `folder` or inside it, comparing names the way this OS does.
pub fn is_within(path: &Path, folder: &Path) -> bool {
    os::is_within(path, folder)
}

/// Bytes free to this user on the disk that holds `path`.
pub fn free_bytes(path: &Path) -> Result<u64> {
    os::free_bytes(path)
}

/// Write a file only its owner may read.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    os::write_private(path, contents)
}

/// Let a downloaded program run.
pub fn make_executable(path: &Path) -> Result<()> {
    os::make_executable(path)
}

/// The file names a program called `name` may have on this OS, in the order they are tried.
pub fn program_file_names(name: &str) -> Vec<String> {
    os::program_file_names(name)
}

/// The first file on `path`, a PATH-style list, that runs as `name`.
pub fn find_program(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let names = program_file_names(name);
    std::env::split_paths(path).find_map(|dir| names.iter().map(|file| dir.join(file)).find(|file| file.is_file()))
}

/// A path in the form the rest of the system writes it, so two spellings of one place compare equal.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    os::canonical(path)
}

/// Show `message` the way this OS shows a note from a program. Failure is silent: a notification
/// never decides anything, and `status` stays the source of truth.
pub fn notify(title: &str, message: &str) {
    os::notify(title, message)
}

/// What the queue's service is started with.
pub struct ServiceSpec<'a> {
    /// The verifique binary.
    pub program: &'a Path,
    /// Arguments to `program`, starting with `serve`.
    pub args: &'a [String],
    /// Variables the service starts with, recorded from the user's own session. Each service
    /// manager takes these either inline or from `env_file`, so some builds never read one of them.
    #[allow(dead_code)]
    pub env: &'a BTreeMap<String, String>,
    /// The same variables as a NAME=value file.
    #[allow(dead_code)]
    pub env_file: &'a Path,
    /// Where the service's output goes.
    pub log: &'a Path,
}

/// Values of `names` as the user's own session has them. Services start with a bare environment,
/// so without these `pnpm` or a compiler's tools are simply not found.
pub fn session_environment(names: &[&str]) -> Result<BTreeMap<String, String>> {
    os::session_environment(names)
}

/// Install the queue as a per-user service that starts at logon and restarts after a crash, and
/// start it now. Installing again replaces it.
pub fn install_service(spec: &ServiceSpec) -> Result<()> {
    os::install_service(spec)
}

pub fn restart_service() -> Result<()> {
    os::restart_service()
}

/// Stop the service and remove everything `install_service` added. A missing one is fine.
pub fn uninstall_service() -> Result<()> {
    os::uninstall_service()
}
