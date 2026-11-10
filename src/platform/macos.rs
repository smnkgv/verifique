//! macOS: POSIX for processes, launchd for the service, Notification Center through osascript.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;

pub use super::posix::{
    alive, ask_to_stop, canonical, is_within, end_orphaned_tree, end_quietly_on_closed_output, free_bytes, make_executable,
    on_stop_request, open_log, program_file_names, session_environment, shell, shell_program, stop_process, write_private, Tree,
};
use super::posix::run;
use super::ServiceSpec;
use crate::paths::user_home as home_dir;
use crate::paths;

const DEFAULT_LABEL: &str = "dev.verifique.server";
const BOOTSTRAP_ATTEMPTS: u32 = 20;
const BOOTSTRAP_RETRY: Duration = Duration::from_millis(500);

pub fn notify(title: &str, message: &str) {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", quote(message), quote(title));
    let _ = Command::new("osascript").arg("-e").arg(script).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn label() -> String {
    paths::own_home_key().map_or(DEFAULT_LABEL.to_string(), |key| format!("{DEFAULT_LABEL}.{key}"))
}

fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents").join(format!("{}.plist", label()))
}

fn domain() -> String {
    // SAFETY: getuid takes no arguments and cannot fail.
    format!("gui/{}", unsafe { libc::getuid() })
}

fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn plist(program: &Path, args: &[String], environment: &BTreeMap<String, String>, log: &Path) -> String {
    let log = xml(&log.to_string_lossy());
    let program: String = std::iter::once(program.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|arg| format!("        <string>{}</string>\n", xml(&arg)))
        .collect();
    let variables: String = environment
        .iter()
        .map(|(name, value)| format!("        <key>{}</key><string>{}</string>\n", xml(name), xml(value)))
        .collect();
    let label = label();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{program}    </array>
    <key>EnvironmentVariables</key>
    <dict>
{variables}    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>{log}</string>
    <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#
    )
}

// `launchctl bootout` returns before the old service has stopped, and until it has, bootstrapping
// the same label fails with an I/O error. So a reinstall has to keep trying for a moment.
fn bootstrap(domain: &str, plist: &Path) -> Result<()> {
    let path = plist.to_string_lossy();
    let mut attempts = BOOTSTRAP_ATTEMPTS;
    loop {
        match run("launchctl", &["bootstrap", domain, &path]) {
            Err(_) if attempts > 1 => {
                attempts -= 1;
                sleep(BOOTSTRAP_RETRY);
            }
            result => return result,
        }
    }
}

/// launchd takes the variables inline, so the env file is for verifique's own `service env`.
pub fn install_service(spec: &ServiceSpec) -> Result<()> {
    let ServiceSpec { program, args, env, log, env_file: _ } = spec;
    let path = plist_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let domain = domain();
    // Reinstalling replaces the service, so a missing one is fine here.
    let _ = run("launchctl", &["bootout", &format!("{domain}/{}", label())]);
    fs::write(&path, plist(program, args, env, log))?;
    bootstrap(&domain, &path)
}

pub fn restart_service() -> Result<()> {
    run("launchctl", &["kickstart", "-k", &format!("{}/{}", domain(), label())])
}

pub fn uninstall_service() -> Result<()> {
    let _ = run("launchctl", &["bootout", &format!("{}/{}", domain(), label())]);
    let _ = fs::remove_file(plist_path());
    Ok(())
}
