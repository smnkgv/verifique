//! Drives the real binary against throwaway git repositories, with a real server running.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_verifique");
const FIRST_SLOT: u32 = 8;

// Tests run in parallel, so each world takes its own range of ports.
// Below 32768, where Linux starts handing out ports for outgoing connections (macOS starts at
// 49152), so a test's app never finds its port taken by one.
static NEXT_BASE: AtomicU32 = AtomicU32::new(20100);

struct World {
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    base_port: u32,
    server: Option<Child>,
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A path as shell commands in verifique.yaml write it: sh reads a backslash as an escape, and
/// every OS's sh takes forward slashes.
fn in_sh(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn config(base_port: u32, helpers: &Path) -> String {
    let helpers = in_sh(helpers);
    format!(
        r#"
retries: 3
stacks:
  web:
    apps:
      web: {{ run: "python3 -m http.server {{port}} --bind 127.0.0.1", port: {base_port}, ready: / }}
  dying:
    apps:
      web: {{ run: exit 1, port: {base_port}, ready: / }}
  device:
    apps:
      web: {{ run: "python3 -m http.server {{port}} --bind 127.0.0.1", port: {base_port}, ready: / }}
    needs: {{ simulator: 1 }}
  signed-in:
    apps:
      web: {{ run: "python3 -m http.server {{port}} --bind 127.0.0.1", port: {base_port}, ready: / }}
    setup: echo "sign in as throwaway@example.test"
checks:
  manual: {{ stack: web, run: sh {helpers}/check.sh, verdict: file }}
  suite: {{ stack: web, run: sh {helpers}/check.sh }}
  dying: {{ stack: dying, run: "true" }}
  mobile: {{ stack: device, run: sh {helpers}/check.sh, verdict: file }}
  phone: {{ run: "true", needs: {{ gpu: 1 }} }}
  typecheck: {{ run: echo "$VERIFIQUE_SLOT" > "$VERIFIQUE_EVIDENCE/slot" }}
  leaves: {{ run: sh {helpers}/leave.sh {base_port} }}
  named: {{ run: printf '%s\n' > "$VERIFIQUE_EVIDENCE/args" }}
  sleeper: {{ stack: web, run: sleep 60 }}
  agent:
    type: agent
    stack: signed-in
    agent: {{ command: sh {helpers}/agent.sh }}
    notes: Remember the side door.
"#
    )
}

const CHECK: &str = r#"
echo "start $VERIFIQUE_JOB $(python3 -c 'import time; print(time.time_ns())')" >> "$LOG"
curl -fsS "http://127.0.0.1:$VERIFIQUE_PORT_WEB/" > /dev/null || exit 9
code=0
case "$(cat "$VERIFIQUE_BRIEF")" in
  slow) sleep 1; echo '{"verdict":"passed","summary":"slow"}' > "$VERIFIQUE_RESULT" ;;
  pass) echo '{"verdict":"passed","summary":"looks right"}' > "$VERIFIQUE_RESULT" ;;
  fail) echo '{"verdict":"failed","summary":"button missing"}' > "$VERIFIQUE_RESULT" ;;
  blocked) echo '{"verdict":"blocked","summary":"cannot reach it"}' > "$VERIFIQUE_RESULT" ;;
  crash) code=7 ;;
  silent) ;;
  *) echo '{"verdict":"maybe"}' > "$VERIFIQUE_RESULT" ;;
esac
echo "end $VERIFIQUE_JOB $(python3 -c 'import time; print(time.time_ns())')" >> "$LOG"
exit $code
"#;

const AGENT: &str = r#"
cp "$VERIFIQUE_PROMPT" "$VERIFIQUE_EVIDENCE/.prompt"
case "$(cat "$VERIFIQUE_BRIEF")" in
  proof)
    echo image > "$VERIFIQUE_EVIDENCE/final.png"
    echo '{"verdict":"passed","summary":"ok","steps":["opened it"],"saw":"the page"}' > "$VERIFIQUE_RESULT" ;;
  no-file)
    echo '{"verdict":"passed","summary":"ok","steps":["opened it"],"saw":"the page"}' > "$VERIFIQUE_RESULT" ;;
  no-steps)
    echo image > "$VERIFIQUE_EVIDENCE/final.png"
    echo '{"verdict":"passed","summary":"ok","saw":"the page"}' > "$VERIFIQUE_RESULT" ;;
  fail)
    echo '{"verdict":"failed","summary":"it broke"}' > "$VERIFIQUE_RESULT" ;;
esac
"#;

// A new session is how a server an agent restarts by hand escapes the job's process group.
const LEAVE: &str = r#"
port=$(( $1 + VERIFIQUE_SLOT ))
python3 -c '
import subprocess, sys
server = subprocess.Popen(
    ["python3", "-m", "http.server", sys.argv[1], "--bind", "127.0.0.1"],
    start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
)
print(server.pid)
' "$port" > "$(dirname "$0")/left.pid"
until curl -fsS "http://127.0.0.1:$port/" > /dev/null; do sleep 0.1; done
"#;

// An app with no database driver: it reports the environment it was started with.
const APP: &str = r#"
import http.server, json, os
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(dict(os.environ)).encode()
        self.send_response(200)
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
http.server.HTTPServer(("127.0.0.1", int(os.environ["PORT"])), Handler).serve_forever()
"#;

const POSTGRES: &str = "postgres://postgres:postgres@127.0.0.1:5432";

impl World {
    fn new() -> Self {
        let base_port = NEXT_BASE.fetch_add(50, Ordering::SeqCst);
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-{unique}-{base_port}"));
        // Databases are named after the repository, so each world needs a name of its own.
        let repo = root.join("project").join(format!("repo{base_port}"));
        let home = root.join("home");
        fs::create_dir_all(&repo).unwrap();
        fs::write(root.join("project/check.sh"), CHECK).unwrap();
        fs::write(root.join("project/agent.sh"), AGENT).unwrap();
        fs::write(root.join("project/app.py"), APP).unwrap();
        fs::write(root.join("project/leave.sh"), LEAVE).unwrap();
        let world = Self { root, home, repo, base_port, server: None };
        world.write_repo(&world.repo.clone());
        world
    }

    fn write_repo(&self, repo: &Path) {
        git(repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("verifique.yaml"), config(self.base_port, &self.root.join("project"))).unwrap();
        fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        commit(repo, "first");
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .current_dir(cwd)
            .args(args)
            .env("VERIFIQUE_HOME", &self.home)
            .env("VERIFIQUE_POLL_MS", "50")
            // A test that reaches needs-you would otherwise raise a real notification here.
            .env("VERIFIQUE_NOTIFY", "off")
            // Only the plugins built with this binary, never ones installed on this machine.
            .env("VERIFIQUE_PLUGINS", self.root.join("plugins"))
            .env("LOG", self.root.join("events.log"));
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.repo, args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    fn submit(&self, check: &str, brief: &str) -> String {
        self.submit_in(&self.repo, check, brief)
    }

    fn submit_in(&self, cwd: &Path, check: &str, brief: &str) -> String {
        let out = self.run_in(cwd, &["submit", "--check", check, "--request", "Add the button", "--brief", brief]);
        assert!(out.status.success(), "submit: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn serve(&mut self, slots: u32, resources: &[&str]) {
        self.serve_from(FIRST_SLOT, slots, resources);
    }

    fn serve_from(&mut self, first: u32, slots: u32, resources: &[&str]) {
        // Otherwise checks wait whenever the machine running the tests is low on disk.
        self.serve_with(first, slots, resources, &["--min-free-gb", "0"]);
    }

    fn serve_with(&mut self, first: u32, slots: u32, resources: &[&str], extra: &[&str]) {
        let slots = slots.to_string();
        let first = first.to_string();
        let mut args = vec!["serve", "--slots", &slots, "--first-slot", &first];
        args.extend(extra);
        for resource in resources {
            args.extend(["--resource", resource]);
        }
        let child = self
            .command(&self.repo, &args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.server = Some(child);
    }

    fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            // Asked the way its service manager asks, so it finishes what it holds.
            let asked = self.run(&["stop-server"]);
            if !asked.status.success() {
                let _ = server.kill();
            }
            let _ = server.wait();
        }
    }

    fn wait(&self, job: &str) -> (i32, String) {
        let out = self.run(&["wait", job]);
        (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn events(&self) -> Vec<(String, String, u128)> {
        let raw = fs::read_to_string(self.root.join("events.log")).unwrap_or_default();
        let mut events: Vec<_> = raw
            .lines()
            .map(|line| {
                let parts: Vec<&str> = line.split(' ').collect();
                (parts[0].to_string(), parts[1].to_string(), parts[2].parse().unwrap())
            })
            .collect();
        events.sort_by_key(|event: &(String, String, u128)| event.2);
        events
    }

    fn peak_concurrency(&self) -> i32 {
        let (mut open, mut peak) = (0, 0);
        for (kind, _, _) in self.events() {
            open += if kind == "start" { 1 } else { -1 };
            peak = peak.max(open);
        }
        peak
    }

    fn starts(&self) -> Vec<String> {
        self.events().into_iter().filter(|e| e.0 == "start").map(|e| e.1).collect()
    }

    fn helpers(&self) -> PathBuf {
        self.root.join("project")
    }

    /// A project that leaves isolation to the tool. `check` is YAML for its checks.
    fn use_isolation(&self, services: &str, env: &str, seed: &str, checks: &str) {
        let helpers = in_sh(&self.helpers());
        let port = self.base_port;
        let yaml = format!(
            r#"
services: {{ {services} }}
stacks:
  web:
    seed: {seed}
    env: {{ {env} }}
    apps:
      api:
        run: python3 {helpers}/app.py
        port: {port}
        ready: /health
        env: {{ PORT: "{{port}}" }}
checks:
{checks}
"#
        );
        fs::write(self.repo.join("verifique.yaml"), yaml).unwrap();
    }

    fn postgres_has(&self, database: &str) -> bool {
        let sql = format!("SELECT count(*) FROM pg_database WHERE datname = '{database}'");
        let out = Command::new("psql").args([&format!("{POSTGRES}/postgres"), "-Atc", &sql]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim() == "1"
    }

    fn database(&self, slot: u32) -> String {
        format!("verifique_repo{}_s{slot}", self.base_port)
    }

    /// Every log a job wrote, for a failure message that has to explain itself.
    fn job_logs(&self, job: &str) -> String {
        let dir = self.home.join("jobs").join(job);
        let mut logs: Vec<_> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
        logs.sort();
        logs.iter()
            .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
            .map(|path| format!("--- {}\n{}", path.display(), fs::read_to_string(path).unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A file a job wrote. A job that ended before writing it says so, which a failure message needs.
    fn job_file(&self, job: &str, name: &str) -> String {
        fs::read_to_string(self.home.join("jobs").join(job).join(name)).unwrap_or_else(|_| format!("(job {job} wrote no {name})"))
    }

    fn status_json(&self) -> serde_json::Value {
        let out = self.run(&["status", "--json"]);
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.stop_server();
        // A killed server can't tidy up, and these databases live on the machine's real server.
        for slot in [FIRST_SLOT, FIRST_SLOT + 1] {
            let sql = format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.database(slot));
            let _ = Command::new("psql").args([&format!("{POSTGRES}/postgres"), "-qc", &sql]).output();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn commit(repo: &Path, message: &str) {
    fs::write(repo.join("file.txt"), message).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", message]);
}

// Detached submissions never replace one another, so a test can queue several at once.
fn detached<T>(repo: &Path, body: impl FnOnce() -> T) -> T {
    git(repo, &["checkout", "-q", "--detach"]);
    let out = body();
    git(repo, &["checkout", "-q", "-"]);
    out
}

#[test]
fn a_file_verdict_decides_the_outcome_and_a_crash_never_passes() {
    let mut world = World::new();
    world.serve(2, &[]);
    for (brief, code, status) in [
        ("pass", 0, "passed"),
        ("fail", 1, "failed"),
        ("crash", 2, "error"),
        ("silent", 2, "error"),
        ("nonsense", 2, "error"),
    ] {
        let (got, out) = detached(&world.repo, || {
            let job = world.submit("manual", brief);
            world.wait(&job)
        });
        assert_eq!(got, code, "{brief}: {out}");
        assert!(out.starts_with(&format!("{status}:")), "{brief}: {out}");
    }
}

#[test]
fn an_exit_code_check_passes_on_zero_and_fails_otherwise() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);
    let job = world.submit("suite", "crash");
    assert_eq!(world.wait(&job).0, 1);
}

#[test]
fn a_check_with_no_stack_still_gets_a_slot() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("typecheck", "");
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.job_file(&job, "evidence/slot").trim(), FIRST_SLOT.to_string());
}

#[test]
fn what_follows_a_double_dash_reaches_the_check_word_for_word() {
    let mut world = World::new();
    world.serve(1, &[]);
    let out = world.run(&["submit", "--check", "named", "--brief", "", "--", "tests/a b.spec.ts", "it's:42"]);
    assert!(out.status.success(), "submit: {}", String::from_utf8_lossy(&out.stderr));
    let job = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.job_file(&job, "evidence/args"), "tests/a b.spec.ts\nit's:42\n");

    let refused = world.run(&["submit", "--check", "agent", "--request", "Add the button", "--brief", "", "--", "x"]);
    assert!(!refused.status.success(), "an agent check took arguments");
}

#[test]
fn a_stack_that_dies_is_reported_instead_of_hanging_the_slot() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("dying", "");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2);
    assert!(out.contains("stack exited"), "{out}");
}

#[test]
fn a_job_this_machine_can_never_run_fails_at_once() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("phone", "");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2);
    assert!(out.contains("no gpu"), "{out}");
}

#[test]
fn the_stack_is_gone_once_the_job_ends() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);
    let url = format!("127.0.0.1:{}", world.base_port + FIRST_SLOT);
    assert!(std::net::TcpStream::connect(url).is_err());
}

#[test]
fn a_server_a_check_left_behind_is_stopped_before_the_next_stack_starts() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("leaves", "");
    assert_eq!(world.wait(&job).0, 0);
    let url = format!("127.0.0.1:{}", world.base_port + FIRST_SLOT);
    // A new session escapes a process group, so on macOS and Linux the server outlives its check.
    // A Windows job holds everything a step started and ends with it, so nothing is left there.
    let left = std::net::TcpStream::connect(&url).is_ok();
    assert_eq!(left, cfg!(unix), "a new session outlives its check only outside a job");

    let job = world.submit("suite", "pass");
    let (code, out) = world.wait(&job);
    let survived = std::net::TcpStream::connect(&url).is_ok();
    if survived {
        let pid = fs::read_to_string(world.helpers().join("left.pid")).unwrap();
        let _ = Command::new("kill").arg(pid.trim()).status();
    }
    assert_eq!(code, 0, "{out}");
    assert!(!survived, "the leftover outlived the next job");
}

#[test]
fn a_process_verifique_does_not_own_is_named_and_left_alone() {
    let mut world = World::new();
    let port = world.base_port + FIRST_SLOT;
    let stranger = std::net::TcpListener::bind(("127.0.0.1", port as u16)).unwrap();
    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2);
    assert!(out.contains(&format!("port {port}")) && out.contains(&std::process::id().to_string()), "{out}");
    assert!(std::net::TcpStream::connect(stranger.local_addr().unwrap()).is_ok());
}

#[test]
fn never_more_jobs_run_at_once_than_there_are_slots() {
    let mut world = World::new();
    world.serve(2, &[]);
    let jobs = detached(&world.repo, || {
        (0..5).map(|_| world.submit("manual", "slow")).collect::<Vec<_>>()
    });
    for job in &jobs {
        assert_eq!(world.wait(job).0, 0);
    }
    assert_eq!(world.events().len(), 10);
    assert_eq!(world.peak_concurrency(), 2);
}

#[test]
fn the_slot_count_changes_while_the_server_runs() {
    let mut world = World::new();
    world.serve(1, &[]);
    let started = Instant::now();
    while world.status_json()["server"] != true && started.elapsed() < Duration::from_secs(10) {
        sleep(Duration::from_millis(50));
    }
    assert!(world.run(&["slots", "2"]).status.success());
    assert!(!world.run(&["slots", "0"]).status.success());
    let jobs = detached(&world.repo, || {
        (0..4).map(|_| world.submit("manual", "slow")).collect::<Vec<_>>()
    });
    for job in &jobs {
        assert_eq!(world.wait(job).0, 0);
    }
    assert_eq!(world.peak_concurrency(), 2);
    assert_eq!(world.status_json()["slots"], 2);
}

#[test]
fn clean_carries_on_past_a_repository_that_no_longer_exists() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = detached(&world.repo, || world.submit("manual", "pass"));
    assert_eq!(world.wait(&job).0, 0);
    world.stop_server();

    let elsewhere = world.root.join("elsewhere");
    fs::rename(&world.repo, &elsewhere).unwrap();
    let out = world.run_in(&elsewhere, &["clean"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    fs::rename(&elsewhere, &world.repo).unwrap();
    assert_eq!(fs::read_dir(world.home.join("worktrees")).unwrap().count(), 0);
}

#[test]
fn an_upgrade_keeps_the_queue_readable() {
    let mut world = World::new();
    world.serve(1, &[]);
    let finished = detached(&world.repo, || world.submit("manual", "pass"));
    assert_eq!(world.wait(&finished).0, 0);
    world.stop_server();

    // What an older version left behind: a setting this version has never heard of.
    let db = rusqlite::Connection::open(world.home.join("queue.db")).unwrap();
    let old = r#"{"check":{"browser":"headless"}}"#;
    db.execute("UPDATE jobs SET plan = ?1", [old]).unwrap();
    db.execute_batch(
        "INSERT INTO jobs (repo, commit_sha, tree, check_name, request, brief, plan, needs, status)
         SELECT repo, commit_sha, tree, check_name, request, brief, plan, needs, 'queued' FROM jobs",
    )
    .unwrap();
    let queued = db.last_insert_rowid().to_string();
    drop(db);

    // Reading the queue changes nothing, so an older server still running keeps its work.
    let status = world.status_json();
    assert_eq!(status["queued"].as_array().unwrap().len(), 1);
    assert_eq!(status["recent"][0]["status"], "passed");

    world.serve(1, &[]);
    let (code, verdict) = world.wait(&queued);
    assert_eq!(code, 2);
    assert!(verdict.contains("different version"), "{verdict}");
}

#[test]
fn a_repository_cloned_again_in_the_same_place_keeps_working() {
    let mut world = World::new();
    world.serve(1, &[]);
    let first = detached(&world.repo, || world.submit("manual", "pass"));
    assert_eq!(world.wait(&first).0, 0);

    fs::remove_dir_all(world.repo.join(".git")).unwrap();
    world.write_repo(&world.repo.clone());
    commit(&world.repo, "cloned again");
    let second = detached(&world.repo, || world.submit("manual", "pass"));
    assert_eq!(world.wait(&second).0, 0);
}

#[test]
fn output_piped_into_a_reader_that_stops_early_ends_quietly() {
    let bin = in_sh(Path::new(BIN));
    let out = Command::new("sh").arg("-c").arg(format!("'{bin}' init | head -1")).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn checks_wait_for_disk_after_idle_slots_are_cleared() {
    let mut world = World::new();
    world.serve(1, &[]);
    let first = detached(&world.repo, || world.submit("manual", "pass"));
    assert_eq!(world.wait(&first).0, 0);
    world.stop_server();
    let slots = fs::read_dir(world.home.join("worktrees")).unwrap().next().unwrap().unwrap().path();
    assert!(slots.join("0").exists());

    world.serve_with(FIRST_SLOT, 1, &[], &["--min-free-gb", "100000000"]);
    commit(&world.repo, "more work");
    let second = detached(&world.repo, || world.submit("manual", "pass"));
    let started = Instant::now();
    let cleared = || world.status_json()["low_disk"].is_string() && !slots.join("0").exists();
    while !cleared() && started.elapsed() < Duration::from_secs(10) {
        sleep(Duration::from_millis(50));
    }
    let status = world.status_json();
    assert_eq!(status["server"], true);
    assert!(status["low_disk"].as_str().unwrap().contains("GB"), "{status}");
    assert_eq!(status["queued"].as_array().unwrap().len(), 1);
    assert!(!slots.join("0").exists());
    world.stop_server();

    world.serve(1, &[]);
    assert_eq!(world.wait(&second).0, 0);
    assert!(world.status_json()["low_disk"].is_null());
}

#[test]
fn two_repositories_share_one_machines_slots_and_take_turns() {
    let mut world = World::new();
    let other = world.root.join("project").join("other");
    fs::create_dir_all(&other).unwrap();
    world.write_repo(&other);
    git(&other, &["checkout", "-q", "--detach"]);

    let busy = detached(&world.repo, || {
        let mut jobs: Vec<String> = (0..4).map(|_| world.submit("manual", "slow")).collect();
        jobs.push(world.submit_in(&other, "manual", "slow"));
        jobs
    });
    world.serve(1, &[]);
    for job in &busy {
        assert_eq!(world.wait(job).0, 0);
    }
    assert_eq!(world.peak_concurrency(), 1);
    // The other repository submitted last, yet it does not wait behind all four.
    let last = busy.last().unwrap();
    let position = world.starts().iter().position(|job| job == last).unwrap();
    assert!(position <= 1, "started at {position}");
}

#[test]
fn a_job_waiting_on_a_busy_resource_does_not_hold_up_jobs_behind_it() {
    let mut world = World::new();
    let (first, second, web) = detached(&world.repo, || {
        (world.submit("mobile", "slow"), world.submit("mobile", "slow"), world.submit("manual", "pass"))
    });
    world.serve(2, &["simulator=1"]);
    for job in [&first, &second, &web] {
        assert_eq!(world.wait(job).0, 0);
    }
    assert_eq!(world.starts().last(), Some(&second));
    assert_eq!(world.starts().len(), 3);
    let _ = (first, web);
}

#[test]
fn a_newer_submission_replaces_its_branchs_queued_job_for_the_same_check() {
    let mut world = World::new();
    let older = world.submit("manual", "pass");
    let other_check = world.submit("suite", "pass");
    commit(&world.repo, "second");
    let newer = world.submit("manual", "pass");
    world.serve(2, &[]);

    assert_eq!(world.wait(&older).0, 3);
    assert_eq!(world.wait(&other_check).0, 0);
    assert_eq!(world.wait(&newer).0, 0);
}

#[test]
fn next_puts_a_queued_job_first() {
    let mut world = World::new();
    let (first, second, urgent) = detached(&world.repo, || {
        (world.submit("manual", "pass"), world.submit("manual", "pass"), world.submit("manual", "pass"))
    });
    assert!(world.run(&["next", &urgent]).status.success());
    world.serve(1, &[]);
    for job in [&first, &second, &urgent] {
        world.wait(job);
    }
    assert_eq!(world.starts(), vec![urgent, first, second]);
}

#[test]
fn three_failures_send_a_branch_to_needs_you_until_it_is_cleared() {
    let mut world = World::new();
    world.serve(1, &[]);
    for _ in 0..3 {
        let job = world.submit("manual", "fail");
        assert_eq!(world.wait(&job).0, 1);
    }
    let refused = world.run(&["submit", "--check", "manual", "--brief", "pass"]);
    assert_eq!(refused.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("needs you"));
    assert_eq!(world.status_json()["needs_you"][0]["failures"], 3);

    // Another check on the same branch is a separate count.
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);

    assert!(world.run(&["clear"]).status.success());
    assert_eq!(world.status_json()["needs_you"].as_array().unwrap().len(), 0);
    let job = world.submit("manual", "pass");
    assert_eq!(world.wait(&job).0, 0);
}

#[test]
fn a_pass_resets_the_failure_count() {
    let mut world = World::new();
    world.serve(1, &[]);
    for brief in ["fail", "fail", "pass", "fail", "fail"] {
        let job = world.submit("manual", brief);
        world.wait(&job);
    }
    assert_eq!(world.status_json()["needs_you"].as_array().unwrap().len(), 0);
}

#[test]
fn blocked_goes_straight_to_needs_you_and_a_crash_counts_for_nothing() {
    let mut world = World::new();
    world.serve(1, &[]);
    for _ in 0..4 {
        let job = world.submit("manual", "crash");
        assert_eq!(world.wait(&job).0, 2);
    }
    assert_eq!(world.status_json()["needs_you"].as_array().unwrap().len(), 0);

    let job = world.submit("manual", "blocked");
    assert_eq!(world.wait(&job).0, 2);
    assert_eq!(world.status_json()["needs_you"].as_array().unwrap().len(), 1);
}

#[test]
fn uncommitted_work_is_checked_and_the_branch_is_left_alone() {
    let mut world = World::new();
    fs::write(world.repo.join("draft.txt"), "new file").unwrap();
    fs::write(world.repo.join("file.txt"), "edited").unwrap();
    fs::write(world.repo.join(".env"), "SECRET=1").unwrap();
    git(&world.repo, &["add", "file.txt"]);
    let head = git(&world.repo, &["rev-parse", "HEAD"]);
    let staged = git(&world.repo, &["diff", "--cached", "--name-only"]);

    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);

    assert_eq!(git(&world.repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&world.repo, &["diff", "--cached", "--name-only"]), staged);
    let diff = world.job_file(&job, "diff.patch");
    assert!(diff.contains("draft.txt"), "{diff}");
    assert!(diff.contains("edited"), "{diff}");
    assert!(!diff.contains("SECRET"), "{diff}");
}

#[test]
fn a_commit_of_the_same_content_already_counts_as_verified() {
    let mut world = World::new();
    fs::write(world.repo.join("draft.txt"), "work").unwrap();
    assert_eq!(world.run(&["verdict", "--check", "suite"]).status.code(), Some(4));

    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.run(&["verdict", "--check", "suite"]).status.code(), Some(0));

    commit(&world.repo, "file.txt unchanged content differs");
    assert_eq!(world.run(&["verdict", "--check", "suite"]).status.code(), Some(4));
    git(&world.repo, &["reset", "-q", "--soft", "HEAD~1"]);
    git(&world.repo, &["checkout", "-q", "HEAD", "--", "file.txt"]);
    git(&world.repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "the work"]);
    assert_eq!(world.run(&["verdict", "--check", "suite"]).status.code(), Some(0));
}

#[test]
fn a_linked_worktree_submits_with_its_own_config_and_shares_the_repository() {
    let mut world = World::new();
    let linked = world.root.join("project").join("linked");
    let target = linked.to_string_lossy().into_owned();
    git(&world.repo, &["worktree", "add", "-q", "-b", "feature", &target]);
    commit(&linked, "feature");
    world.serve(1, &[]);
    let job = world.submit_in(&linked, "suite", "pass");
    assert_eq!(world.wait(&job).0, 0);
    let shown = world.run(&["show", &job, "--json"]);
    let shown: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["job"]["branch"], "feature");
    let repo = fs::canonicalize(shown["job"]["repo"].as_str().unwrap()).unwrap();
    assert_eq!(repo, world.repo.canonicalize().unwrap());
}

#[test]
fn an_agent_check_refuses_a_submission_without_the_original_request() {
    let world = World::new();
    let out = world.run(&["submit", "--check", "agent", "--brief", "proof"]);
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("original request"));
}

#[test]
fn an_agent_pass_needs_steps_observation_and_evidence() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("agent", "proof");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 0, "{out}");

    let job = world.submit("agent", "no-file");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("evidence file"), "{out}");

    let job = world.submit("agent", "no-steps");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("no steps"), "{out}");
}

#[test]
fn an_agent_failure_needs_no_proof() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("agent", "fail");
    assert_eq!(world.wait(&job).0, 1);
}

#[test]
fn the_checker_receives_the_request_brief_notes_setup_and_diff() {
    let mut world = World::new();
    world.serve(1, &[]);
    fs::write(world.repo.join("draft.txt"), "work").unwrap();
    let job = world.submit("agent", "proof");
    world.wait(&job);
    let prompt = world.job_file(&job, "evidence/.prompt");
    for expected in [
        "Add the button",
        "proof",
        "Remember the side door.",
        "throwaway@example.test",
        "diff.patch",
        "independent verifier",
    ] {
        assert!(prompt.contains(expected), "missing {expected}:\n{prompt}");
    }
    assert!(world.job_file(&job, "diff.patch").contains("draft.txt"));
}

#[test]
fn a_restarted_server_picks_up_what_was_running() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("manual", "slow");
    let deadline = Instant::now() + Duration::from_secs(20);
    while world.status_json()["running"].as_array().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "job never started");
        sleep(Duration::from_millis(20));
    }
    // A kill leaves no chance to tidy up, which is what a crash or a reboot looks like.
    if let Some(mut server) = world.server.take() {
        server.kill().unwrap();
        server.wait().unwrap();
    }
    world.serve(1, &[]);
    assert_eq!(world.wait(&job).0, 0);
}

#[test]
fn a_graceful_stop_puts_a_running_job_back_in_the_queue() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("manual", "slow");
    let deadline = Instant::now() + Duration::from_secs(20);
    while world.status_json()["running"].as_array().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "job never started");
        sleep(Duration::from_millis(20));
    }
    world.stop_server();
    let status = world.status_json();
    assert_eq!(status["queued"].as_array().unwrap().len(), 1);
    assert_eq!(status["running"].as_array().unwrap().len(), 0);
    world.serve(1, &[]);
    assert_eq!(world.wait(&job).0, 0);
}

#[test]
fn nothing_running_is_said_plainly() {
    let world = World::new();
    let out = world.run(&["submit", "--check", "suite", "--brief", "pass"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no verifique server is running"));
}

#[test]
fn a_second_server_refuses_to_start() {
    let mut world = World::new();
    world.serve(1, &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while world.status_json()["server"] != true {
        assert!(Instant::now() < deadline, "server never started");
        sleep(Duration::from_millis(20));
    }
    let out = world.run(&["serve", "--slots", "1"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already running"));
}

#[test]
fn doctor_starts_two_stacks_side_by_side_and_stops_them() {
    let world = World::new();
    let yaml = format!(
        "stacks:\n  web:\n    apps:\n      web: {{ run: \"python3 -m http.server {{port}} --bind 127.0.0.1\", \
         port: {}, ready: / }}\nchecks:\n  smoke: {{ stack: web, run: \"true\" }}\n",
        world.base_port
    );
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.run(&["doctor"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{printed}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains("doctor passed"));
    assert!(printed.contains("stopped cleanly"));
}

#[test]
fn doctor_catches_an_app_that_ignores_its_slots_port() {
    let world = World::new();
    let port = world.base_port;
    let yaml = format!(
        "timeouts: {{ ready_sec: 3 }}\nstacks:\n  web:\n    apps:\n      web: {{ run: \"python3 -m http.server {port} --bind 127.0.0.1\", \
         port: {port}, ready: / }}\nchecks:\n  smoke: {{ stack: web, run: \"true\" }}\n"
    );
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ready"));
}

#[test]
fn a_database_whose_plugin_is_missing_is_named_with_the_command_that_adds_it() {
    let world = World::new();
    let yaml = "services: { cassandra: {} }\nstacks:\n  web:\n    env: { DB: \"{cassandra.url}\" }\nchecks:\n  smoke: { stack: web, run: \"true\" }\n";
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.command(&world.repo, &["doctor"]).env("VERIFIQUE_PLUGINS", world.root.join("no-plugins")).output().unwrap();
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("verifique-cassandra plugin") && said.contains("verifique plugin add cassandra"), "{said}");
}

#[test]
fn doctor_catches_two_slots_that_would_collide() {
    let world = World::new();
    let (web, api) = (world.base_port, world.base_port + 1);
    let yaml = format!(
        "stacks:\n  web:\n    apps:\n      web: {{ run: \"python3 -m http.server {{port}} --bind 127.0.0.1\", port: {web}, ready: / }}\n      \
         api: {{ run: \"python3 -m http.server {{port}} --bind 127.0.0.1\", port: {api}, ready: / }}\n\
         checks:\n  smoke: {{ stack: web, run: \"true\" }}\n"
    );
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("would both use port"));
}

#[test]
fn doctor_catches_a_stack_that_leaves_a_process_behind() {
    let world = World::new();
    let port = world.base_port;
    let start = format!("nohup setsid python3 -m http.server $(( {port} + {{slot}} )) --bind 127.0.0.1 > /dev/null 2>&1 & sleep 60");
    if Command::new("sh").args(["-c", "command -v setsid"]).output().unwrap().status.code() != Some(0) {
        return; // macOS has no setsid, and a stack cannot leave its process group without it.
    }
    let yaml = format!(
        "stacks:\n  web:\n    apps:\n      web: {{ run: \"{start}\", port: {port}, ready: / }}\n\
         checks:\n  smoke: {{ stack: web, run: \"true\" }}\n"
    );
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    let _ = Command::new("pkill").args(["-f", &format!("http.server {port}")]).status();
}

#[test]
fn clean_removes_slot_worktrees_and_spent_snapshots() {
    let mut world = World::new();
    fs::write(world.repo.join("draft.txt"), "work").unwrap();
    world.serve(1, &[]);
    let job = world.submit("suite", "pass");
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(git(&world.repo, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 2);
    world.stop_server();

    let out = world.run(&["clean"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("removed 1"));
    assert_eq!(git(&world.repo, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);
    assert_eq!(git(&world.repo, &["for-each-ref", "refs/verifique"]), "");
}

#[test]
fn init_prints_the_guide_and_the_rules_and_writes_nothing() {
    let world = World::new();
    let before = git(&world.repo, &["status", "--porcelain"]);
    let guide = String::from_utf8_lossy(&world.run(&["init"]).stdout).into_owned();
    assert!(guide.contains("verifique doctor"));
    assert!(guide.contains("Do not summarise it"));
    let rules = String::from_utf8_lossy(&world.run(&["init", "--rules"]).stdout).into_owned();
    assert!(rules.contains("needs-you"));
    assert_eq!(git(&world.repo, &["status", "--porcelain"]), before);
}

#[test]
fn a_steps_output_and_errors_reach_its_log() {
    let mut world = World::new();
    let yaml = "checks:\n  noisy: { run: 'echo to-stdout; echo to-stderr >&2; sh -c \"echo from-a-child\"; exit 3' }\n";
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    commit(&world.repo, "noisy");
    world.serve(1, &[]);
    let job = world.submit("noisy", "");
    assert_eq!(world.wait(&job).0, 1);
    let log = world.job_file(&job, "check.log");
    for line in ["to-stdout", "to-stderr", "from-a-child"] {
        assert!(log.contains(line), "{line} missing from the log: {log:?}");
    }
}

#[test]
fn an_error_shows_the_end_of_the_log_that_explains_it() {
    let mut world = World::new();
    world.serve(1, &[]);
    let job = world.submit("dying", "");
    assert_eq!(world.wait(&job).0, 2);
    let out = world.run(&["show", &job]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("end of "));
}

#[test]
fn a_swarm_can_open_a_brand_new_queue_at_once() {
    for round in 0..5 {
        let world = World::new();
        let children: Vec<Child> = (0..24)
            .map(|_| {
                world
                    .command(&world.repo, &["status", "--json"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let out = child.wait_with_output().unwrap();
            let error = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "round {round}: {error}");
        }
    }
}

#[test]
fn an_app_gets_its_slots_port_and_database_and_those_win_over_env() {
    let mut world = World::new();
    fs::write(world.repo.join(".env"), "DATABASE_URL=postgres://real.example/app\nKEPT=from-env-file\n").unwrap();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'set -x; curl -fsS "$VERIFIQUE_API" > "$VERIFIQUE_EVIDENCE/env.json" && grep KEPT .env', prepare: ['true'] }"#,
    );
    let yaml = fs::read_to_string(world.repo.join("verifique.yaml")).unwrap();
    fs::write(
        world.repo.join("verifique.yaml"),
        yaml.replace(r#"DATABASE_URL: "{postgres.url}""#, r#"DATABASE_URL: "{postgres.url}", VERIFIQUE_API: "{app.api.url}/""#),
    )
    .unwrap();
    world.serve(1, &[]);
    let job = world.submit("look", "");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 0, "{out}\n{}", world.job_logs(&job));
    let env: serde_json::Value = serde_json::from_str(&world.job_file(&job, "evidence/env.json")).unwrap();
    assert_eq!(env["PORT"], (world.base_port + FIRST_SLOT).to_string());
    let expected = format!("{POSTGRES}/{}", world.database(FIRST_SLOT));
    assert_eq!(env["DATABASE_URL"], expected);
    assert_eq!(env["KEPT"], "from-env-file");
}

#[test]
fn every_check_starts_with_a_fresh_database_and_leaves_none_behind() {
    let mut world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        r#"psql "$DATABASE_URL" -c 'create table items (id int)'"#,
        r#"  write: { stack: web, run: 'test "$(psql "$DATABASE_URL" -Atc "select count(*) from items")" = 0 && psql "$DATABASE_URL" -c "insert into items values (1)"' }"#,
    );
    world.serve(1, &[]);
    for _ in 0..2 {
        let job = world.submit("write", "");
        let (code, out) = world.wait(&job);
        assert_eq!(code, 0, "{out}: {}", world.job_file(&job, "seed.log"));
    }
    assert!(!world.postgres_has(&world.database(FIRST_SLOT)));
}

#[test]
fn mongo_and_redis_start_clean_too() {
    let mut world = World::new();
    world.use_isolation(
        "mongo: {}, redis: {}",
        r#"MONGO_URL: "{mongo.url}", REDIS_URL: "{redis.url}", PREFIX: "{redis.prefix}""#,
        "'true'",
        r#"  write: { stack: web, run: 'test -z "$(redis-cli -u "$REDIS_URL" --no-auth-warning GET seen)" && redis-cli -u "$REDIS_URL" --no-auth-warning SET seen 1 && test -z "$(redis-cli --no-auth-warning -u redis://127.0.0.1:6379/0 GET "${PREFIX}seen")" && redis-cli -u redis://127.0.0.1:6379/0 --no-auth-warning SET "${PREFIX}seen" 1 && test "$(mongosh --quiet "$MONGO_URL" --eval "print(db.items.countDocuments({}))")" = 0 && mongosh --quiet "$MONGO_URL" --eval "db.items.insertOne({a: 1})"' }"#,
    );
    // Numbered Redis databases are per machine, so this test keeps a slot no other test uses.
    world.serve_from(13, 1, &[]);
    for _ in 0..2 {
        let job = world.submit("write", "");
        let (code, out) = world.wait(&job);
        assert_eq!(code, 0, "{out}\n{}", world.job_logs(&job));
    }
}

#[test]
fn a_slot_redis_has_no_database_for_leaves_database_zero_alone() {
    let mut world = World::new();
    let port = (world.base_port + 40).to_string();
    let mut redis = Command::new("redis-server")
        .args(["--port", &port, "--save", "", "--appendonly", "no", "--bind", "127.0.0.1"])
        .current_dir(&world.root)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let cli = |args: &[&str]| {
        let out = Command::new("redis-cli").args(["-p", &port]).args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let started = Instant::now();
    while cli(&["PING"]) != "PONG" && started.elapsed() < Duration::from_secs(5) {
        sleep(Duration::from_millis(50));
    }
    cli(&["SET", "canary", "alive"]);
    world.use_isolation(
        &format!("redis: {{ url: redis://127.0.0.1:{port} }}"),
        r#"REDIS_URL: "{redis.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'true' }"#,
    );
    world.serve_from(20, 1, &[]);
    let job = world.submit("look", "");
    let (code, out) = world.wait(&job);
    let canary = cli(&["GET", "canary"]);
    redis.kill().unwrap();
    let _ = redis.wait();
    assert_eq!(canary, "alive");
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("redis.conf"), "{out}");
}

#[test]
fn a_database_server_that_is_not_there_is_named_plainly() {
    let mut world = World::new();
    world.use_isolation(
        "postgres: { url: postgres://postgres:secret-word@127.0.0.1:1 }",
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'true' }"#,
    );
    world.serve(1, &[]);
    let job = world.submit("look", "");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 2);
    assert!(out.contains("prerequisite"), "{out}");
    assert!(!out.contains("secret-word"), "{out}");
}

#[test]
fn a_suite_gets_one_stack_per_slot_from_one_build() {
    let mut world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  suite: { stack: web, slots: 2, prepare: ['echo built >> "$VERIFIQUE_EVIDENCE/built"'], run: 'for port in $(python3 -c "import json,os; [print(s[\"ports\"][\"api\"]) for s in json.load(open(os.environ[\"VERIFIQUE_STACKS\"]))]" | tr -d "\r"); do curl -fsS "http://127.0.0.1:$port/" >> "$VERIFIQUE_EVIDENCE/both"; echo >> "$VERIFIQUE_EVIDENCE/both"; done; echo "$VERIFIQUE_SLOTS" > "$VERIFIQUE_EVIDENCE/slots"' }"#,
    );
    world.serve(2, &[]);
    let job = world.submit("suite", "");
    let (code, out) = world.wait(&job);
    assert_eq!(code, 0, "{out}");
    assert_eq!(world.job_file(&job, "evidence/slots").trim(), "8,9");
    assert_eq!(world.job_file(&job, "evidence/built").lines().count(), 1);
    let both = world.job_file(&job, "evidence/both");
    let databases: Vec<String> = both
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["DATABASE_URL"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(databases.len(), 2);
    assert_ne!(databases[0], databases[1]);
}

#[test]
fn a_suite_takes_what_the_machine_has() {
    let mut world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  suite: { stack: web, slots: 5, run: 'echo "$VERIFIQUE_SLOTS" > "$VERIFIQUE_EVIDENCE/slots"' }"#,
    );
    world.serve(2, &[]);
    let job = world.submit("suite", "");
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.job_file(&job, "evidence/slots").trim(), "8,9");
}

#[test]
fn a_wide_job_is_not_starved_by_small_ones_behind_it() {
    let mut world = World::new();
    let log = in_sh(&world.root.join("events.log"));
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        &format!(
            r#"  slow: {{ stack: web, run: 'echo "start $VERIFIQUE_JOB 1" >> {log}; sleep 2' }}
  wide: {{ stack: web, slots: 2, run: 'echo "start $VERIFIQUE_JOB 2" >> {log}' }}
  small: {{ stack: web, run: 'echo "start $VERIFIQUE_JOB 3" >> {log}' }}"#
        ),
    );
    let (slow, wide, small) = detached(&world.repo, || {
        (world.submit("slow", ""), world.submit("wide", ""), world.submit("small", ""))
    });
    world.serve(2, &[]);
    for job in [&slow, &wide, &small] {
        let (code, out) = world.wait(job);
        assert_eq!(code, 0, "{out}");
    }
    let raw = fs::read_to_string(world.root.join("events.log")).unwrap();
    let order: Vec<&str> = raw.lines().map(|line| line.split(' ').nth(1).unwrap()).collect();
    assert_eq!(order, vec![slow.as_str(), wide.as_str(), small.as_str()]);
}

#[test]
fn doctor_proves_each_slot_used_its_own_database() {
    let world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        r#"psql "$DATABASE_URL" -c 'create table items (id int)'"#,
        r#"  look: { stack: web, run: 'true' }"#,
    );
    let out = world.run(&["doctor"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{printed}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains("each slot used its own data"), "{printed}");
    assert!(!world.postgres_has(&world.database(4)));
}

#[test]
fn doctor_catches_an_app_that_ignores_its_slots_database() {
    let world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"UNUSED: "{postgres.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'true' }"#,
    );
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing reached its postgres database"));
}

#[test]
fn doctor_names_a_remote_database_left_in_env_without_showing_its_password() {
    let world = World::new();
    fs::write(
        world.repo.join(".env"),
        "DATABASE_URL=postgres://u:hunter2@db.prod.example:5432/app\nWAREHOUSE_URL=postgres://u:hunter2@wh.prod.example/x\n",
    )
    .unwrap();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        r#"psql "$DATABASE_URL" -c 'create table items (id int)'"#,
        r#"  look: { stack: web, run: 'true' }"#,
    );
    let out = world.run(&["doctor"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "a warning must not block: {printed}");
    assert!(printed.contains("WAREHOUSE_URL at a remote database (wh.prod.example)"), "{printed}");
    assert!(!printed.contains("DATABASE_URL at a remote"), "{printed}");
    assert!(!printed.contains("hunter2"), "{printed}");
}

fn up_for_a_stack(world: &World) {
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'true' }"#,
    );
}

fn start_stack(world: &World, minutes: &str) -> serde_json::Value {
    let out = world.run(&["stack", "start", "--minutes", minutes, "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn job_status(world: &World, job: &str) -> String {
    let out = world.run(&["show", job, "--json"]);
    let shown: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    shown["job"]["status"].as_str().unwrap().to_string()
}

fn until_status(world: &World, job: &str, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while job_status(world, job) != wanted {
        assert!(Instant::now() < deadline, "job {job} never reached {wanted}");
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_started_stack_stays_up_until_it_is_stopped_and_checks_wait_for_it() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let hold = start_stack(&world, "5");
    let port = hold["stacks"][0]["ports"]["api"].as_u64().unwrap();
    assert_eq!(port, u64::from(world.base_port + FIRST_SLOT));
    assert!(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok());
    assert!(world.postgres_has(&world.database(FIRST_SLOT)));

    let check = world.submit("look", "");
    sleep(Duration::from_millis(400));
    assert_eq!(job_status(&world, &check), "queued");

    let job = hold["job"].to_string();
    assert!(world.run(&["stack", "stop", &job]).status.success());
    assert_eq!(world.wait(&check).0, 0);
    until_status(&world, &job, "stopped");
    assert!(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_err());
}

#[test]
fn a_forgotten_stack_stops_by_itself_and_leaves_nothing_behind() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let hold = start_stack(&world, "0.02");
    let job = hold["job"].to_string();
    until_status(&world, &job, "stopped");
    let out = world.run(&["show", &job]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("expired"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while world.postgres_has(&world.database(FIRST_SLOT)) {
        assert!(Instant::now() < deadline, "the hold's database stayed behind");
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn started_stacks_always_leave_a_slot_for_checks() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(2, &[]);
    let first = start_stack(&world, "5");

    let second = world
        .command(&world.repo, &["stack", "start", "--minutes", "5", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let check = detached(&world.repo, || world.submit("look", ""));
    until_status(&world, &check, "passed");

    let held = first["job"].to_string();
    assert!(world.run(&["stack", "stop", &held]).status.success());
    let out = second.wait_with_output().unwrap();
    assert!(out.status.success());
    let next: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(world.run(&["stack", "stop", &next["job"].to_string()]).status.success());
}

#[test]
fn a_started_stack_records_no_verdict() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let hold = start_stack(&world, "5");
    assert!(world.run(&["stack", "stop", &hold["job"].to_string()]).status.success());
    until_status(&world, &hold["job"].to_string(), "stopped");
    let verdicts = world.home.join("verdicts");
    assert_eq!(fs::read_dir(verdicts).map(Iterator::count).unwrap_or(0), 0);
}

#[test]
fn a_crash_stops_a_started_stack_rather_than_bringing_it_back_with_nobody_there() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let hold = start_stack(&world, "5");
    if let Some(mut server) = world.server.take() {
        server.kill().unwrap();
        server.wait().unwrap();
    }
    world.serve(1, &[]);
    let job = hold["job"].to_string();
    until_status(&world, &job, "stopped");
}

#[test]
fn a_cancelled_job_stops_counts_for_nothing_and_tells_whoever_waits() {
    let mut world = World::new();
    world.serve(1, &[]);
    let running = world.submit("sleeper", "");
    until_status(&world, &running, "running");
    let queued = world.submit("suite", "pass");
    assert!(world.run(&["cancel", &queued]).status.success());
    assert_eq!(world.wait(&queued).0, 3);

    assert!(world.run(&["cancel", &running]).status.success());
    let began = Instant::now();
    assert_eq!(world.wait(&running).0, 3);
    assert!(began.elapsed() < Duration::from_secs(20), "the check ran on after its cancel");
    assert_eq!(job_status(&world, &running), "cancelled");
    let url = format!("127.0.0.1:{}", world.base_port + FIRST_SLOT);
    assert!(std::net::TcpStream::connect(url).is_err(), "the cancelled check's stack is still up");
    assert_eq!(world.status_json()["needs_you"], serde_json::json!([]));
}

#[test]
fn only_a_started_stack_can_be_stopped() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let job = world.submit("look", "");
    let out = world.run(&["stack", "stop", &job]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a started stack"));
}

#[test]
fn a_started_stack_can_load_one_of_its_named_seeds() {
    let mut world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'\n    seeds: { mark: 'printf ''%s\\n'' > \"$VERIFIQUE_EVIDENCE/seeded\"' }",
        r#"  look: { stack: web, run: 'true' }"#,
    );
    world.serve(1, &[]);
    let out = world.run(&["stack", "start", "--minutes", "5", "--json", "--seed", "mark", "--", "a real dump"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let started: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let job = started["job"].to_string();
    assert_eq!(world.job_file(&job, "evidence/seeded"), "a real dump\n");
    assert!(world.run(&["stack", "stop", &job]).status.success());

    let unknown = world.run(&["stack", "start", "--minutes", "5", "--seed", "nope"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("no seed nope"));
}

#[test]
fn a_command_runs_in_a_started_stack_with_its_settings_and_passes_its_exit_code_on() {
    let mut world = World::new();
    up_for_a_stack(&world);
    world.serve(1, &[]);
    let job = start_stack(&world, "5")["job"].to_string();
    let exec_file = world.home.join("jobs").join(&job).join("exec.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&exec_file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(exec_file.is_file());

    let out = world.run(&["stack", "exec", &job, "--", "echo \"$DATABASE_URL\" && pwd"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains(&world.database(FIRST_SLOT)), "{printed}");
    assert!(printed.contains("/worktrees/"), "{printed}");
    assert_eq!(world.run(&["stack", "exec", &job, "--", "exit", "7"]).status.code(), Some(7));

    let check = world.submit("look", "");
    let refused = world.run(&["stack", "exec", &check, "--", "true"]);
    assert!(String::from_utf8_lossy(&refused.stderr).contains("is a check, not a started stack"));

    assert!(world.run(&["stack", "stop", &job]).status.success());
    until_status(&world, &job, "stopped");
    let after = world.run(&["stack", "exec", &job, "--", "true"]);
    assert!(!after.status.success());
    assert!(String::from_utf8_lossy(&after.stderr).contains("no longer up"));
    assert!(!exec_file.exists());
}

#[test]
fn a_stack_with_no_apps_gives_its_checks_their_own_database() {
    let mut world = World::new();
    let yaml = format!(
        r#"
services:
  postgres: {{ url: {POSTGRES} }}
stacks:
  db:
    seed: psql "$DATABASE_URL" -c 'create table items (id int)'
    env:
      DATABASE_URL: "{{postgres.url}}"
checks:
  unit:
    stack: db
    run: psql "$DATABASE_URL" -Atc 'select count(*) from items'
"#
    );
    fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    let out = world.run(&["doctor"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{printed}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains("each slot used its own data"), "{printed}");

    world.serve(1, &[]);
    let job = world.submit("unit", "");
    assert_eq!(world.wait(&job).0, 0, "{}", world.job_file(&job, "check.log"));
    assert!(!world.postgres_has(&world.database(FIRST_SLOT)));
}

#[test]
fn doctor_takes_an_app_less_stacks_settings_for_what_its_seed_leaves_alone() {
    let world = World::new();
    let write = |env: &str| {
        let yaml = format!(
            r#"
services:
  postgres: {{ url: {POSTGRES} }}
  redis: {{}}
stacks:
  db:
    seed: psql "$DATABASE_URL" -c 'create table items (id int)'
    env:
      DATABASE_URL: "{{postgres.url}}"
{env}
checks:
  unit: {{ stack: db, run: "true" }}
"#
        );
        fs::write(world.repo.join("verifique.yaml"), yaml).unwrap();
    };

    write(r#"      REDIS_URL: "{redis.url}""#);
    let out = world.run(&["doctor"]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{printed}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains("redis: nothing used it before a check"), "{printed}");
    assert!(!printed.contains("postgres: nothing used it"), "{printed}");

    write("");
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing reached its redis database"));
}

#[test]
fn a_headed_run_is_passed_on_to_the_check() {
    let mut world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        "'true'",
        r#"  look: { stack: web, run: 'echo "[$VERIFIQUE_HEADED]" > "$VERIFIQUE_EVIDENCE/headed"' }"#,
    );
    world.serve(1, &[]);
    let job = world.submit("look", "");
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.job_file(&job, "evidence/headed").trim(), "[]");

    let out = world.run(&["submit", "--check", "look", "--headed"]);
    let job = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(world.wait(&job).0, 0);
    assert_eq!(world.job_file(&job, "evidence/headed").trim(), "[1]");
}

#[test]
fn doctor_runs_the_setup_step_a_check_depends_on() {
    let world = World::new();
    world.use_isolation(
        &format!("postgres: {{ url: {POSTGRES} }}"),
        r#"DATABASE_URL: "{postgres.url}""#,
        r#"psql "$DATABASE_URL" -c 'create table items (id int)'"#,
        r#"  look: { stack: web, run: 'true', setup: 'exit 3' }"#,
    );
    let out = world.run(&["doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("setup failed"));
}
