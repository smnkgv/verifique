use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::paths;
use crate::platform::{self, ServiceSpec};

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
    pub min_free_gb: u64,
    pub resources: Vec<String>,
}

fn env_file() -> PathBuf {
    paths::home().join("service.env")
}

/// Whether `service install` has run, and `uninstall` hasn't since.
pub fn installed() -> bool {
    env_file().is_file()
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
        "--min-free-gb".to_string(),
        options.min_free_gb.to_string(),
    ];
    for resource in &options.resources {
        args.push("--resource".to_string());
        args.push(resource.clone());
    }
    args
}

pub fn install(options: &Install) -> Result<()> {
    let mut environment = platform::session_environment(&RECORDED)?;
    if paths::own_home_key().is_some() {
        environment.insert("VERIFIQUE_HOME".to_string(), paths::home().to_string_lossy().into_owned());
    }
    fs::create_dir_all(paths::home())?;
    let recorded: String = environment.iter().map(|(name, value)| format!("{name}={value}\n")).collect();
    fs::write(env_file(), recorded)?;

    let program = env::current_exe()?;
    let args = serve_args(options);
    let log = paths::home().join("server.log");
    platform::install_service(&ServiceSpec { program: &program, args: &args, env: &environment, env_file: &env_file(), log: &log })?;
    println!("installed: {} slot(s), starting at login", options.slots);
    println!("\n{WARNING}\n");
    show_env()
}

pub fn restart() -> Result<()> {
    platform::restart_service()?;
    println!("restarted");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    platform::uninstall_service()?;
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
        match platform::find_program(tool, path.as_ref()) {
            Some(file) => println!("{tool}: {}", file.display()),
            None => println!("{tool}: not found"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_server_keeps_every_machine_setting() {
        let args = serve_args(&Install { slots: 3, first_slot: 4, min_free_gb: 9, resources: vec!["simulator=1".into()] });
        assert_eq!(args, ["serve", "--slots", "3", "--first-slot", "4", "--min-free-gb", "9", "--resource", "simulator=1"]);
    }
}
