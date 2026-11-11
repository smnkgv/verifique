//! Linux: POSIX for processes, a systemd user unit for the service, notify-send for notifications.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

pub use super::posix::{
    alive, ask_to_stop, canonical, is_within, end_orphaned_tree, end_quietly_on_closed_output, free_bytes, make_executable,
    on_stop_request, open_log, program_file_names, session_environment, shell, shell_program, spawn_detached, stop_process,
    write_private, Tree, RELEASE_ARCHIVE,
};
use super::posix::{from_homebrew, homebrew_upgrade, run, script_update, HOMEBREW_UPDATE};
use super::ServiceSpec;
use crate::paths::user_home as home_dir;
use crate::paths::{self, NAME};

pub const PANEL_PACKAGE: &str = "";

pub fn notify(title: &str, message: &str) {
    let _ = Command::new("notify-send").arg(title).arg(message).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Homebrew's Cellar, then the records of the package manager that put it in /usr/bin, or else
/// install.sh. Nothing says how a copy in /usr/bin that no package records got there.
pub fn update_command(program: &Path) -> Option<String> {
    if from_homebrew(program) {
        return Some(HOMEBREW_UPDATE.to_string());
    }
    if program.parent() == Some(Path::new("/usr/bin")) {
        return package(Path::new("/")).map(|package| package.command());
    }
    Some(script_update(program))
}

/// Homebrew's own upgrade, the system package manager's as root, or else the release swapped in as
/// install.sh would. In a terminal sudo asks for the password; from the panel, pkexec asks in the
/// desktop's own password window.
pub fn upgrade(program: &Path, version: &str, interactive: bool) -> Result<Option<PathBuf>> {
    if from_homebrew(program) {
        return homebrew_upgrade(program).map(Some);
    }
    if program.parent() == Some(Path::new("/usr/bin")) {
        let Some(package) = package(Path::new("/")) else { return Ok(None) };
        let asker = if interactive { "sudo" } else { "pkexec" };
        let status = Command::new(asker)
            .args(["sh", "-c", package.as_root()])
            .status()
            .with_context(|| format!("running {asker}, which asks for the password to update system packages"))?;
        if !status.success() {
            anyhow::bail!("the system's package manager didn't update {NAME}: run `{}` to see why", package.command());
        }
        return Ok(Some(program.to_path_buf()));
    }
    crate::release::install_over(program, version).map(Some)
}

/// The package managers whose repositories carry verifique.
#[derive(Debug, PartialEq)]
enum Package {
    Apt,
    Dnf,
    Pacman,
}

impl Package {
    /// What a person types.
    fn command(&self) -> String {
        match self {
            Package::Apt => format!("sudo apt update && sudo apt install --only-upgrade {NAME}"),
            Package::Dnf => format!("sudo dnf upgrade --refresh {NAME}"),
            Package::Pacman => format!("sudo pacman -Syu {NAME}"),
        }
    }

    /// What runs as root, asking nothing more. verifique is built statically and needs only git, so
    /// pacman updates it alone rather than the whole system behind a click.
    fn as_root(&self) -> &'static str {
        match self {
            Package::Apt => "apt-get update && apt-get install -y --only-upgrade verifique",
            Package::Dnf => "dnf upgrade -y --refresh verifique",
            Package::Pacman => "pacman -Sy --noconfirm verifique",
        }
    }
}

/// apt, dnf and pacman all put verifique at /usr/bin/verifique, so each one's records, under
/// `root`, say which did. The rpm database isn't plain files, so dnf is told by the repository file
/// that the user guide adds.
fn package(root: &Path) -> Option<Package> {
    let dpkg = root.join("var/lib/dpkg/info");
    if ["", ":amd64", ":arm64"].iter().any(|arch| dpkg.join(format!("{NAME}{arch}.list")).is_file()) {
        return Some(Package::Apt);
    }
    let pacman = fs::read_dir(root.join("var/lib/pacman/local")).into_iter().flatten().flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.strip_prefix(&format!("{NAME}-")).is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
    });
    if pacman {
        return Some(Package::Pacman);
    }
    if root.join(format!("etc/yum.repos.d/{NAME}.repo")).is_file() {
        return Some(Package::Dnf);
    }
    None
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

pub fn panel_installed() -> bool {
    panel_program().exists()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn records(files: &[&str], dirs: &[&str]) -> PathBuf {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-records-{unique}-{}", files.len() + dirs.len()));
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        for dir in dirs {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        root
    }

    #[test]
    fn each_package_manager_is_told_by_its_own_records() {
        let apt = records(&["var/lib/dpkg/info/verifique.list"], &[]);
        assert_eq!(package(&apt), Some(Package::Apt));
        let pacman = records(&[], &["var/lib/pacman/local/verifique-0.3.3-1", "var/lib/pacman/local/verifiquex-1.0-1"]);
        assert_eq!(package(&pacman), Some(Package::Pacman));
        let dnf = records(&["etc/yum.repos.d/verifique.repo"], &[]);
        assert_eq!(package(&dnf), Some(Package::Dnf));
        let none = records(&["var/lib/dpkg/info/other.list"], &["var/lib/pacman/local/verifique-panel"]);
        assert_eq!(package(&none), None);
        assert_eq!(Package::Apt.command(), "sudo apt update && sudo apt install --only-upgrade verifique");
        for root in [apt, pacman, dnf, none] {
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn homebrew_and_the_install_script_are_told_by_the_folder() {
        assert_eq!(
            update_command(Path::new("/home/linuxbrew/.linuxbrew/Cellar/verifique/0.3.3/bin/verifique")).as_deref(),
            Some(HOMEBREW_UPDATE)
        );
        assert!(update_command(Path::new("/opt/tools/verifique")).unwrap().contains("install.sh"));
    }
}
