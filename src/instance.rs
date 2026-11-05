use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{ServiceSpec, Stack, Values};
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

    pub fn start(&mut self, stack: &Stack, worktree: &Path, track: &dyn Fn(i32)) -> Result<()> {
        self.clear_ports(worktree)?;
        let slot = self.values.slot;
        if let Some(start) = &stack.start {
            let log = self.log_dir.join(format!("stack-{slot}.log"));
            let options = Options { cwd: worktree, env: &self.env, log: &log, stdout: None };
            let proc = process::start(&self.values.expand(start), &options)?;
            track(proc.pgid);
            self.procs.push(proc);
        }
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
        let mut urls: Vec<String> = stack.ready.iter().map(|url| self.values.expand(url)).collect();
        for (name, app) in &stack.apps {
            let (Some(ready), Some(port)) = (&app.ready, self.values.ports.get(name)) else { continue };
            urls.push(format!("http://127.0.0.1:{port}{}", self.values.expand_for(name, ready)));
        }
        urls
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
