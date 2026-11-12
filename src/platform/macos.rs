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
    on_stop_request, open_log, program_file_names, session_environment, shell, shell_program, spawn_detached, stop_process,
    write_private, Tree, RELEASE_ARCHIVE,
};
use super::posix::{from_homebrew, homebrew_upgrade, run, script_update, HOMEBREW_UPDATE};
use super::ServiceSpec;
use crate::paths::user_home as home_dir;
use crate::paths;

const DEFAULT_LABEL: &str = "dev.verifique.server";
const PANEL_LABEL: &str = "dev.verifique.panel";

pub const PANEL_PACKAGE: &str = ".zip";
const BOOTSTRAP_ATTEMPTS: u32 = 20;
const BOOTSTRAP_RETRY: Duration = Duration::from_millis(500);

pub fn notify(title: &str, message: &str) {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", quote(message), quote(title));
    let _ = Command::new("osascript").arg("-e").arg(script).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Homebrew's Cellar, or else install.sh, the only other way verifique reaches a Mac.
pub fn update_command(program: &Path) -> Option<String> {
    if from_homebrew(program) {
        return Some(HOMEBREW_UPDATE.to_string());
    }
    Some(script_update(program))
}

/// Homebrew's own upgrade, or else the release swapped in as install.sh would.
pub fn upgrade(program: &Path, version: &str, _interactive: bool) -> Result<Option<PathBuf>> {
    if from_homebrew(program) {
        return homebrew_upgrade(program).map(Some);
    }
    crate::release::install_over(program, version).map(Some)
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

fn panel_app() -> PathBuf {
    home_dir().join("Applications/verifique.app")
}

pub fn panel_installed() -> bool {
    panel_app().exists()
}

fn panel_plist() -> PathBuf {
    home_dir().join("Library/LaunchAgents").join(format!("{PANEL_LABEL}.plist"))
}

fn panel_agent(app: &Path) -> String {
    let program = xml(&app.join("Contents/MacOS/verifique").to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{PANEL_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
    )
}

/// In ~/Applications, where Spotlight and Launchpad find it, opened at login by a LaunchAgent.
pub fn install_panel(package: &Path, start: bool) -> Result<PathBuf> {
    let app = panel_app();
    let folder = app.parent().map(Path::to_path_buf).unwrap_or_default();
    fs::create_dir_all(&folder)?;
    // A panel this installed is stopped before its app is replaced; one started by hand keeps going.
    if panel_plist().exists() {
        let _ = run("launchctl", &["bootout", &format!("{}/{PANEL_LABEL}", domain())]);
    }
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    run("ditto", &["-x", "-k", &package.to_string_lossy(), &folder.to_string_lossy()])?;
    if !app.is_dir() {
        anyhow::bail!("the panel's package held no verifique.app");
    }
    if start {
        let plist = panel_plist();
        if let Some(parent) = plist.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&plist, panel_agent(&app))?;
        bootstrap(&domain(), &plist)?;
    }
    Ok(app)
}

pub fn remove_panel() -> Result<()> {
    if panel_plist().exists() {
        let _ = run("launchctl", &["bootout", &format!("{}/{PANEL_LABEL}", domain())]);
        fs::remove_file(panel_plist())?;
    }
    let app = panel_app();
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    Ok(())
}
