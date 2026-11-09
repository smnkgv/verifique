mod checker;
mod config;
mod doctor;
mod git;
mod init;
mod instance;
mod notify;
mod paths;
mod platform;
mod plugins;
mod process;
mod scheduler;
mod server;
mod service;
mod services;
mod store;
mod worker;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use config::{CheckType, Config, Needs};
use store::{Job, Store, Submission, TERMINAL};

const REFUSED: u8 = 5;
const NO_VERDICT: u8 = 4;
const LOG_TAIL: usize = 8;

#[derive(Parser)]
#[command(name = paths::NAME, version, about = "A local verification queue for parallel coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the queue in this terminal
    Serve(Machine),
    /// Hand work over for checking, committed or not
    Submit {
        #[arg(long)]
        check: Option<String>,
        /// The user's original request, word for word
        #[arg(long)]
        request: Option<String>,
        #[arg(long)]
        request_file: Option<PathBuf>,
        /// How to reach the change
        #[arg(long)]
        brief: Option<String>,
        #[arg(long)]
        brief_file: Option<PathBuf>,
        /// Show the browser while the check runs
        #[arg(long)]
        headed: bool,
        /// Added to the end of the check's `run`, such as the tests to run
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Start the app on a free slot to use by hand, with no check and no verdict
    Stack {
        #[command(subcommand)]
        action: StackAction,
    },
    /// Stop a queued or running job; whoever waits on it gets exit 3
    Cancel { job: i64 },
    /// Wait for a verdict: exit 0 passed, 1 failed, 2 blocked or error, 3 replaced or cancelled
    Wait { job: i64 },
    /// The queue, what is running and what needs you
    Status {
        #[arg(long)]
        json: bool,
    },
    /// One job in full
    Show {
        job: i64,
        #[arg(long)]
        json: bool,
    },
    /// Recent jobs
    List {
        #[arg(long)]
        json: bool,
    },
    /// The verdict for the content here, committed or not: exit 0 only on a pass
    Verdict {
        #[arg(long)]
        check: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run one queued job next
    Next { job: i64 },
    /// Change how many checks run at once, while the server runs
    Slots { count: usize },
    /// Hand a branch in needs-you back to its agents
    Clear {
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        check: Option<String>,
    },
    /// Delete slot worktrees and spent snapshots
    Clean,
    /// Prove this project's stacks can run side by side
    Doctor,
    /// Print the setup guide for your coding agent
    Init {
        /// Print only the wording for the project's agent instructions
        #[arg(long)]
        rules: bool,
    },
    /// Manage the background service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Add-ons such as database support, installed on top of verifique
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Ask the queue running from this home to stop, as its service manager would
    #[command(hide = true)]
    StopServer,
}

#[derive(clap::Args)]
struct Machine {
    #[arg(long, default_value_t = 2)]
    slots: usize,
    #[arg(long, default_value_t = 4)]
    first_slot: u32,
    /// Start no check with less disk free than this, in GB
    #[arg(long, default_value_t = 5)]
    min_free_gb: u64,
    /// Something limited on this machine, such as simulator=1
    #[arg(long = "resource", value_name = "NAME=COUNT")]
    resources: Vec<String>,
}

#[derive(Subcommand)]
enum StackAction {
    /// Wait for a free slot, start the stack there and print its addresses
    Start {
        /// Which stack, when the project has more than one
        name: Option<String>,
        /// When it stops by itself
        #[arg(long, default_value_t = 30.0)]
        minutes: f64,
        /// One of the stack's `seeds`, run after its usual seed, such as to load real data
        #[arg(long)]
        seed: Option<String>,
        #[arg(long)]
        json: bool,
        /// Added to the end of the seed's command, such as the files to load
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Stop a started stack and give its slot back
    Stop { job: i64 },
    /// Run a command in a started stack's slot with its settings, as its seed runs
    Exec {
        job: i64,
        /// Joined and run by the shell, as ssh does: quote what your own shell should not read,
        /// such as `&&`
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// Install plugins from this version's release, such as `postgres mongo redis`
    Add {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Which plugins are installed, where, and at which version
    List,
    /// Bring every installed plugin to this version of verifique
    Update,
    /// Uninstall plugins
    Remove {
        #[arg(required = true)]
        names: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Install and start it, recording this shell's PATH
    Install(Machine),
    /// Restart it, for example after a new build
    Restart,
    /// Stop and remove it
    Uninstall,
    /// Show what the service sees and which tools it can find
    Env,
}

fn parse_resources(pairs: &[String]) -> Result<Needs> {
    pairs
        .iter()
        .map(|pair| {
            let (name, count) = pair.split_once('=').ok_or_else(|| anyhow!("bad --resource: {pair}"))?;
            Ok((name.to_string(), count.parse().with_context(|| format!("bad --resource: {pair}"))?))
        })
        .collect()
}

fn text(inline: Option<String>, file: Option<PathBuf>) -> Result<String> {
    match file {
        Some(path) => fs::read_to_string(&path).with_context(|| format!("reading {}", path.display())),
        None => Ok(inline.unwrap_or_default()),
    }
}

fn exit_code(status: &str) -> u8 {
    match status {
        "passed" => 0,
        "failed" => 1,
        "superseded" | "cancelled" => 3,
        _ => 2,
    }
}

fn warn_if_no_server(store: &Store) {
    if !server::is_running(store).unwrap_or(true) {
        eprintln!("{}", server::no_server_message());
    }
}

fn submit(check: Option<String>, request: String, brief: String, headed: bool, args: Vec<String>) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&git::worktree_root(&cwd)?)?;
    let (check, mut plan, needs) = config.plan(check.as_deref())?;
    plan.headed |= headed;
    if plan.check.kind == CheckType::Agent {
        if request.trim().is_empty() {
            bail!("check {check} needs the user's original request: pass --request or --request-file, word for word");
        }
        if !args.is_empty() {
            bail!("check {check} is an agent and takes nothing after --: say what to check in --brief");
        }
    }
    plan.args = args;

    let repo = git::repo_root(&cwd)?;
    let branch = git::current_branch(&cwd)?;
    let mut store = Store::open()?;
    if let Some(branch) = &branch {
        if let Some(reason) = store.needs_you_reason(&repo, branch, &check)? {
            eprintln!(
                "refused: {branch} needs you ({reason}). Stop and tell the user; `{} clear` hands it back.",
                paths::NAME
            );
            return Ok(ExitCode::from(REFUSED));
        }
    }

    let snapshot = git::snapshot(&cwd)?;
    let base = git::diff_base(&cwd, config.base.as_deref(), &snapshot.commit);
    let id = store.submit(Submission {
        repo: &repo,
        branch: branch.as_deref(),
        commit: &snapshot.commit,
        tree: &snapshot.tree,
        base: base.as_deref(),
        check: &check,
        request: &request,
        brief: &brief,
        plan: &plan,
        needs: &needs,
    })?;
    println!("{id}");
    if snapshot.dirty {
        eprintln!("uncommitted work captured as {}", &snapshot.tree[..9]);
    }
    warn_if_no_server(&store);
    Ok(ExitCode::SUCCESS)
}

/// Queue for a stack, and return once it is up, with everything needed to work on it.
fn stack_start(name: Option<String>, minutes: f64, seed: Option<String>, args: Vec<String>, as_json: bool) -> Result<ExitCode> {
    if !minutes.is_finite() || minutes <= 0.0 {
        bail!("--minutes must be more than zero");
    }
    if seed.is_none() && !args.is_empty() {
        bail!("what follows -- goes to a seed: name one with --seed");
    }
    let cwd = std::env::current_dir()?;
    let config = Config::load(&git::worktree_root(&cwd)?)?;
    let (mut plan, needs) = config.plan_hold(name.as_deref(), minutes, seed.as_deref())?;
    plan.args = args;
    let repo = git::repo_root(&cwd)?;
    let branch = git::current_branch(&cwd)?;
    let snapshot = git::snapshot(&cwd)?;
    let mut store = Store::open()?;
    let id = store.submit(Submission {
        repo: &repo,
        branch: branch.as_deref(),
        commit: &snapshot.commit,
        tree: &snapshot.tree,
        base: None,
        check: "stack",
        request: "",
        brief: "",
        plan: &plan,
        needs: &needs,
    })?;
    warn_if_no_server(&store);
    eprintln!("job {id}: waiting for a slot");
    loop {
        let job = store.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
        if TERMINAL.contains(&job.status.as_str()) {
            eprintln!("{}: {}", job.status, job.summary.unwrap_or_default());
            return Ok(ExitCode::from(exit_code("error")));
        }
        if let Some(until) = job.hold_until {
            print_hold(&job, until, as_json)?;
            return Ok(ExitCode::SUCCESS);
        }
        sleep(Duration::from_millis(500));
    }
}

fn print_hold(job: &Job, until: u64, as_json: bool) -> Result<()> {
    let dir = paths::job_dir(job.id);
    let stacks: Value = serde_json::from_str(&fs::read_to_string(dir.join("stacks.json"))?)?;
    let setup = fs::read_to_string(dir.join("setup.txt")).unwrap_or_default();
    let left = until.saturating_sub(server::now());
    if as_json {
        let out = json!({ "job": job.id, "seconds_left": left, "stacks": stacks, "setup": setup, "logs": dir });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!("stack {} is up for {} minutes", job.id, left.div_ceil(60));
    for (name, port) in stacks[0]["ports"].as_object().into_iter().flatten() {
        println!("  {name}: http://127.0.0.1:{port}");
    }
    if !setup.trim().is_empty() {
        println!("\nsetup printed:\n{}", setup.trim());
    }
    println!("\nlogs: {}", dir.display());
    println!("run a command in it with: {} stack exec {} -- <command>", paths::NAME, job.id);
    println!("stop it with: {} stack stop {}", paths::NAME, job.id);
    Ok(())
}

/// Run a command in a started stack the way its seed runs, and pass its exit code on.
fn stack_exec(id: i64, command: Vec<String>) -> Result<ExitCode> {
    let store = Store::open()?;
    let job = store.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
    if job.plan.hold_minutes.is_none() {
        bail!("job {id} is a check, not a started stack");
    }
    let raw = match (job.status.as_str(), fs::read_to_string(paths::job_dir(id).join("exec.json"))) {
        ("running", Ok(raw)) => raw,
        ("queued" | "running", _) => bail!("stack {id} is not up yet: `{} stack start` prints its addresses once it is", paths::NAME),
        (status, _) => bail!("stack {id} is no longer up ({status}): start a new one with `{} stack start`", paths::NAME),
    };
    let exec: Value = serde_json::from_str(&raw)?;
    let env: BTreeMap<String, String> = serde_json::from_value(exec["env"].clone())?;
    let cwd = exec["cwd"].as_str().ok_or_else(|| anyhow!("stack {id} recorded no folder"))?;
    let status = platform::shell(&command.join(" "))?
        .current_dir(cwd)
        .envs(&env)
        .status()
        .with_context(|| format!("running in stack {id}"))?;
    Ok(ExitCode::from((status.code().unwrap_or(1) & 0xff) as u8))
}

fn stack_stop(id: i64) -> Result<ExitCode> {
    let mut store = Store::open()?;
    let job = store.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
    if job.plan.hold_minutes.is_none() {
        bail!("job {id} is a check, not a started stack: stop it with `{} cancel {id}`", paths::NAME);
    }
    match job.status.as_str() {
        "queued" => {
            store.finish(&job, "stopped", "stopped before it started")?;
        }
        "running" => store.request_release(id)?,
        _ => {}
    }
    println!("stack {id} stopped");
    Ok(ExitCode::SUCCESS)
}

fn cancel(id: i64) -> Result<ExitCode> {
    let mut store = Store::open()?;
    let job = store.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
    match job.status.as_str() {
        "queued" => {
            store.finish(&job, "cancelled", "cancelled before it started")?;
            println!("job {id} cancelled");
        }
        "running" => {
            store.request_release(id)?;
            println!("job {id} is being stopped");
            warn_if_no_server(&store);
        }
        status => println!("job {id} already ended: {status}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn wait(id: i64) -> Result<ExitCode> {
    let store = Store::open()?;
    let mut warned = false;
    loop {
        let job = store.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
        if TERMINAL.contains(&job.status.as_str()) {
            println!("{}: {}", job.status, job.summary.unwrap_or_default());
            return Ok(ExitCode::from(exit_code(&job.status)));
        }
        if !warned && !server::is_running(&store)? {
            eprintln!("{}", server::no_server_message());
            warned = true;
        }
        sleep(Duration::from_secs(1));
    }
}

fn slot_list(job: &Job) -> String {
    let slots: Vec<String> = job.slots.iter().map(u32::to_string).collect();
    if slots.is_empty() {
        "-".to_string()
    } else {
        slots.join(",")
    }
}

fn short(value: &str) -> &str {
    &value[..value.len().min(9)]
}

fn status(as_json: bool) -> Result<ExitCode> {
    let store = Store::open()?;
    let running = store.running()?;
    let queued = store.queued()?;
    let needs_you = store.needs_you()?;
    let recent = store.finished(5)?;
    let serving = server::is_running(&store)?;
    let slots: u32 = store.setting(server::KEY_SLOTS)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let low_disk = store.setting(server::KEY_LOW_DISK)?.filter(|_| !queued.is_empty());
    let paused = server::is_paused(&store)?;

    if as_json {
        let out = json!({
            "server": serving,
            "slots": slots,
            "low_disk": low_disk,
            "paused": paused,
            "running": running,
            "queued": queued,
            "needs_you": needs_you,
            "recent": recent,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }

    if !serving {
        println!("{}\n", server::no_server_message());
    }
    if let Some(message) = &low_disk {
        println!("{message}\n");
    }
    if paused {
        println!("paused: `{} doctor` is running, and checks start again when it ends\n", paths::NAME);
    }
    println!("{}/{slots} slots busy, {} queued", running.len(), queued.len());
    let line = |job: &Job| {
        format!(
            "  #{}  {}  {}  {}",
            job.id,
            paths::repo_label(&job.repo),
            job.branch.as_deref().unwrap_or("-"),
            job.check
        )
    };
    if !running.is_empty() {
        println!("\nrunning");
        for job in &running {
            println!("{}  slot {}", line(job), slot_list(job));
        }
    }
    if !queued.is_empty() {
        println!("\nqueued");
        for job in &queued {
            println!("{}", line(job));
        }
    }
    if !needs_you.is_empty() {
        println!("\nneeds you");
        for item in &needs_you {
            println!("  {}  {}  {}: {}", paths::repo_label(&item.repo), item.branch, item.check, item.reason);
        }
    }
    if !recent.is_empty() {
        println!("\nrecent");
        for job in &recent {
            println!("{}  {}: {}", line(job), job.status, job.summary.as_deref().unwrap_or(""));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn verdict_json(tree: &str, check: &str) -> Option<Value> {
    let raw = fs::read_to_string(paths::verdict_file(tree, check)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn show(id: i64, as_json: bool) -> Result<ExitCode> {
    let job = Store::open()?.get(id)?.ok_or_else(|| anyhow!("no job {id}"))?;
    let verdict = verdict_json(&job.tree, &job.check).filter(|v| v["job"] == json!(id));
    let dir = paths::job_dir(id);
    if as_json {
        let out = json!({ "job": job, "args": job.plan.args, "verdict": verdict, "dir": dir });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }
    println!("job {id}: {} on {}", job.status, paths::repo_label(&job.repo));
    println!("check   {}", job.check);
    if !job.plan.args.is_empty() {
        println!("args    {}", job.plan.args.join(" "));
    }
    println!("branch  {}", job.branch.as_deref().unwrap_or("-"));
    println!("content {}  commit {}", short(&job.tree), short(&job.commit));
    if let Some(summary) = &job.summary {
        println!("\n{summary}");
    }
    if let Some(verdict) = &verdict {
        if let Some(steps) = verdict["steps"].as_array().filter(|steps| !steps.is_empty()) {
            println!("\nsteps");
            for step in steps {
                println!("  - {}", step.as_str().unwrap_or_default());
            }
        }
        for (label, field) in [("saw", "saw"), ("not checked", "not_checked")] {
            if let Some(value) = verdict[field].as_str().filter(|value| !value.is_empty()) {
                println!("\n{label}: {value}");
            }
        }
    }
    if job.status == "error" {
        print_log_tail(&dir);
    }
    println!("\nfiles   {}", dir.display());
    Ok(ExitCode::SUCCESS)
}

/// A one-line error rarely says why, and the reason is usually the end of whichever step ran last.
fn print_log_tail(dir: &Path) {
    let newest = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .max_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    let Some(log) = newest else { return };
    let Ok(raw) = fs::read_to_string(log.path()) else { return };
    let lines: Vec<&str> = raw.lines().collect();
    println!("\nend of {}", log.file_name().to_string_lossy());
    for line in &lines[lines.len().saturating_sub(LOG_TAIL)..] {
        println!("  {line}");
    }
}

fn list(as_json: bool) -> Result<ExitCode> {
    let jobs = Store::open()?.recent(30)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&jobs)?);
        return Ok(ExitCode::SUCCESS);
    }
    for job in jobs {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            job.id,
            job.status,
            slot_list(&job),
            job.check,
            job.branch.as_deref().unwrap_or("-"),
            short(&job.tree),
            job.summary.as_deref().unwrap_or("")
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn verdict(check: Option<String>, as_json: bool) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&git::worktree_root(&cwd)?)?;
    let (check, _, _) = config.plan(check.as_deref())?;
    let tree = git::snapshot(&cwd)?.tree;
    match verdict_json(&tree, &check) {
        Some(found) => {
            let status = found["status"].as_str().unwrap_or("error").to_string();
            if as_json {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                println!("{status}: {}", found["summary"].as_str().unwrap_or_default());
            }
            Ok(ExitCode::from(exit_code(&status)))
        }
        None => {
            println!("no verdict for this content ({})", short(&tree));
            Ok(ExitCode::from(NO_VERDICT))
        }
    }
}

fn clear(branch: Option<String>, check: Option<String>) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let repo = git::repo_root(&cwd)?;
    let branch = match branch {
        Some(branch) => branch,
        None => git::current_branch(&cwd)?.ok_or_else(|| anyhow!("not on a branch: pass --branch"))?,
    };
    let changed = Store::open()?.clear(&repo, &branch, check.as_deref())?;
    println!("cleared {branch} ({changed} check(s))");
    Ok(ExitCode::SUCCESS)
}

fn clean() -> Result<ExitCode> {
    let store = Store::open()?;
    let active: Vec<Job> = store.running()?.into_iter().chain(store.queued()?).collect();
    let in_use: Vec<PathBuf> = store
        .running()?
        .iter()
        .filter_map(|job| {
            let first: u32 = store.setting(server::KEY_FIRST_SLOT).ok()??.parse().ok()?;
            Some(paths::slot_worktree(&job.repo, (*job.slots.first()? - first) as usize))
        })
        .collect();

    let removed = git::remove_idle_worktrees(&in_use)?;
    for repo_dir in fs::read_dir(paths::home().join("worktrees")).into_iter().flatten().flatten() {
        let Ok(repo) = fs::read_to_string(repo_dir.path().join("repo")) else { continue };
        if !Path::new(&repo).is_dir() {
            fs::remove_dir_all(repo_dir.path())?;
            continue;
        }
        let keep: Vec<String> = active.iter().filter(|job| job.repo == repo).map(|job| job.tree.clone()).collect();
        git::delete_snapshot_refs(Path::new(&repo), &keep)?;
    }
    println!("removed {removed} slot worktree(s)");
    Ok(ExitCode::SUCCESS)
}

fn run(cli: Cli) -> Result<ExitCode> {
    // Agents pipe output into `head` and `grep -q`, which stop reading early. A long-lived server
    // should outlast its reader, and everything else should end quietly as other commands do.
    if !matches!(cli.command, Commands::Serve(_)) {
        platform::end_quietly_on_closed_output();
    }
    match cli.command {
        Commands::Serve(machine) => {
            let poll = std::env::var("VERIFIQUE_POLL_MS").ok().and_then(|ms| ms.parse().ok()).unwrap_or(500);
            server::serve(server::Options {
                slots: machine.slots,
                first_slot: machine.first_slot,
                resources: parse_resources(&machine.resources)?,
                min_free_gb: machine.min_free_gb,
                poll: Duration::from_millis(poll),
            })?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Submit { check, request, request_file, brief, brief_file, headed, args } => {
            submit(check, text(request, request_file)?, text(brief, brief_file)?, headed, args)
        }
        Commands::Stack { action } => match action {
            StackAction::Start { name, minutes, seed, json, args } => stack_start(name, minutes, seed, args, json),
            StackAction::Stop { job } => stack_stop(job),
            StackAction::Exec { job, command } => stack_exec(job, command),
        },
        Commands::Cancel { job } => cancel(job),
        Commands::Wait { job } => wait(job),
        Commands::Status { json } => status(json),
        Commands::Show { job, json } => show(job, json),
        Commands::List { json } => list(json),
        Commands::Verdict { check, json } => verdict(check, json),
        Commands::Next { job } => {
            if !Store::open()?.prioritise(job)? {
                bail!("job {job} is not queued");
            }
            println!("job {job} runs next");
            Ok(ExitCode::SUCCESS)
        }
        Commands::Slots { count } => {
            if count == 0 {
                bail!("the queue needs at least one slot");
            }
            Store::open()?.set(server::KEY_SLOTS, &count.to_string())?;
            println!("{count} slot(s)");
            Ok(ExitCode::SUCCESS)
        }
        Commands::Clear { branch, check } => clear(branch, check),
        Commands::Clean => clean(),
        Commands::Plugin { action } => {
            match action {
                PluginAction::Add { names } => plugins::add(&names)?,
                PluginAction::List => plugins::list()?,
                PluginAction::Update => plugins::update()?,
                PluginAction::Remove { names } => plugins::remove(&names)?,
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::StopServer => {
            server::ask_to_stop()?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Doctor => {
            doctor::run(&std::env::current_dir()?)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Init { rules } => {
            print!("{}", if rules { init::rules() } else { init::guide() });
            Ok(ExitCode::SUCCESS)
        }
        Commands::Service { action } => {
            match action {
                ServiceAction::Install(machine) => {
                    parse_resources(&machine.resources)?;
                    service::install(&service::Install {
                        slots: machine.slots,
                        first_slot: machine.first_slot,
                        min_free_gb: machine.min_free_gb,
                        resources: machine.resources,
                    })?;
                }
                ServiceAction::Restart => service::restart()?,
                ServiceAction::Uninstall => service::uninstall()?,
                ServiceAction::Env => service::show_env()?,
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(64)
        }
    }
}
