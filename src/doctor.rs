use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self as std_process, Command};
use std::sync::atomic::AtomicBool;
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::config::{Agent, Config, Stack};
use crate::git;
use crate::instance::{copy_env_files, Instance};
use crate::paths;
use crate::process::{self, Ended, Env, Options};
use crate::server::{self, KEY_FIRST_SLOT, KEY_PAUSED_BY};
use crate::services::remote_database_host;
use crate::store::Store;

const DEFAULT_FIRST_SLOT: u32 = 4;
const GONE_WITHIN: Duration = Duration::from_secs(10);

struct Pause<'a>(&'a Store);

impl Drop for Pause<'_> {
    fn drop(&mut self) {
        let _ = self.0.unset(KEY_PAUSED_BY);
    }
}

struct Project<'a> {
    config: &'a Config,
    repo: &'a str,
    commit: &'a str,
    first_slot: u32,
}

/// Settings a check would use as they stand, and could write real data through.
fn warn_about_remote_databases(stack: &Stack, repo: &Path) {
    let overridden: BTreeSet<&String> = stack.overridden().collect();
    for file in stack.env_files() {
        let Ok(raw) = fs::read_to_string(repo.join(&file)) else { continue };
        for line in raw.lines() {
            let line = line.trim().trim_start_matches("export ");
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().to_string();
            if line.starts_with('#') || overridden.contains(&key) {
                continue;
            }
            if let Some(host) = remote_database_host(value.trim().trim_matches(['"', '\''])) {
                println!("  warning: {file} points {key} at a remote database ({host}), and nothing here overrides it");
            }
        }
    }
}

/// An agent that cannot sign in fails only after a whole stack has been built, so find out first.
fn check_agents(config: &Config) -> Result<()> {
    let uses_claude = config.checks.values().any(|check| matches!(check.agent, Some(Agent::Preset(_))));
    if !uses_claude {
        return Ok(());
    }
    let out = Command::new("claude").args(["auth", "status"]).output();
    let status = out.ok().and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok());
    match status {
        None => bail!("the claude command could not be run, so no agent check can work"),
        Some(status) if status["loggedIn"] != true => {
            bail!("the claude command is not signed in. Run `claude auth login` in a terminal")
        }
        Some(_) => {
            println!("agent: claude is signed in");
            Ok(())
        }
    }
}

fn run_step(command: &str, instance: &Instance, worktree: &Path, log: &Path, what: &str, timeout: u64) -> Result<()> {
    let stop = AtomicBool::new(false);
    let options = Options { cwd: worktree, env: &instance.env, log, stdout: None };
    let ended = process::run(&instance.values.expand(command), &options, Duration::from_secs(timeout), &stop, |_| {})?;
    match ended {
        Ended::Exited(0) => Ok(()),
        _ => bail!("slot {}: {what} failed: {command} (see {})", instance.values.slot, log.display()),
    }
}

fn side_by_side(instances: &mut [Instance], stack: &Stack, worktrees: &[PathBuf], ready_sec: u64) -> Result<()> {
    for (instance, worktree) in instances.iter_mut().zip(worktrees) {
        instance.start(stack, worktree, &|_| {})?;
    }
    let stop = AtomicBool::new(false);
    thread::scope(|scope| -> Result<()> {
        let waits: Vec<_> = instances
            .iter_mut()
            .map(|instance| {
                let stop = &stop;
                scope.spawn(move || {
                    let slot = instance.values.slot;
                    instance
                        .await_ready(stack, Duration::from_secs(ready_sec), stop)
                        .map_err(|error| anyhow::anyhow!("slot {slot}: {error}"))
                })
            })
            .collect();
        for wait in waits {
            wait.join().expect("readiness thread panicked")?;
        }
        Ok(())
    })
}

fn each_uses_its_own_data(instances: &[Instance]) -> Result<()> {
    for instance in instances {
        let unused = instance.services_unused()?;
        if !unused.is_empty() {
            bail!(
                "slot {}: nothing reached its {} database. If the app uses one, it's still going to the one in .env: \
                 point the setting it reads at the slot's value in verifique.yaml. If it doesn't, remove that service",
                instance.values.slot,
                unused.join(" or ")
            );
        }
    }
    Ok(())
}

fn stopped_cleanly(urls: &[String]) -> Result<()> {
    let deadline = Instant::now() + GONE_WITHIN;
    while urls.iter().any(|url| ureq::get(url).timeout(Duration::from_secs(1)).call().is_ok()) {
        if Instant::now() > deadline {
            bail!("something still answers after the stack stopped: an app left a process behind");
        }
        sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn check_stack(name: &str, stack: &Stack, project: &Project) -> Result<()> {
    let repo = Path::new(project.repo);
    let timeouts = &project.config.timeouts;
    let log_dir = paths::home().join("doctor").join(name);
    fs::create_dir_all(&log_dir)?;
    let log = log_dir.join("prepare.log");

    // A stack that needs a limited resource, such as one simulator, can't be proved side by side.
    let count: usize = if stack.needs.is_empty() { 2 } else { 1 };
    let base = Env::from([("VERIFIQUE_REPO".to_string(), project.repo.to_string())]);
    let mut instances = (0..count)
        .map(|index| {
            let slot = project.first_slot + index as u32;
            Instance::new(Some(stack), &project.config.services, project.repo, slot, &base, &log_dir)
        })
        .collect::<Result<Vec<_>>>()?;

    let urls: Vec<String> = instances.iter().flat_map(|instance| instance.ready_urls(stack)).collect();
    if urls.is_empty() {
        bail!("stack {name} has no ready check, so nothing can prove it started");
    }
    // Copies share one build, as a job on several slots does.
    let worktrees: Vec<PathBuf> = (0..count)
        .map(|_| paths::slot_worktree(project.repo, 0))
        .collect();
    let mut prepared: Vec<&PathBuf> = Vec::new();
    for (instance, worktree) in instances.iter_mut().zip(&worktrees) {
        if !prepared.contains(&worktree) {
            println!("  preparing {}", worktree.display());
            git::checkout_slot(repo, worktree, project.commit)?;
            copy_env_files(stack, repo, worktree)?;
            instance.load_env_files(stack, worktree);
            for command in &stack.prepare {
                run_step(command, instance, worktree, &log, "prepare", timeouts.prepare_sec)?;
            }
            prepared.push(worktree);
        } else {
            instance.load_env_files(stack, worktree);
        }
    }

    println!("  starting {count} side by side");
    let began = Instant::now();
    let proved = (|| -> Result<()> {
        for (instance, worktree) in instances.iter().zip(&worktrees) {
            instance.reset_services()?;
            if let Some(seed) = &stack.seed {
                run_step(seed, instance, worktree, &log_dir.join("seed.log"), "seed", timeouts.seed_sec)?;
            }
        }
        side_by_side(&mut instances, stack, &worktrees, timeouts.ready_sec)?;
        println!("  ready in {}s", began.elapsed().as_secs());
        // Sign-in often lives here, and a check that dies on it wastes a whole build.
        let check_steps = project.config.checks.values().filter(|check| check.stack.as_deref() == Some(name));
        let setups: Vec<&String> = stack.setup.iter().chain(check_steps.flat_map(|check| &check.setup)).collect();
        for (instance, worktree) in instances.iter().zip(&worktrees) {
            for setup in &setups {
                run_step(setup, instance, worktree, &log_dir.join("setup.log"), "setup", timeouts.setup_sec)?;
            }
        }
        each_uses_its_own_data(&instances)
    })();
    for instance in &mut instances {
        instance.stop();
        instance.remove_services();
    }
    proved?;
    stopped_cleanly(&urls)?;
    println!("  each slot used its own data, and both stopped cleanly");
    Ok(())
}

pub fn run(cwd: &Path) -> Result<()> {
    let root = git::worktree_root(cwd)?;
    let repo = git::repo_root(cwd)?;
    let config = Config::load(&root)?;
    println!("config: {} check(s), {} stack(s)", config.checks.len(), config.stacks.len());
    check_agents(&config)?;
    for stack in config.stacks.values() {
        warn_about_remote_databases(stack, Path::new(&repo));
    }

    let store = Store::open()?;
    store.set(KEY_PAUSED_BY, &std_process::id().to_string())?;
    let _pause = Pause(&store);
    if server::is_running(&store)? {
        while !store.running()?.is_empty() {
            println!("waiting for running checks to finish");
            sleep(Duration::from_secs(5));
        }
    }

    let snapshot = git::snapshot(cwd)?;
    let first_slot = store
        .setting(KEY_FIRST_SLOT)?
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_FIRST_SLOT);
    let project = Project { config: &config, repo: &repo, commit: &snapshot.commit, first_slot };
    for (name, stack) in &config.stacks {
        println!("stack {name}");
        check_stack(name, stack, &project)?;
    }
    println!("doctor passed");
    Ok(())
}
