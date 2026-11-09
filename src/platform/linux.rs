//! Linux: POSIX for processes, a systemd user unit for the service, notify-send for notifications.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

pub use super::posix::{
    alive, ask_to_stop, canonical, end_orphaned_tree, end_quietly_on_closed_output, free_bytes, make_executable,
    on_stop_request, program_file_names, session_environment, shell, shell_program, stop_process, write_private, Tree,
};
use super::posix::run;
use super::ServiceSpec;
use crate::paths::user_home as home_dir;
use crate::paths::{self, NAME};

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
