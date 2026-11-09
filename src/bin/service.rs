#![cfg_attr(windows, windows_subsystem = "windows")]
//! Starts the queue for a service manager, with the recorded environment and its output in a log.
//!
//!     verifique-service --env-file FILE --log FILE -- PROGRAM ARGS...
//!
//! It exists for Windows, where every console program a scheduled task starts opens a window. This
//! one is built as a windowed program that opens none, and it starts the queue without one too. It
//! holds the queue in a job, so ending the task ends the queue and everything it runs.

use std::fs::{self, OpenOptions};
use std::process::{Command, ExitCode, Stdio};

#[cfg(windows)]
#[path = "../platform/job_object.rs"]
mod job_object;

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
    let Ok(output) = OpenOptions::new().create(true).append(true).open(&log) else {
        return ExitCode::from(1);
    };
    let Ok(errors) = output.try_clone() else {
        return ExitCode::from(1);
    };
    let mut command = Command::new(program);
    command.args(rest).envs(read_env(&env_file)).stdin(Stdio::null()).stdout(output).stderr(errors);

    #[cfg(windows)]
    let (_job, child) = match job_object::Job::new().and_then(|job| job.spawn(command).map(|child| (job, child))) {
        Ok(started) => started,
        Err(_) => return ExitCode::from(1),
    };
    #[cfg(not(windows))]
    let child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return ExitCode::from(1),
    };

    let mut child = child;
    let status = child.wait();
    // Whatever the queue left running ends with it.
    #[cfg(windows)]
    _job.terminate();
    match status {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(_) => ExitCode::from(1),
    }
}
