use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker;
use crate::config::{CheckType, Values, VerdictSource};
use crate::git;
use crate::instance::{copy_env_files, Instance};
use crate::paths;
use crate::process::{self, Ended, Env, Options};
use crate::server::now;
use crate::store::{Job, Store};

/// What a verifier reports. Only an agent has to fill in more than the verdict.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Report {
    pub verdict: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub saw: String,
    #[serde(default)]
    pub not_checked: String,
}

const HOLD_POLL: Duration = Duration::from_millis(250);

pub struct Finished {
    pub status: String,
    pub summary: String,
    pub timings: BTreeMap<String, u64>,
    pub report: Option<Report>,
    pub evidence: Vec<String>,
}

pub enum Outcome {
    Done(Finished),
    Interrupted,
}

enum Halt {
    Failed(String),
    Interrupted,
}

impl From<anyhow::Error> for Halt {
    fn from(error: anyhow::Error) -> Self {
        Halt::Failed(format!("{error:#}"))
    }
}

impl From<std::io::Error> for Halt {
    fn from(error: std::io::Error) -> Self {
        Halt::Failed(error.to_string())
    }
}

fn failed<T>(message: impl Into<String>) -> Result<T, Halt> {
    Err(Halt::Failed(message.into()))
}

fn evidence_files(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(&path) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if !entry.file_name().to_string_lossy().starts_with('.') {
                found.push(path.to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

// A check that reports through a file may only pass by writing a well-formed verdict: a verifier
// that crashes or stops early must never read as success.
fn read_report(path: &Path) -> Result<Report, Halt> {
    let Ok(raw) = fs::read_to_string(path) else {
        return failed("check left no readable verdict file");
    };
    let Ok(report) = serde_json::from_str::<Report>(&raw) else {
        return failed("check left no readable verdict file");
    };
    if !["passed", "failed", "blocked"].contains(&report.verdict.as_str()) {
        return failed(format!("check gave an unknown verdict: {}", report.verdict));
    }
    Ok(report)
}

fn require_proof(report: &Report, evidence: &[String]) -> Result<(), Halt> {
    let mut missing = Vec::new();
    if report.steps.iter().all(|step| step.trim().is_empty()) {
        missing.push("steps");
    }
    if report.saw.trim().is_empty() {
        missing.push("what it saw");
    }
    if evidence.is_empty() {
        missing.push("an evidence file");
    }
    if !missing.is_empty() {
        return failed(format!("a pass needs proof, and this one has no {}", missing.join(" or ")));
    }
    Ok(())
}

struct Run<'a> {
    job: &'a Job,
    store: Store,
    worktree: PathBuf,
    dir: PathBuf,
    stop: &'a AtomicBool,
    timings: BTreeMap<String, u64>,
}

impl Run<'_> {
    fn step(
        &self,
        command: &str,
        values: &Values,
        env: &Env,
        log: &str,
        stdout: Option<&Path>,
        timeout: u64,
    ) -> Result<Ended, Halt> {
        let log = self.dir.join(format!("{log}.log"));
        let options = Options { cwd: &self.worktree, env, log: &log, stdout };
        let ended = process::run(
            &values.expand(command),
            &options,
            Duration::from_secs(timeout),
            self.stop,
            |pgid| {
                let _ = self.store.add_pgid(self.job.id, pgid);
            },
        )?;
        if matches!(ended, Ended::Interrupted) {
            return Err(Halt::Interrupted);
        }
        Ok(ended)
    }

    fn timed<T>(&mut self, name: &str, step: impl FnOnce(&mut Self) -> Result<T, Halt>) -> Result<T, Halt> {
        let began = Instant::now();
        let out = step(self);
        self.timings.insert(name.to_string(), began.elapsed().as_secs());
        out
    }

    fn require(&self, what: &str, command: &str, ended: Ended) -> Result<(), Halt> {
        match ended {
            Ended::Exited(0) => Ok(()),
            Ended::Exited(code) => failed(format!("{what} failed ({code}): {command}")),
            _ => failed(format!("{what} timed out: {command}")),
        }
    }

    fn prepare(&self, first: &Instance) -> Result<(), Halt> {
        let plan = &self.job.plan;
        let stack_steps = plan.stack.iter().flat_map(|stack| &stack.prepare);
        for command in stack_steps.chain(&plan.check.prepare) {
            let ended = self.step(command, &first.values, &first.env, "prepare", None, plan.timeouts.prepare_sec)?;
            self.require("prepare", command, ended)?;
        }
        Ok(())
    }

    fn fresh_data(&self, instances: &[Instance]) -> Result<(), Halt> {
        let plan = &self.job.plan;
        let seed = plan.stack.as_ref().and_then(|stack| stack.seed.as_ref());
        for instance in instances {
            instance.reset_services()?;
            if let Some(seed) = seed {
                let ended = self.step(seed, &instance.values, &instance.env, "seed", None, plan.timeouts.seed_sec)?;
                self.require("seed", seed, ended)?;
            }
        }
        Ok(())
    }

    fn start_stacks(&self, instances: &mut [Instance]) -> Result<(), Halt> {
        let plan = &self.job.plan;
        let Some(stack) = &plan.stack else { return Ok(()) };
        if let Some(collision) = Instance::shared_port(instances) {
            return failed(collision);
        }
        let track = |pgid: i32| {
            let _ = self.store.add_pgid(self.job.id, pgid);
        };
        for instance in instances.iter_mut() {
            instance.start(stack, &self.worktree, &track)?;
        }
        let timeout = Duration::from_secs(plan.timeouts.ready_sec);
        for instance in instances.iter_mut() {
            match instance.await_ready(stack, timeout, self.stop) {
                Err(_) if self.stop.load(Ordering::Relaxed) => return Err(Halt::Interrupted),
                Err(error) => return failed(error.to_string()),
                Ok(()) => {}
            }
        }
        Ok(())
    }

    /// Everything the project's setup printed, for the check to use.
    fn setup(&self, instances: &[Instance]) -> Result<String, Halt> {
        let plan = &self.job.plan;
        let commands: Vec<&String> = plan.stack.iter().flat_map(|stack| &stack.setup).chain(&plan.check.setup).collect();
        let mut printed = String::new();
        for instance in instances {
            for command in &commands {
                let out = self.dir.join("setup.out");
                let ended = self.step(command, &instance.values, &instance.env, "setup", Some(&out), plan.timeouts.setup_sec)?;
                self.require("setup", command, ended)?;
                printed.push_str(&fs::read_to_string(&out).unwrap_or_default());
            }
        }
        fs::write(self.dir.join("setup.txt"), &printed)?;
        Ok(printed)
    }

    /// Every stack's details, for a suite that runs one worker per stack.
    fn describe_stacks(&self, instances: &[Instance]) -> Result<Env, Halt> {
        let stacks: Vec<serde_json::Value> = instances
            .iter()
            .map(|instance| serde_json::json!({
                "slot": instance.values.slot,
                "ports": instance.values.ports,
                "values": instance.values.tokens(),
            }))
            .collect();
        let path = self.dir.join("stacks.json");
        fs::write(&path, serde_json::to_vec_pretty(&stacks).map_err(anyhow::Error::from)?)?;
        let slots: Vec<String> = instances.iter().map(|instance| instance.values.slot.to_string()).collect();
        let mut env = instances[0].env.clone();
        env.insert("VERIFIQUE_SLOTS".to_string(), slots.join(","));
        env.insert("VERIFIQUE_STACKS".to_string(), path.to_string_lossy().into_owned());
        Ok(env)
    }

    fn check(&mut self, instances: &[Instance], setup: &str) -> Result<(String, String, Option<Report>, Vec<String>), Halt> {
        let plan = &self.job.plan;
        let check = &plan.check;
        let result_file = self.dir.join("result.json");
        let evidence_dir = self.dir.join("evidence");
        let is_agent = check.kind == CheckType::Agent;
        let values = &instances[0].values;

        let command = if is_agent {
            let prompt = checker::prompt(&checker::Inputs {
                job: self.job,
                values,
                setup,
                diff_file: &self.dir.join("diff.patch"),
                result_file: &result_file,
                evidence: &evidence_dir,
            });
            fs::write(self.dir.join("prompt.md"), prompt)?;
            checker::command(check, plan.headed, &self.dir, &evidence_dir)?
        } else {
            check.run.clone().expect("validated when the config was loaded")
        };
        let env = self.describe_stacks(instances)?;
        let timeout = plan.timeouts.run_sec;
        let ended = self.timed("check", |run| run.step(&command, values, &env, "check", None, timeout))?;

        let code = match ended {
            Ended::Exited(code) => code,
            _ => return failed("check timed out"),
        };
        let evidence = evidence_files(&evidence_dir);
        if is_agent || check.verdict == VerdictSource::File {
            if code != 0 {
                return failed(format!("check exited with {code}"));
            }
            let report = read_report(&result_file)?;
            if is_agent && report.verdict == "passed" {
                require_proof(&report, &evidence)?;
            }
            return Ok((report.verdict.clone(), report.summary.clone(), Some(report), evidence));
        }
        Ok(if code == 0 {
            ("passed".to_string(), "check exited 0".to_string(), None, evidence)
        } else {
            ("failed".to_string(), format!("check exited with {code}"), None, evidence)
        })
    }

    /// Keep the stack up for an investigation until it is released or its time runs out.
    fn hold(&mut self, instances: &mut [Instance], minutes: f64) -> Result<Finished, Halt> {
        self.describe_stacks(instances)?;
        let until = now() + (minutes * 60.0).ceil() as u64;
        self.store.set_hold_until(self.job.id, until)?;
        let summary = loop {
            if self.stop.load(Ordering::Relaxed) {
                break "ended because the server stopped".to_string();
            }
            if self.store.release_requested(self.job.id)? {
                break "released".to_string();
            }
            if now() >= until {
                break format!("expired after {minutes} minutes");
            }
            if !instances.iter_mut().all(Instance::running) {
                break "ended because the stack stopped: see its logs".to_string();
            }
            sleep(HOLD_POLL);
        };
        Ok(Finished {
            status: "released".to_string(),
            summary,
            timings: std::mem::take(&mut self.timings),
            report: None,
            evidence: Vec::new(),
        })
    }

    fn attempt(&mut self, instances: &mut [Instance]) -> Result<Finished, Halt> {
        let job = self.job;
        let repo = Path::new(&job.repo);
        fs::write(self.dir.join("request.md"), &job.request)?;
        fs::write(self.dir.join("brief.md"), &job.brief)?;
        fs::write(self.dir.join("diff.patch"), git::diff(repo, job.base.as_deref(), &job.commit)?)?;
        git::checkout_slot(repo, &self.worktree, &job.commit)?;
        if let Some(stack) = &job.plan.stack {
            copy_env_files(stack, repo, &self.worktree)?;
            for instance in instances.iter_mut() {
                instance.load_env_files(stack, &self.worktree);
            }
        }

        self.timed("prepare", |run| run.prepare(&instances[0]))?;
        self.timed("seed", |run| run.fresh_data(instances))?;
        self.timed("stack", |run| run.start_stacks(instances))?;
        let setup = self.timed("setup", |run| run.setup(instances))?;
        if let Some(minutes) = job.plan.hold_minutes {
            return self.hold(instances, minutes);
        }
        let (status, summary, report, evidence) = self.check(instances, &setup)?;
        Ok(Finished { status, summary, timings: std::mem::take(&mut self.timings), report, evidence })
    }
}

fn job_env(job: &Job, dir: &Path) -> Env {
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let mut env = Env::from([
        ("VERIFIQUE_JOB".to_string(), job.id.to_string()),
        ("VERIFIQUE_REPO".to_string(), job.repo.clone()),
        ("VERIFIQUE_COMMIT".to_string(), job.commit.clone()),
        ("VERIFIQUE_TREE".to_string(), job.tree.clone()),
        ("VERIFIQUE_REQUEST".to_string(), path("request.md")),
        ("VERIFIQUE_BRIEF".to_string(), path("brief.md")),
        ("VERIFIQUE_DIFF".to_string(), path("diff.patch")),
        ("VERIFIQUE_SETUP".to_string(), path("setup.txt")),
        ("VERIFIQUE_PROMPT".to_string(), path("prompt.md")),
        ("VERIFIQUE_RESULT".to_string(), path("result.json")),
        ("VERIFIQUE_EVIDENCE".to_string(), path("evidence")),
    ]);
    if job.plan.headed {
        env.insert("VERIFIQUE_HEADED".to_string(), "1".to_string());
    }
    env
}

pub fn run(job: &Job, slot_index: usize, slots: &[u32], stop: &AtomicBool) -> Result<Outcome> {
    let dir = paths::job_dir(job.id);
    fs::create_dir_all(dir.join("evidence"))?;
    let base = job_env(job, &dir);
    let mut instances = slots
        .iter()
        .map(|slot| Instance::new(job.plan.stack.as_ref(), &job.plan.services, &job.repo, *slot, &base, &dir))
        .collect::<Result<Vec<_>>>()?;
    let mut run = Run {
        job,
        store: Store::open()?,
        worktree: paths::slot_worktree(&job.repo, slot_index),
        dir,
        stop,
        timings: BTreeMap::new(),
    };
    let result = run.attempt(&mut instances);
    for instance in &mut instances {
        instance.stop();
        instance.remove_services();
    }
    Ok(match result {
        Ok(finished) => Outcome::Done(finished),
        Err(Halt::Interrupted) => Outcome::Interrupted,
        Err(Halt::Failed(summary)) => Outcome::Done(Finished {
            status: "error".to_string(),
            summary,
            timings: run.timings,
            report: None,
            evidence: Vec::new(),
        }),
    })
}
