use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

const TICK: Duration = Duration::from_millis(100);
const GRACE: Duration = Duration::from_secs(3);

pub type Env = BTreeMap<String, String>;

pub struct Options<'a> {
    pub cwd: &'a Path,
    pub env: &'a Env,
    pub log: &'a Path,
    /// Where stdout goes when the caller wants to read it. Otherwise it joins the log.
    pub stdout: Option<&'a Path>,
}

pub struct Proc {
    child: Child,
    pub pgid: i32,
}

pub enum Ended {
    Exited(i32),
    TimedOut,
    Interrupted,
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// A shell command in its own process group, so that stopping it reaches its children too.
pub fn start(command: &str, options: &Options) -> Result<Proc> {
    let stderr = append(options.log).with_context(|| format!("opening {}", options.log.display()))?;
    let stdout = match options.stdout {
        Some(path) => File::create(path)?,
        None => stderr.try_clone()?,
    };
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(options.cwd)
        .envs(options.env)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting: {command}"))?;
    let pgid = child.id() as i32;
    Ok(Proc { child, pgid })
}

pub fn kill_group(pgid: i32, signal: i32) {
    // SAFETY: killpg takes plain integers and touches no memory. A group that has already gone
    // reports ESRCH, which is the outcome we want.
    unsafe {
        libc::killpg(pgid, signal);
    }
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: as above; signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

pub fn stop_pid(pid: i32) {
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        if !alive(pid) {
            return;
        }
        // SAFETY: as above.
        unsafe {
            libc::kill(pid, signal);
        }
        let deadline = Instant::now() + GRACE;
        while alive(pid) && Instant::now() < deadline {
            sleep(TICK);
        }
    }
}

pub fn listeners(port: u32) -> Result<Vec<i32>> {
    let port = format!("-iTCP:{port}");
    let out = Command::new("lsof").args(["-nP", &port, "-sTCP:LISTEN", "-t"]).output().context("running lsof")?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(|line| line.parse().ok()).collect())
}

pub fn working_dir(pid: i32) -> Option<PathBuf> {
    let out = Command::new("lsof").args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|line| line.strip_prefix('n')).map(PathBuf::from)
}

pub fn name(pid: i32) -> String {
    let out = Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output();
    out.map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()).unwrap_or_default()
}

impl Proc {
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    pub fn stop(&mut self) {
        kill_group(self.pgid, libc::SIGTERM);
        let deadline = Instant::now() + GRACE;
        while self.exited().is_none() && Instant::now() < deadline {
            sleep(TICK);
        }
        // A stack's children can outlive its leader, so the group always gets the final word.
        kill_group(self.pgid, libc::SIGKILL);
        let _ = self.child.wait();
    }
}

/// Run to completion, or stop the group when time runs out or the server is shutting down.
pub fn run(
    command: &str,
    options: &Options,
    timeout: Duration,
    stop: &AtomicBool,
    on_start: impl FnOnce(i32),
) -> Result<Ended> {
    let mut proc = start(command, options)?;
    on_start(proc.pgid);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = proc.exited() {
            // Whatever the command left running must not outlive its step.
            kill_group(proc.pgid, libc::SIGKILL);
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
