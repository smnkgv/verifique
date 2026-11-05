use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::paths::{self, NAME};

const LABEL: &str = "dev.verifique.server";
const BOOTSTRAP_ATTEMPTS: u32 = 20;
const BOOTSTRAP_RETRY: Duration = Duration::from_millis(500);
const MARKER: &str = "__VERIFIQUE_ENV__";

/// Only what tools need to be found. A service must not inherit credentials from a shell.
const RECORDED: [&str; 12] = [
    "PATH", "HOME", "USER", "LANG", "SHELL", "JAVA_HOME", "ANDROID_HOME", "ANDROID_SDK_ROOT",
    "NVM_DIR", "PNPM_HOME", "DEVELOPER_DIR", "DOCKER_HOST",
];

const LOOKED_FOR: [&str; 6] = ["git", "node", "pnpm", "npm", "docker", "claude"];

const WARNING: &str = "Before you connect a project: checks run your app unattended, with its own .env.
Anything in that file is live. A check can send real email to real people, charge real
cards or write to any database or service it points at. Only slot ports and the databases
named in verifique.yaml are replaced. Keep .env pointed at test accounts and local services.
`verifique doctor` names any remote database left in it, but it cannot see everything.";

pub struct Install {
    pub slots: usize,
    pub first_slot: u32,
    pub resources: Vec<String>,
}

fn env_file() -> PathBuf {
    paths::home().join("service.env")
}

fn home_dir() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

/// What the user's interactive login shell really has. launchd and systemd start services with a
/// bare environment, so without this `pnpm` or Xcode's tools are simply not found.
fn capture_shell_env() -> Result<BTreeMap<String, String>> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let script = format!(
        "printf '{MARKER}'; for name in {}; do eval \"value=\\${{$name:-}}\"; printf '%s=%s\\n' \"$name\" \"$value\"; done",
        RECORDED.join(" ")
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

fn read_env() -> Result<BTreeMap<String, String>> {
    let raw = fs::read_to_string(env_file()).context("the service is not installed")?;
    Ok(raw
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect())
}

fn serve_args(options: &Install) -> Vec<String> {
    let mut args = vec![
        "serve".to_string(),
        "--slots".to_string(),
        options.slots.to_string(),
        "--first-slot".to_string(),
        options.first_slot.to_string(),
    ];
    for resource in &options.resources {
        args.push("--resource".to_string());
        args.push(resource.clone());
    }
    args
}

fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn plist(binary: &Path, args: &[String], environment: &BTreeMap<String, String>) -> String {
    let log = paths::home().join("server.log");
    let log = xml(&log.to_string_lossy());
    let program: String = std::iter::once(binary.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|arg| format!("        <string>{}</string>\n", xml(&arg)))
        .collect();
    let variables: String = environment
        .iter()
        .map(|(name, value)| format!("        <key>{}</key><string>{}</string>\n", xml(name), xml(value)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
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

fn unit_path() -> PathBuf {
    home_dir().join(".config/systemd/user").join(format!("{NAME}.service"))
}

fn unit(binary: &Path, args: &[String]) -> String {
    format!(
        "[Unit]\nDescription={NAME} verification queue\n\n[Service]\nEnvironmentFile={env}\nExecStart={binary} {args}\nRestart=always\n\n[Install]\nWantedBy=default.target\n",
        env = env_file().display(),
        binary = binary.display(),
        args = args.join(" "),
    )
}

fn user_id() -> u32 {
    // SAFETY: getuid takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program).args(args).output().with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
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

pub fn install(options: &Install) -> Result<()> {
    let environment = capture_shell_env()?;
    fs::create_dir_all(paths::home())?;
    let recorded: String = environment.iter().map(|(name, value)| format!("{name}={value}\n")).collect();
    fs::write(env_file(), recorded)?;

    let binary = env::current_exe()?;
    let args = serve_args(options);
    if cfg!(target_os = "macos") {
        let path = plist_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let domain = format!("gui/{}", user_id());
        // Reinstalling replaces the service, so a missing one is fine here.
        let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
        fs::write(&path, plist(&binary, &args, &environment))?;
        bootstrap(&domain, &path)?;
    } else {
        let path = unit_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, unit(&binary, &args))?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", &format!("{NAME}.service")])?;
    }
    println!("installed: {} slot(s), starting at login", options.slots);
    println!("\n{WARNING}\n");
    show_env()
}

pub fn restart() -> Result<()> {
    if cfg!(target_os = "macos") {
        run("launchctl", &["kickstart", "-k", &format!("gui/{}/{LABEL}", user_id())])?;
    } else {
        run("systemctl", &["--user", "restart", &format!("{NAME}.service")])?;
    }
    println!("restarted");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        let _ = run("launchctl", &["bootout", &format!("gui/{}/{LABEL}", user_id())]);
        let _ = fs::remove_file(plist_path());
    } else {
        let _ = run("systemctl", &["--user", "disable", "--now", &format!("{NAME}.service")]);
        let _ = fs::remove_file(unit_path());
    }
    let _ = fs::remove_file(env_file());
    println!("removed");
    Ok(())
}

/// What the service sees, and which tools it can find with it.
pub fn show_env() -> Result<()> {
    let environment = read_env()?;
    for (name, value) in &environment {
        println!("{name}={value}");
    }
    let path = environment.get("PATH").cloned().unwrap_or_default();
    println!();
    for tool in LOOKED_FOR {
        let found = env::split_paths(&path).map(|dir| dir.join(tool)).find(|file| file.is_file());
        match found {
            Some(file) => println!("{tool}: {}", file.display()),
            None => println!("{tool}: not found"),
        }
    }
    Ok(())
}
