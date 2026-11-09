#![cfg_attr(windows, windows_subsystem = "windows")]
//! Keeps the queue running for a service manager, with the recorded environment and its output in
//! a log.
//!
//!     verifique-service --env-file FILE --log FILE -- PROGRAM ARGS...
//!
//! It exists for Windows, where every console program a scheduled task starts opens a window. This
//! one is built as a windowed program that opens none, and it starts the queue without one too. A
//! scheduled task only restarts a program that fails to start, so this launcher brings the queue
//! back whenever it ends, as launchd's KeepAlive and systemd's Restart=always do. Ending the task
//! ends the launcher, and the job it holds the queue in ends the queue and everything it runs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "../platform/job_object.rs"]
mod job_object;

/// A queue that ends sooner than this after starting is failing, so the next start waits longer.
const STEADY: Duration = Duration::from_secs(10);

fn read_env(path: &str) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|arg| arg == flag).and_then(|at| args.get(at + 1)).cloned();
    let (Some(env_file), Some(log), Some(split)) = (value("--env-file"), value("--log"), args.iter().position(|arg| arg == "--"))
    else {
        eprintln!("usage: verifique-service --env-file FILE --log FILE -- PROGRAM ARGS...");
        return ExitCode::from(2);
    };
    let Some((program, rest)) = args[split + 1..].split_first() else {
        return ExitCode::from(2);
    };
    #[cfg(windows)]
    let Ok(job) = job_object::Job::new() else {
        return ExitCode::from(1);
    };

    // A program that is gone, such as after an uninstall, will not come back by waiting.
    while Path::new(program).is_file() {
        let began = Instant::now();
        let Ok(mut note) = OpenOptions::new().create(true).append(true).open(&log) else {
            return ExitCode::from(1);
        };
        let (Ok(output), Ok(errors)) = (note.try_clone(), note.try_clone()) else {
            return ExitCode::from(1);
        };
        let mut command = Command::new(program);
        command.args(rest).envs(read_env(&env_file)).stdin(Stdio::null()).stdout(output).stderr(errors);

        #[cfg(windows)]
        let child = job.spawn(command);
        #[cfg(not(windows))]
        let child = command.spawn();

        let ended = match child {
            Ok(mut child) => child.wait().map(|status| status.to_string()).unwrap_or_else(|error| error.to_string()),
            Err(error) => format!("could not start: {error}"),
        };
        // Whatever the queue left running ends with it.
        #[cfg(windows)]
        job.terminate();

        let pause = if began.elapsed() < STEADY { STEADY } else { Duration::from_secs(1) };
        let _ = writeln!(note, "verifique-service: the queue ended ({ended}); starting it again in {}s", pause.as_secs());
        sleep(pause);
    }
    ExitCode::from(1)
}
