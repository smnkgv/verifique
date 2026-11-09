//! Windows: Git for Windows' sh for commands, Job Objects for process trees, a scheduled task for
//! the service, toasts for notifications.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use windows::core::{BOOL, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, STILL_ACTIVE};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetExitCodeProcess, OpenEventW, OpenProcess, SetEvent, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, EVENT_MODIFY_STATE, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

use super::job_object::Job;
use super::{Pid, ServiceSpec};
use crate::paths::{self, NAME};

/// Toasts carry this id, which `install_service` registers under the user's account.
const APP_ID: &str = "verifique";
const APP_ID_KEY: &str = r"HKCU\Software\Classes\AppUserModelId\verifique";
const LAUNCHER: &str = "verifique-service.exe";
const NO_GIT_SH: &str = "verifique runs commands with the sh that comes with Git for Windows, and found none. \
    Install Git for Windows (winget install Git.Git, or https://git-scm.com/download/win), or point \
    VERIFIQUE_SH at another sh.exe";

/// Git's own `bin\sh.exe` puts Git's Unix tools on PATH before it starts sh. Found through git, not
/// PATH, because `C:\Windows\System32\bash.exe` is WSL's launcher.
fn find_sh() -> Result<PathBuf, String> {
    if let Some(sh) = env::var_os("VERIFIQUE_SH") {
        return Ok(PathBuf::from(sh));
    }
    let out = Command::new("git").arg("--exec-path").creation_flags(CREATE_NO_WINDOW.0).output();
    let exec_path = match out {
        Ok(out) if out.status.success() => PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()),
        _ => return Err(NO_GIT_SH.to_string()),
    };
    exec_path
        .ancestors()
        .flat_map(|dir| [dir.join(r"bin\sh.exe"), dir.join(r"usr\bin\sh.exe")])
        .find(|sh| sh.is_file())
        .ok_or_else(|| NO_GIT_SH.to_string())
}

/// Found once and kept. Not finding it is not kept: the user may install Git while the queue runs.
pub fn shell_program() -> Result<PathBuf> {
    static SH: OnceLock<PathBuf> = OnceLock::new();
    if let Some(sh) = SH.get() {
        return Ok(sh.clone());
    }
    let sh = find_sh().map_err(|message| anyhow!(message))?;
    Ok(SH.get_or_init(|| sh).clone())
}

pub fn shell(script: &str) -> Result<Command> {
    let mut command = Command::new(shell_program()?);
    command.arg("-c").arg(script);
    Ok(command)
}

/// A Job Object holding the first process and everything it starts.
pub struct Tree {
    job: Job,
    child: Child,
}

impl Tree {
    pub fn spawn(command: Command) -> io::Result<Self> {
        let job = Job::new()?;
        let child = job.spawn(command)?;
        Ok(Self { job, child })
    }

    pub fn id(&self) -> Pid {
        self.child.id()
    }

    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Windows has no signal a windowless program reliably answers, so the grace is not used.
    pub fn stop(&mut self, _grace: Duration) {
        self.job.terminate();
        let _ = self.child.wait();
    }

    pub fn kill(&mut self) {
        self.job.terminate();
    }
}

/// A job ends with the last handle to it, so a server that died took its trees with it.
pub fn end_orphaned_tree(_id: Pid) {}

pub fn alive(pid: Pid) -> bool {
    // SAFETY: a query-only handle, closed after use.
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(process) => {
                let mut code = 0u32;
                let running = GetExitCodeProcess(process, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
                let _ = CloseHandle(process);
                running
            }
            Err(error) => error.code() == ERROR_ACCESS_DENIED.to_hresult(),
        }
    }
}

pub fn stop_process(pid: Pid, _grace: Duration) {
    // SAFETY: a terminate-only handle, closed after use.
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(process, 1);
            let _ = CloseHandle(process);
        }
    }
}

fn stop_event_name(pid: Pid) -> HSTRING {
    HSTRING::from(format!(r"Local\{NAME}-stop-{pid}"))
}

pub fn ask_to_stop(pid: Pid) -> Result<()> {
    // SAFETY: opens an event by name, sets it and closes the handle.
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, false, &stop_event_name(pid))
            .map_err(|error| anyhow!("can't reach pid {pid}: {error}"))?;
        let set = SetEvent(event);
        let _ = CloseHandle(event);
        set.map_err(|error| anyhow!("can't ask pid {pid} to stop: {error}"))
    }
}

static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

unsafe extern "system" fn on_console_event(event: u32) -> BOOL {
    let handled = matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT);
    if let (true, Some(flag)) = (handled, STOP.get()) {
        flag.store(true, Ordering::Relaxed);
    }
    BOOL::from(handled)
}

pub fn on_stop_request(flag: Arc<AtomicBool>) -> Result<()> {
    let _ = STOP.set(Arc::clone(&flag));
    // SAFETY: registers a handler that only touches an atomic. A process without a console has
    // nothing to register with, which is fine: the event below still reaches it.
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(on_console_event), true);
    }
    // SAFETY: an event this process owns for its whole life, waited on by one thread.
    let event = unsafe { CreateEventW(None, true, false, &stop_event_name(std::process::id())) }
        .context("creating the stop event")?;
    let event = event.0 as usize;
    thread::spawn(move || {
        // SAFETY: the handle is never closed, so it stays valid for this wait.
        unsafe { WaitForSingleObject(windows::Win32::Foundation::HANDLE(event as *mut _), INFINITE) };
        flag.store(true, Ordering::Relaxed);
    });
    Ok(())
}

/// Windows has no SIGPIPE: writing to a pipe whose reader has gone fails, and printing panics. So
/// a panic that says exactly that ends the process quietly instead.
pub fn end_quietly_on_closed_output() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        // 232: the pipe is being closed. 109: the pipe has been ended.
        let closed = message.contains("os error 232") || message.contains("os error 109");
        if message.starts_with("failed printing to std") && closed {
            std::process::exit(0);
        }
        default(info)
    }));
}

pub fn free_bytes(path: &Path) -> Result<u64> {
    let mut available = 0u64;
    // SAFETY: reads a valid wide string and writes one u64 this function owns.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path), Some(&mut available), None, None) }
        .map_err(|error| anyhow!("can't read free disk: {error}"))?;
    Ok(available)
}

/// The file inherits its folder's permissions, and the user's own folder is private by default.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents)?;
    Ok(())
}

/// Windows runs any `.exe`.
pub fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn program_file_names(name: &str) -> Vec<String> {
    if Path::new(name).extension().is_some() {
        return vec![name.to_string()];
    }
    let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    extensions
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!("{name}{}", extension.to_lowercase()))
        .collect()
}

/// Windows compares names without regard to case, and may report one folder as `C:\x` and `c:\X`.
pub fn is_within(path: &Path, folder: &Path) -> bool {
    let lower = |path: &Path| -> Vec<String> {
        path.components().map(|part| part.as_os_str().to_string_lossy().to_lowercase()).collect()
    };
    let (path, folder) = (lower(path), lower(folder));
    path.len() >= folder.len() && path[..folder.len()] == folder[..]
}

/// `canonicalize` answers in the `\\?\C:\...` form, which nothing else on Windows writes.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{share}")));
    }
    match text.strip_prefix(r"\\?\") {
        Some(local) if local.as_bytes().get(1) == Some(&b':') => Ok(PathBuf::from(local)),
        _ => Ok(path),
    }
}

fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn toast(title: &str, message: &str) -> windows::core::Result<()> {
    // SAFETY: joins the process's multithreaded apartment; joining twice is harmless.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let content = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml(title),
        xml(message)
    );
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(content))?;
    let notification = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&notification)
}

pub fn notify(title: &str, message: &str) {
    let _ = toast(title, message);
}

/// The terminal `service install` runs in has the user's PATH as they use it.
pub fn session_environment(names: &[&str]) -> Result<BTreeMap<String, String>> {
    Ok(names
        .iter()
        .filter_map(|name| env::var(name).ok().filter(|value| !value.is_empty()).map(|value| (name.to_string(), value)))
        .collect())
}

fn task_name() -> String {
    paths::own_home_key().map_or(NAME.to_string(), |key| format!("{NAME}-{key}"))
}

fn task_file() -> PathBuf {
    paths::home().join("service.xml")
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program).args(args).output().with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr);
        let detail = if detail.trim().is_empty() { String::from_utf8_lossy(&out.stdout) } else { detail };
        bail!("{program} {}: {}", args.join(" "), detail.trim());
    }
    Ok(())
}

/// One argument as the C runtime reads a command line back: quoted when it holds a space, with
/// backslashes doubled only where they come before a quote.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(c);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn current_user() -> String {
    match (env::var("USERDOMAIN"), env::var("USERNAME")) {
        (Ok(domain), Ok(user)) => format!(r"{domain}\{user}"),
        (_, Ok(user)) => user,
        _ => String::new(),
    }
}

/// At logon, as the user, in their session, never stopped for running long or on battery. The
/// launcher brings the queue back whenever it ends; restarting on failure covers the launcher.
fn task(launcher: &Path, args: &[String]) -> String {
    let user = xml(&current_user());
    let command = xml(&launcher.to_string_lossy());
    let arguments = xml(&args.iter().map(|arg| quote_arg(arg)).collect::<Vec<_>>().join(" "));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{NAME} verification queue</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// Task Scheduler reads task files as UTF-16.
fn write_utf16(path: &Path, text: &str) -> Result<()> {
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(path, bytes)?;
    Ok(())
}

/// The launcher reads the env file and writes the log, since a scheduled task does neither.
pub fn install_service(spec: &ServiceSpec) -> Result<()> {
    let ServiceSpec { program, args: serve, env_file, log, env: _ } = spec;
    let launcher = program.with_file_name(LAUNCHER);
    if !launcher.is_file() {
        bail!("{} is missing, and the service starts through it. Reinstall {NAME}", launcher.display());
    }
    let mut args = vec![
        "--env-file".to_string(),
        env_file.to_string_lossy().into_owned(),
        "--log".to_string(),
        log.to_string_lossy().into_owned(),
        "--".to_string(),
        program.to_string_lossy().into_owned(),
    ];
    args.extend(serve.iter().cloned());
    let file = task_file();
    write_utf16(&file, &task(&launcher, &args))?;
    let name = task_name();
    // Reinstalling replaces the task, and whatever queue the old one was running.
    let _ = run("schtasks", &["/End", "/TN", &name]);
    run("schtasks", &["/Create", "/F", "/TN", &name, "/XML", &file.to_string_lossy()])?;
    let _ = run("reg", &["add", APP_ID_KEY, "/v", "DisplayName", "/t", "REG_SZ", "/d", NAME, "/f"]);
    run("schtasks", &["/Run", "/TN", &name])
}

pub fn restart_service() -> Result<()> {
    let name = task_name();
    let _ = run("schtasks", &["/End", "/TN", &name]);
    run("schtasks", &["/Run", "/TN", &name])
}

pub fn uninstall_service() -> Result<()> {
    let name = task_name();
    let _ = run("schtasks", &["/End", "/TN", &name]);
    let _ = run("schtasks", &["/Delete", "/F", "/TN", &name]);
    let _ = run("reg", &["delete", APP_ID_KEY, "/f"]);
    let _ = fs::remove_file(task_file());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn arguments_survive_the_command_line() {
        assert_eq!(quote_arg("serve"), "serve");
        assert_eq!(quote_arg(r"C:\Program Files\verifique"), r#""C:\Program Files\verifique""#);
        assert_eq!(quote_arg(r"C:\with space\"), r#""C:\with space\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(""), r#""""#);
    }
}
