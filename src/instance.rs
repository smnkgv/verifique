use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{App, ServiceSpec, Stack, Values};
use crate::process::{self, Env, Options, Proc};
use crate::services::Service;

const READY_POLL: Duration = Duration::from_millis(250);
const READY_REQUEST: Duration = Duration::from_secs(3);

/// One isolated copy of a stack: its slot's values, its databases and its running processes.
pub struct Instance {
    pub values: Values,
    pub env: Env,
    services: Vec<Service>,
    procs: Vec<Proc>,
    log_dir: PathBuf,
}

/// A gitignored file a project needs is not in a slot's worktree, so bring it over from the main
/// checkout, where a user's own copy lives.
pub fn copy_env_files(stack: &Stack, repo: &Path, worktree: &Path) -> Result<()> {
    for file in stack.env_files() {
        let source = repo.join(&file);
        if source.is_file() {
            let target = worktree.join(&file);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target).with_context(|| format!("copying {file}"))?;
        }
    }
    Ok(())
}

/// Production start commands rarely read `.env` themselves, so the file reaches them through the
/// environment. A line it can't read is skipped, as dotenv skips it.
fn parse_env(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches("export ");
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() || key.contains(char::is_whitespace) {
                return None;
            }
            let value = value.trim();
            let quoted = value.len() >= 2 && (value.starts_with('"') && value.ends_with('"') || value.starts_with('\'') && value.ends_with('\''));
            let value = if quoted { &value[1..value.len() - 1] } else { value.split(" #").next().unwrap_or("").trim_end() };
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

impl Instance {
    pub fn new(
        stack: Option<&Stack>,
        services: &BTreeMap<String, ServiceSpec>,
        repo: &str,
        slot: u32,
        base: &Env,
        log_dir: &Path,
    ) -> Result<Self> {
        let services: Vec<Service> = services
            .iter()
            .map(|(kind, spec)| Service::new(kind, spec.url.as_deref(), repo, slot))
            .collect::<Result<_>>()?;
        let tokens = services.iter().flat_map(Service::tokens).collect();
        let values = Values::new(stack, slot, tokens);

        let mut env = base.clone();
        env.insert("VERIFIQUE_SLOT".to_string(), slot.to_string());
        for (name, port) in &values.ports {
            env.insert(format!("VERIFIQUE_PORT_{}", name.to_uppercase()), port.to_string());
        }
        for (name, value) in stack.iter().flat_map(|stack| &stack.env) {
            env.insert(name.clone(), values.expand(value));
        }
        Ok(Self { values, env, services, procs: Vec::new(), log_dir: log_dir.to_path_buf() })
    }

    /// Anything already set wins: the stack's own settings, and the slot's values above all.
    pub fn load_env_files(&mut self, stack: &Stack, worktree: &Path) {
        for file in stack.env_files() {
            let Ok(raw) = fs::read_to_string(worktree.join(&file)) else { continue };
            for (key, value) in parse_env(&raw) {
                self.env.entry(key).or_insert(value);
            }
        }
    }

    pub fn reset_services(&self) -> Result<()> {
        self.services.iter().try_for_each(Service::reset)
    }

    pub fn remove_services(&self) {
        // Whatever fails here is put right by the next reset of this slot.
        for service in &self.services {
            let _ = service.remove();
        }
    }

    pub fn services_unused(&self) -> Result<Vec<String>> {
        let mut unused = Vec::new();
        for service in &self.services {
            if !service.in_use()? {
                unused.push(service.kind().to_string());
            }
        }
        Ok(unused)
    }

    /// Anything listening on a slot's port would answer the ready check in place of the app about
    /// to start, and the check would then run against the wrong code. A worktree runs one job at
    /// a time, so a listener from this one can only be left over from an earlier job.
    fn clear_ports(&self, worktree: &Path) -> Result<()> {
        let worktree = fs::canonicalize(worktree)?;
        for port in self.values.ports.values() {
            for pid in process::listeners(*port)? {
                let name = process::name(pid);
                match process::working_dir(pid) {
                    Some(cwd) if cwd.starts_with(&worktree) => {
                        eprintln!("stopping {name} (pid {pid}), left over on port {port}");
                        process::stop_pid(pid);
                    }
                    cwd => {
                        let place = cwd.map_or_else(String::new, |cwd| format!(" in {}", cwd.display()));
                        bail!("port {port} is held by {name} (pid {pid}){place}, so this stack can't start. Stop it and submit again");
                    }
                }
            }
        }
        Ok(())
    }

    /// The first port that two copies, or two apps in one copy, would both use. Copies of one job
    /// share a worktree, so starting the second would stop the first one's app as if it were left over.
    pub fn shared_port(instances: &[Instance]) -> Option<String> {
        let mut seen = BTreeSet::new();
        let port = instances.iter().flat_map(|instance| instance.values.ports.values().copied()).find(|port| !seen.insert(*port))?;
        Some(format!(
            "two copies would both use port {port}, so one would stop the other's app. \
             Space the apps' ports further apart than the number of slots, such as 3500 and 3600"
        ))
    }

    pub fn start(&mut self, stack: &Stack, worktree: &Path, track: &dyn Fn(i32)) -> Result<()> {
        self.clear_ports(worktree)?;
        let slot = self.values.slot;
        for (name, app) in &stack.apps {
            let mut env = self.env.clone();
            for (key, value) in &app.env {
                env.insert(key.clone(), self.values.expand_for(name, value));
            }
            let cwd = app.cwd.as_ref().map_or_else(|| worktree.to_path_buf(), |cwd| worktree.join(cwd));
            let log = self.log_dir.join(format!("app-{name}-{slot}.log"));
            let options = Options { cwd: &cwd, env: &env, log: &log, stdout: None };
            let proc = process::start(&self.values.expand_for(name, &app.run), &options)?;
            track(proc.pgid);
            self.procs.push(proc);
        }
        Ok(())
    }

    pub fn ready_urls(&self, stack: &Stack) -> Vec<String> {
        let ready = |(name, app): (&String, &App)| {
            let (ready, port) = (app.ready.as_ref()?, self.values.ports.get(name)?);
            Some(format!("http://127.0.0.1:{port}{}", self.values.expand_for(name, ready)))
        };
        stack.apps.iter().filter_map(ready).collect()
    }

    pub fn await_ready(&mut self, stack: &Stack, timeout: Duration, stop: &AtomicBool) -> Result<()> {
        let urls = self.ready_urls(stack);
        let deadline = Instant::now() + timeout;
        for url in &urls {
            loop {
                if self.procs.iter_mut().any(|proc| proc.exited().is_some()) {
                    return Err(anyhow!("stack exited before it was ready"));
                }
                if stop.load(Ordering::Relaxed) {
                    return Err(anyhow!("interrupted"));
                }
                if ureq::get(url).timeout(READY_REQUEST).call().is_ok() {
                    break;
                }
                if Instant::now() > deadline {
                    return Err(anyhow!("{url} never became ready"));
                }
                sleep(READY_POLL);
            }
        }
        Ok(())
    }

    pub fn running(&mut self) -> bool {
        self.procs.iter_mut().all(|proc| proc.exited().is_none())
    }

    pub fn stop(&mut self) {
        for proc in &mut self.procs {
            proc.stop();
        }
        self.procs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::parse_env;

    #[test]
    fn env_files_are_read_the_way_dotenv_reads_them() {
        let raw = "# a comment\nPLAIN=one\nexport EXPORTED=two\nQUOTED=\"three # kept\"\nSINGLE='four'\nTRAILING=five # dropped\nEMPTY=\nno equals here\n";
        let parsed = parse_env(raw);
        let get = |key: &str| parsed.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("PLAIN"), Some("one"));
        assert_eq!(get("EXPORTED"), Some("two"));
        assert_eq!(get("QUOTED"), Some("three # kept"));
        assert_eq!(get("SINGLE"), Some("four"));
        assert_eq!(get("TRAILING"), Some("five"));
        assert_eq!(get("EMPTY"), Some(""));
        assert_eq!(parsed.len(), 6);
    }
}
