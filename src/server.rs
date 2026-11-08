use std::collections::BTreeMap;
use std::fs;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, sleep};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde_json::json;

use crate::config::Needs;
use crate::git;
use crate::notify::Notifier;
use crate::paths;
use crate::process::{alive, kill_group};
use crate::scheduler::{pick, unavailable, Candidate};
use crate::store::{Job, Store};
use crate::worker::{self, Finished, Outcome};

pub const KEY_PID: &str = "server_pid";
pub const KEY_HEARTBEAT: &str = "server_heartbeat";
const UNREADABLE_PLAN: &str = "submitted by a different version of verifique, so submit it again";
pub const KEY_SLOTS: &str = "server_slots";
pub const KEY_FIRST_SLOT: &str = "server_first_slot";
pub const KEY_PAUSED_BY: &str = "paused_by";
pub const KEY_LOW_DISK: &str = "low_disk";

/// A heartbeat older than this means the server is gone, however it ended.
pub const HEARTBEAT_LIMIT: u64 = 15;

pub struct Options {
    pub slots: usize,
    pub first_slot: u32,
    pub resources: Needs,
    pub min_free_gb: u64,
    pub poll: Duration,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0)
}

/// Whether a server is alive. Agents and people need telling when nothing will run their job.
pub fn is_running(store: &Store) -> Result<bool> {
    let beat = store.setting(KEY_HEARTBEAT)?.and_then(|value| value.parse::<u64>().ok());
    Ok(beat.is_some_and(|beat| now().saturating_sub(beat) < HEARTBEAT_LIMIT))
}

pub fn is_paused(store: &Store) -> Result<bool> {
    let holder = store.setting(KEY_PAUSED_BY)?.and_then(|value| value.parse::<i32>().ok());
    Ok(holder.is_some_and(alive))
}

fn publish(job: &Job, finished: &Finished) -> Result<()> {
    let path = paths::verdict_file(&job.tree, &job.check);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let verdict = json!({
        "job": job.id,
        "repo": job.repo,
        "branch": job.branch,
        "commit": job.commit,
        "tree": job.tree,
        "check": job.check,
        "status": finished.status,
        "summary": finished.summary,
        "steps": finished.report.as_ref().map(|report| &report.steps),
        "saw": finished.report.as_ref().map(|report| &report.saw),
        "not_checked": finished.report.as_ref().map(|report| &report.not_checked),
        "evidence": finished.evidence,
        "timings": finished.timings,
    });
    fs::write(path, serde_json::to_vec_pretty(&verdict)?)?;
    Ok(())
}

struct Done {
    job: Job,
    outcome: Result<Outcome>,
}

struct Running {
    repo: String,
    indices: Vec<usize>,
    needs: Needs,
    hold: bool,
    /// Set to stop this job: on a cancel, or when the server itself stops.
    stop: Arc<AtomicBool>,
}

struct Server {
    store: Store,
    options: Options,
    busy: BTreeMap<i64, Running>,
    last_served: BTreeMap<String, u64>,
    turn: u64,
    freeing: Arc<AtomicBool>,
    done: Sender<Done>,
    notifier: Notifier,
}

impl Server {
    fn recover(&mut self) -> Result<()> {
        // A job a previous server left behind has no owner, and its stack holds the slot's ports.
        for mut job in self.store.running()? {
            for pgid in std::mem::take(&mut job.pgids) {
                kill_group(pgid, libc::SIGKILL);
            }
            // A check can simply run again, but whoever held a stack has lost it.
            if !job.plan_readable {
                self.store.finish(&job, "error", UNREADABLE_PLAN)?;
            } else if job.plan.hold_minutes.is_some() {
                self.store.finish(&job, "released", "ended because the server restarted")?;
            } else if self.store.release_requested(job.id)? {
                self.store.finish(&job, "cancelled", "cancelled while it ran")?;
            } else {
                self.store.requeue(job.id)?;
            }
        }
        Ok(())
    }

    fn in_use(&self) -> Needs {
        let mut used = Needs::new();
        for running in self.busy.values() {
            for (name, count) in &running.needs {
                *used.entry(name.clone()).or_default() += count;
            }
        }
        used
    }

    /// Read fresh each time, because `verifique slots` changes it while the server runs.
    fn slots(&self) -> usize {
        self.store.setting(KEY_SLOTS).ok().flatten().and_then(|value| value.parse().ok()).unwrap_or(self.options.slots)
    }

    fn free_indices(&self) -> Vec<usize> {
        let taken: Vec<usize> = self.busy.values().flat_map(|running| running.indices.iter().copied()).collect();
        (0..self.slots()).filter(|index| !taken.contains(index)).collect()
    }

    /// A check that runs out of disk fails in ways that read as a fault in the change, so the builds
    /// in idle slots go first, and checks wait until there is room. Deleting them can take minutes,
    /// which is why it happens off this loop: the loop's heartbeat is how people know the queue is up.
    fn enough_disk(&self) -> Result<bool> {
        if self.freeing.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let (floor, free) = (self.options.min_free_gb, paths::free_gb()?);
        if free >= floor {
            self.store.unset(KEY_LOW_DISK)?;
            return Ok(true);
        }
        self.store.set(KEY_LOW_DISK, &format!("Only {free} GB of disk is free, so checks are waiting until {floor} GB is"))?;
        let in_use: Vec<_> = self
            .busy
            .values()
            .map(|running| paths::slot_worktree(&running.repo, running.indices[0]))
            .collect();
        let freeing = Arc::clone(&self.freeing);
        freeing.store(true, Ordering::Relaxed);
        thread::spawn(move || {
            match git::remove_idle_worktrees(&in_use) {
                Ok(0) => {}
                Ok(removed) => println!("removed {removed} idle slot worktree(s) to free disk"),
                Err(error) => eprintln!("could not free disk: {error:#}"),
            }
            freeing.store(false, Ordering::Relaxed);
        });
        Ok(false)
    }

    fn launch(&mut self, job: Job, indices: Vec<usize>) -> Result<()> {
        let slots: Vec<u32> = indices.iter().map(|index| self.options.first_slot + *index as u32).collect();
        self.store.claim(job.id, &slots)?;
        self.turn += 1;
        self.last_served.insert(job.repo.clone(), self.turn);
        println!("job {} {} started on slot(s) {slots:?}", job.id, job.check);

        let worktree_index = indices[0];
        let hold = job.plan.hold_minutes.is_some();
        let stop = Arc::new(AtomicBool::new(false));
        self.busy.insert(job.id, Running { repo: job.repo.clone(), indices, needs: job.needs.clone(), hold, stop: Arc::clone(&stop) });
        let done = self.done.clone();
        thread::spawn(move || {
            let outcome = worker::run(&job, worktree_index, &slots, &stop);
            let _ = done.send(Done { job, outcome });
        });
        Ok(())
    }

    /// A hold watches for its own release; a check is stopped from here.
    fn stop_cancelled(&self) -> Result<()> {
        for (id, running) in &self.busy {
            if !running.hold && self.store.release_requested(*id)? {
                running.stop.store(true, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    fn dispatch(&mut self) -> Result<()> {
        let queued = self.store.queued()?;
        let mut runnable = Vec::new();
        for job in &queued {
            if !job.plan_readable {
                self.store.finish(job, "error", UNREADABLE_PLAN)?;
                continue;
            }
            match unavailable(&job.needs, &self.options.resources) {
                Some(missing) => {
                    self.store.finish(job, "error", &format!("this machine has no {missing}"))?;
                }
                None => runnable.push(job),
            }
        }
        if is_paused(&self.store)? {
            return Ok(());
        }
        let free = self.free_indices();
        if free.is_empty() {
            return Ok(());
        }
        // Holds never take the last slot, so an investigation cannot stop every check.
        let holds = self.busy.values().filter(|running| running.hold).count();
        let room_for_hold = holds < self.slots().saturating_sub(1).max(1);
        runnable.retain(|job| room_for_hold || job.plan.hold_minutes.is_none());
        let candidates: Vec<Candidate> = runnable
            .iter()
            .map(|job| Candidate {
                id: job.id,
                repo: &job.repo,
                retry: job.retry,
                priority: job.priority,
                needs: &job.needs,
            })
            .collect();
        let chosen = pick(&candidates, &self.in_use(), &self.options.resources, &self.last_served);
        let Some(job) = chosen.and_then(|id| runnable.iter().find(|job| job.id == id)) else {
            return Ok(());
        };
        // A check gets as many slots as this machine has. When it needs more than are free, nothing
        // else starts: smaller jobs slipping past would otherwise keep it waiting for ever.
        let width = job.plan.check.slots.unwrap_or(1).min(self.slots());
        if free.len() >= width && self.enough_disk()? {
            self.launch((*job).clone(), free[..width].to_vec())?;
        }
        Ok(())
    }

    fn settle(&mut self, done: Done) -> Result<()> {
        let job = done.job;
        self.busy.remove(&job.id);
        match done.outcome {
            Ok(Outcome::Interrupted) if self.store.release_requested(job.id)? => {
                self.store.finish(&job, "cancelled", "cancelled while it ran")?;
                println!("job {} cancelled", job.id);
            }
            Ok(Outcome::Interrupted) => self.store.requeue(job.id)?,
            Ok(Outcome::Done(finished)) => {
                if job.plan.hold_minutes.is_none() {
                    publish(&job, &finished)?;
                }
                let landed = self.store.finish(&job, &finished.status, &finished.summary)?;
                println!("job {} {}: {}", job.id, finished.status, finished.summary);
                if landed.is_some() {
                    self.notifier.needs_you(&job.repo, job.branch.as_deref().unwrap_or("a branch"));
                }
            }
            Err(error) => {
                eprintln!("job {} error: {error:#}", job.id);
                self.store.finish(&job, "error", &format!("{error:#}"))?;
            }
        }
        Ok(())
    }
}

pub fn serve(options: Options) -> Result<()> {
    let store = Store::open()?;
    let holder = store.setting(KEY_PID)?.and_then(|value| value.parse::<i32>().ok());
    if let Some(pid) = holder.filter(|pid| *pid != process::id() as i32 && is_running(&store).unwrap_or(false)) {
        if alive(pid) {
            bail!("a server is already running (pid {pid})");
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(libc::SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(libc::SIGINT, Arc::clone(&stop))?;

    store.set(KEY_PID, &process::id().to_string())?;
    store.set(KEY_SLOTS, &options.slots.to_string())?;
    store.set(KEY_FIRST_SLOT, &options.first_slot.to_string())?;

    let (done_tx, done_rx) = mpsc::channel();
    let poll = options.poll;
    let (slots, first_slot) = (options.slots, options.first_slot);
    let mut server = Server {
        store,
        options,
        busy: BTreeMap::new(),
        last_served: BTreeMap::new(),
        turn: 0,
        freeing: Arc::new(AtomicBool::new(false)),
        done: done_tx,
        notifier: Notifier::default(),
    };
    server.recover()?;
    println!("{} serving {slots} slot(s) from slot {first_slot}", paths::NAME);

    while !stop.load(Ordering::Relaxed) {
        server.store.set(KEY_HEARTBEAT, &now().to_string())?;
        while let Ok(done) = done_rx.try_recv() {
            server.settle(done)?;
        }
        server.notifier.flush();
        server.stop_cancelled()?;
        server.dispatch()?;
        sleep(poll);
    }

    for running in server.busy.values() {
        running.stop.store(true, Ordering::Relaxed);
    }
    while !server.busy.is_empty() {
        let done = done_rx.recv()?;
        server.settle(done)?;
    }
    server.store.unset(KEY_HEARTBEAT)?;
    server.store.unset(KEY_PID)?;
    Ok(())
}

pub fn no_server_message() -> String {
    format!(
        "no {NAME} server is running, so nothing will start. Run `{NAME} service install`, or `{NAME} serve` in a terminal.",
        NAME = paths::NAME
    )
}
