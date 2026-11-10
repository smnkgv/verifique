//! Linux: POSIX for processes, a systemd user unit for the service, notify-send for notifications.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

pub use super::posix::{
    alive, ask_to_stop, canonical, is_within, end_orphaned_tree, end_quietly_on_closed_output, free_bytes, make_executable,
    on_stop_request, open_log, program_file_names, session_environment, shell, shell_program, stop_process, write_private, Tree,
};
use super::posix::run;
use super::ServiceSpec;
use crate::paths::user_home as home_dir;
use crate::paths::{self, NAME};

pub const PANEL_PACKAGE: &str = "";

pub fn notify(title: &str, message: &str) {
    let _ = Command::new("notify-send").arg(title).arg(message).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn unit_name() -> String {
    paths::own_home_key().map_or(format!("{NAME}.service"), |key| format!("{NAME}-{key}.service"))
}

fn unit_path() -> PathBuf {
    home_dir().join(".config/systemd/user").join(unit_name())
}

fn unit(program: &Path, args: &[String], env_file: &Path, log: &Path) -> String {
    format!(
        "[Unit]\nDescription={NAME} verification queue\n\n[Service]\nEnvironmentFile={env}\nExecStart={program} {args}\nRestart=always\nStandardOutput=append:{log}\nStandardError=append:{log}\n\n[Install]\nWantedBy=default.target\n",
        env = env_file.display(),
        program = program.display(),
        args = args.join(" "),
        log = log.display(),
    )
}

/// systemd reads the env file itself.
pub fn install_service(spec: &ServiceSpec) -> Result<()> {
    let ServiceSpec { program, args, env_file, log, env: _ } = spec;
    let path = unit_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, unit(program, args, env_file, log))?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", &unit_name()])
}

pub fn restart_service() -> Result<()> {
    run("systemctl", &["--user", "restart", &unit_name()])
}

pub fn uninstall_service() -> Result<()> {
    let _ = run("systemctl", &["--user", "disable", "--now", &unit_name()]);
    let _ = fs::remove_file(unit_path());
    Ok(())
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable).filter(|value| !value.is_empty()).map_or_else(|| home_dir().join(fallback), PathBuf::from)
}

fn panel_program() -> PathBuf {
    home_dir().join(".local/bin/verifique-panel")
}

fn panel_menu_entry() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("applications/verifique-panel.desktop")
}

fn panel_login_entry() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart/verifique-panel.desktop")
}

fn desktop_entry(program: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={NAME}\nComment=The {NAME} queue in the tray\nExec=\"{}\"\nTerminal=false\nCategories=Development;\n",
        program.display()
    )
}

/// In ~/.local/bin, with an entry in the app menu and, to open at login, one in autostart.
pub fn install_panel(package: &Path, start: bool) -> Result<PathBuf> {
    let program = panel_program();
    for file in [&program, &panel_menu_entry(), &panel_login_entry()] {
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    // Removed first, so a panel that's running keeps its copy and the new one takes its place.
    let _ = fs::remove_file(&program);
    fs::copy(package, &program)?;
    make_executable(&program)?;
    fs::write(panel_menu_entry(), desktop_entry(&program))?;
    if start {
        fs::write(panel_login_entry(), desktop_entry(&program))?;
        use std::os::unix::process::CommandExt;
        Command::new(&program).process_group(0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    }
    Ok(program)
}

pub fn remove_panel() -> Result<()> {
    for file in [panel_login_entry(), panel_menu_entry(), panel_program()] {
        if file.exists() {
            fs::remove_file(&file)?;
        }
    }
    Ok(())
}
