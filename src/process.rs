use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::platform::{self, Pid, Tree};

const TICK: Duration = Duration::from_millis(100);
pub const GRACE: Duration = Duration::from_secs(3);

pub type Env = BTreeMap<String, String>;

pub struct Options<'a> {
    pub cwd: &'a Path,
    pub env: &'a Env,
    pub log: &'a Path,
    /// Where stdout goes when the caller wants to read it. Otherwise it joins the log.
    pub stdout: Option<&'a Path>,
}

/// A shell command and everything it starts.
pub struct Proc {
    tree: Tree,
    pub id: Pid,
}

pub enum Ended {
    Exited(i32),
    TimedOut,
    Interrupted,
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// A shell command held as one tree, so that stopping it reaches its children too.
pub fn start(command: &str, options: &Options) -> Result<Proc> {
    let stderr = append(options.log).with_context(|| format!("opening {}", options.log.display()))?;
    let stdout = match options.stdout {
        Some(path) => File::create(path)?,
        None => stderr.try_clone()?,
    };
    let mut shell = platform::shell(command)?;
    shell.current_dir(options.cwd).envs(options.env).stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    let tree = Tree::spawn(shell).with_context(|| format!("starting: {command}"))?;
    let id = tree.id();
    Ok(Proc { tree, id })
}

impl Proc {
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.tree.exited()
    }

    pub fn stop(&mut self) {
        self.tree.stop(GRACE);
    }
}

/// Run to completion, or stop the tree when time runs out or the server is shutting down.
pub fn run(
    command: &str,
    options: &Options,
    timeout: Duration,
    stop: &AtomicBool,
    on_start: impl FnOnce(Pid),
) -> Result<Ended> {
    let mut proc = start(command, options)?;
    on_start(proc.id);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = proc.exited() {
            // Whatever the command left running must not outlive its step.
            proc.tree.kill();
            return Ok(Ended::Exited(status.code().unwrap_or(128)));
        }
        if stop.load(Ordering::Relaxed) {
            proc.stop();
            return Ok(Ended::Interrupted);
        }
        if Instant::now() > deadline {
            proc.stop();
            return Ok(Ended::TimedOut);
        }
        sleep(TICK);
    }
}
