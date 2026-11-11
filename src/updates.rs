//! Telling people when a newer verifique is out, so nobody runs an old one without knowing.
//!
//! The queue asks GitHub for the latest release once a day, off its main loop, and records what it
//! found, its own version and which add-ons are behind in the queue's settings. `status`, `doctor`,
//! `plugin list`, `updates` and the panel only read that, so no command waits on the network.
//! Nothing here updates anything: an update ends with `verifique service restart`, which stops
//! started stacks, so the person chooses when. `verifique updates off` stops the asking.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::json;

use crate::panel;
use crate::paths::{self, NAME};
use crate::platform;
use crate::plugins;
use crate::release::{self, VERSION};
use crate::server;
use crate::service;
use crate::store::Store;

const KEY_OFF: &str = "updates_off";
const KEY_CHECKED: &str = "updates_checked";
const KEY_LATEST: &str = "updates_latest";
/// The version of the running queue, which a newer CLI compares with its own.
const KEY_QUEUE_VERSION: &str = "server_version";
const KEY_PLUGINS_BEHIND: &str = "plugins_behind";

const EVERY_S: u64 = 24 * 60 * 60;
/// After a failed attempt, such as with no network, the next one comes this much later, not a day.
const RETRY: Duration = Duration::from_secs(60 * 60);
const GUIDE: &str = "https://github.com/smnkgv/verifique#install";

fn is_off(store: &Store) -> Result<bool> {
    Ok(store.setting(KEY_OFF)?.is_some())
}

/// Asks GitHub at most once a day while the queue runs, on a thread of its own. The queue calls it on
/// every turn of its loop.
#[derive(Default)]
pub struct Checker {
    busy: Arc<AtomicBool>,
    failed: Arc<Mutex<Option<Instant>>>,
}

impl Checker {
    pub fn tick(&mut self, store: &Store) -> Result<()> {
        if self.busy.load(Ordering::Relaxed) || is_off(store)? {
            return Ok(());
        }
        let checked = store.setting(KEY_CHECKED)?.and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
        let failed_lately = self.failed.lock().ok().and_then(|failed| *failed).is_some_and(|at| at.elapsed() < RETRY);
        if server::now().saturating_sub(checked) < EVERY_S || failed_lately {
            return Ok(());
        }
        let (busy, failed) = (Arc::clone(&self.busy), Arc::clone(&self.failed));
        busy.store(true, Ordering::Relaxed);
        thread::spawn(move || {
            let outcome = check();
            if let Err(error) = &outcome {
                eprintln!("could not ask GitHub for the latest {NAME}: {error:#}");
            }
            if let Ok(mut failed) = failed.lock() {
                *failed = outcome.is_err().then(Instant::now);
            }
            busy.store(false, Ordering::Relaxed);
        });
        Ok(())
    }
}

/// One look at the latest release, recorded, and at which add-ons are behind.
fn check() -> Result<()> {
    let latest = release::latest();
    let store = Store::open()?;
    record_plugins(&store);
    let latest = latest?;
    // Turned off while GitHub was being asked.
    if is_off(&store)? {
        return Ok(());
    }
    store.set(KEY_LATEST, &latest)?;
    store.set(KEY_CHECKED, &server::now().to_string())?;
    if release::is_newer(&latest, VERSION) {
        println!("{NAME} {latest} is out, and this queue runs {VERSION}");
    }
    Ok(())
}

/// Record which add-ons are behind this verifique. Asking each one runs it, so this happens when the
/// queue starts, after each look at GitHub and when `plugin` changes them, never on a command that
/// only reads.
pub fn record_plugins(store: &Store) {
    let behind = plugins::behind();
    let _ = if behind.is_empty() { store.unset(KEY_PLUGINS_BEHIND) } else { store.set(KEY_PLUGINS_BEHIND, &behind.join(",")) };
}

pub fn queue_started(store: &Store) -> Result<()> {
    store.set(KEY_QUEUE_VERSION, VERSION)?;
    thread::spawn(|| {
        if let Ok(store) = Store::open() {
            record_plugins(&store);
        }
    });
    Ok(())
}

pub fn queue_stopped(store: &Store) -> Result<()> {
    store.unset(KEY_QUEUE_VERSION)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A newer release is out than this verifique.
    Release,
    /// This verifique is newer than the queue that's running, which needs restarting.
    Restart,
    /// Add-ons are behind the verifique the queue runs.
    Plugins,
}

#[derive(Debug, Serialize)]
pub struct Notice {
    pub kind: Kind,
    /// A sentence for a terminal.
    pub line: String,
    /// The same for one line of the panel, beside its Update button.
    pub short: String,
    /// Whether `verifique update` can do it from here. A build from source, or an install nothing on
    /// this machine explains, is updated by hand.
    pub updatable: bool,
    /// Whether updating restarts the queue, which ends started stacks.
    pub restarts: bool,
    /// What the new release changed.
    pub url: Option<String>,
}

/// The running queue, as far as the notice cares.
#[derive(Clone, Debug, PartialEq)]
enum Queue {
    Stopped,
    /// With the version it recorded. Queues from before 0.3.4 record none.
    Runs(Option<String>),
}

impl Queue {
    /// The version it runs, when that's older than `this`.
    fn older_than(&self, this: &str) -> Option<String> {
        match self {
            Queue::Runs(Some(version)) if release::is_newer(this, version) => Some(version.clone()),
            Queue::Runs(None) => Some("an older version".to_string()),
            _ => None,
        }
    }
}

/// How this verifique was installed, as far as updating it goes.
#[derive(Clone, Debug, PartialEq)]
enum Install {
    /// Built from a checkout of verifique's own source.
    Source(PathBuf),
    /// By something `verifique update` repeats.
    Known,
    /// By something nothing on this machine records.
    Unknown,
}

/// Everything a notice is made from.
struct Facts<'a> {
    off: bool,
    latest: Option<String>,
    this: &'a str,
    queue: Queue,
    plugins_behind: Vec<String>,
    install: &'a dyn Fn() -> Install,
}

/// "postgres, mongo and redis"
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn make(facts: &Facts) -> Option<Notice> {
    let this = facts.this;
    let update = format!("{NAME} update");
    let queue_runs = facts.queue != Queue::Stopped;
    if let Some(latest) = facts.latest.as_deref().filter(|latest| !facts.off && release::is_newer(latest, this)) {
        let (line, short, updatable) = match (facts.install)() {
            Install::Known => {
                let restarts = if queue_runs { ", which restarts the queue" } else { "" };
                (format!("{NAME} {latest} is out (this is {this}): run `{update}`{restarts}"), format!("{NAME} {latest} is out"), true)
            }
            Install::Source(root) => (
                format!("{NAME} {latest} is out (this is {this}): pull and build again in {}", root.display()),
                format!("{NAME} {latest} is out: pull and build again"),
                false,
            ),
            Install::Unknown => (
                format!("{NAME} {latest} is out (this is {this}): update it the way you installed it, as {GUIDE} says"),
                format!("{NAME} {latest} is out"),
                false,
            ),
        };
        let url = Some(release::page(latest));
        return Some(Notice { kind: Kind::Release, line, short, updatable, restarts: updatable && queue_runs, url });
    }
    if let Some(running) = facts.queue.older_than(this) {
        let line = format!("the queue still runs {running}, and {NAME} {this} is installed: run `{update}` to finish, which restarts the queue");
        let short = format!("The queue still runs {running}");
        return Some(Notice { kind: Kind::Restart, line, short, updatable: true, restarts: true, url: None });
    }
    if !facts.plugins_behind.is_empty() && facts.queue == Queue::Runs(Some(this.to_string())) {
        let names = and_list(&facts.plugins_behind);
        let are = if facts.plugins_behind.len() == 1 { "add-on is" } else { "add-ons are" };
        let line = format!("the {names} {are} behind {NAME} {this}: run `{update}`");
        let short = format!("The {names} {are} behind {this}");
        return Some(Notice { kind: Kind::Plugins, line, short, updatable: true, restarts: false, url: None });
    }
    None
}

/// A checkout of verifique's own source that this program was built in, as `cargo build` leaves it.
fn source_checkout(program: &Path) -> Option<PathBuf> {
    let target = program.ancestors().skip(1).find(|folder| folder.file_name().is_some_and(|name| name == "target"))?;
    let root = target.parent()?;
    root.join("Cargo.toml").is_file().then(|| root.to_path_buf())
}

/// This program's real file, with links followed, which says how it was installed.
fn this_program() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    Some(platform::canonical(&exe).unwrap_or(exe))
}

fn install_of(program: &Path) -> Install {
    if let Some(root) = source_checkout(program) {
        return Install::Source(root);
    }
    if platform::update_command(program).is_some() {
        Install::Known
    } else {
        Install::Unknown
    }
}

fn install() -> Install {
    this_program().map_or(Install::Unknown, |program| install_of(&program))
}

fn running_queue(store: &Store) -> Result<Queue> {
    Ok(if server::is_running(store)? { Queue::Runs(store.setting(KEY_QUEUE_VERSION)?) } else { Queue::Stopped })
}

fn plugins_behind(store: &Store) -> Result<Vec<String>> {
    Ok(store
        .setting(KEY_PLUGINS_BEHIND)?
        .map(|names| names.split(',').filter(|name| !name.is_empty()).map(str::to_string).collect())
        .unwrap_or_default())
}

/// What, if anything, people should be told about versions: a newer release, a queue that still runs
/// an older verifique than this one, or add-ons behind it. Turning updates off silences only the
/// first, which is the one that comes from the internet.
pub fn notice(store: &Store) -> Result<Option<Notice>> {
    Ok(make(&Facts {
        off: is_off(store)?,
        latest: store.setting(KEY_LATEST)?,
        this: VERSION,
        queue: running_queue(store)?,
        plugins_behind: plugins_behind(store)?,
        install: &install,
    }))
}

/// The notice as one line on stderr, after the output of a command people read. Never with `--json`,
/// never on what agents read, and never a reason for the command to fail.
pub fn print_notice() {
    if !paths::home().join("queue.db").is_file() {
        return;
    }
    if let Ok(Some(notice)) = Store::open().and_then(|store| notice(&store)) {
        eprintln!("\n{}", notice.line);
    }
}

fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => "less than a minute".to_string(),
        60..120 => "a minute".to_string(),
        120..3600 => format!("{} minutes", seconds / 60),
        3600..7200 => "an hour".to_string(),
        _ => format!("{} hours", seconds / 3600),
    }
}

/// `verifique updates`: whether the queue asks, when it last did, what it found, and the notice.
pub fn show(as_json: bool) -> Result<()> {
    let store = Store::open()?;
    let off = is_off(&store)?;
    let checked = store.setting(KEY_CHECKED)?.and_then(|value| value.parse::<u64>().ok());
    let latest = store.setting(KEY_LATEST)?;
    let queue = match running_queue(&store)? {
        Queue::Runs(version) => version,
        Queue::Stopped => None,
    };
    let notice = notice(&store)?;
    if as_json {
        let out = json!({ "checking": !off, "checked": checked, "latest": latest, "this": VERSION, "queue": queue, "notice": notice });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if off {
        println!("off: {NAME} doesn't ask GitHub for new releases. `{NAME} updates on` turns it back on");
    } else {
        println!("on: the queue asks GitHub for the latest release once a day while it runs. `{NAME} updates off` stops it");
        match (checked, latest) {
            (Some(at), Some(latest)) => {
                println!("last asked {} ago: {latest} is the latest, and this is {VERSION}", ago(server::now().saturating_sub(at)))
            }
            _ => println!("not asked yet: the queue asks when it next runs"),
        }
    }
    if let Some(notice) = notice {
        eprintln!("\n{}", notice.line);
        if let Some(url) = &notice.url {
            eprintln!("what it changes: {url}");
        }
    }
    Ok(())
}

/// `verifique updates on` and `off`, which take effect at once, with no restart.
pub fn set(on: bool) -> Result<()> {
    let store = Store::open()?;
    if on {
        store.unset(KEY_OFF)?;
        println!("on: the queue asks GitHub for the latest release once a day, and {NAME} says when a newer one is out");
    } else {
        store.set(KEY_OFF, "1")?;
        store.unset(KEY_LATEST)?;
        store.unset(KEY_CHECKED)?;
        println!("off: {NAME} won't ask GitHub for new releases. `{NAME} updates on` turns it back on");
    }
    Ok(())
}

const REFUSED: &str = "`verifique update` restarts the queue, which ends started stacks, so a person runs it: in a \
    terminal, where it asks first, or from the panel";

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What restarting the queue stops, in words.
fn stopping(store: &Store) -> Result<String> {
    let running = store.running()?;
    let stacks = running.iter().filter(|job| job.plan.hold_minutes.is_some()).count();
    let checks = running.len() - stacks;
    let mut parts = Vec::new();
    if stacks > 0 {
        parts.push(plural(stacks, "started stack stops", "started stacks stop"));
    }
    if checks > 0 {
        parts.push(format!("{} again from the beginning", plural(checks, "running check starts", "running checks start")));
    }
    Ok(if parts.is_empty() { "nothing is running, so nothing stops".to_string() } else { parts.join(", and ") })
}

/// The version a verifique program says it is.
fn version_of(program: &Path) -> Result<String> {
    let out = Command::new(program).arg("--version").output().with_context(|| format!("running {}", program.display()))?;
    let said = String::from_utf8_lossy(&out.stdout);
    said.split_whitespace().nth(1).map(str::to_string).ok_or_else(|| anyhow!("{} gave no version", program.display()))
}

/// One step of the update, run by the verifique that is now installed, which knows its own release.
fn run(program: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new(program).args(args).status().with_context(|| format!("running {}", program.display()))?;
    if !status.success() {
        bail!("`{NAME} {}` failed, so the update stopped there", args.join(" "));
    }
    Ok(())
}

/// `verifique update`: bring verifique, its queue, its add-ons and the panel to the latest release,
/// the way verifique was installed. Restarting the queue ends started stacks, so it asks first in a
/// terminal, and anywhere else goes ahead only with `--yes`, which the panel passes once the person
/// has agreed there.
pub fn update(yes: bool) -> Result<()> {
    let interactive = io::stdin().is_terminal();
    if !yes && !interactive {
        bail!("{REFUSED}");
    }
    let store = Store::open()?;
    let program = this_program().ok_or_else(|| anyhow!("can't tell where this {NAME} is"))?;
    let latest = release::latest()?;
    if !is_off(&store)? {
        store.set(KEY_LATEST, &latest)?;
        store.set(KEY_CHECKED, &server::now().to_string())?;
    }
    let newer = release::is_newer(&latest, VERSION);
    let queue = running_queue(&store)?;
    let restart = queue != Queue::Stopped && (newer || queue.older_than(VERSION).is_some());
    let plugins_behind = !plugins_behind(&store)?.is_empty();
    if !newer && !restart && !plugins_behind {
        println!("{NAME} {VERSION} is the latest, and the queue and its add-ons run it");
        return Ok(());
    }
    if newer {
        match install_of(&program) {
            Install::Source(root) => bail!("this {NAME} was built from source: pull and build again in {}", root.display()),
            Install::Unknown => bail!("{NAME} can't tell how it was installed, so update it the way you installed it, as {GUIDE} says"),
            Install::Known => {}
        }
    }
    if !yes {
        let what = if newer { format!("update {NAME} {VERSION} to {latest}") } else { "finish updating".to_string() };
        let queue = if restart { format!(", and restart the queue: {}", stopping(&store)?) } else { String::new() };
        print!("This will {what}{queue}. Go ahead? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("nothing changed");
            return Ok(());
        }
    }
    let mut program = program;
    if newer {
        println!("updating {NAME} {VERSION} to {latest}");
        let installed = platform::upgrade(&program, &latest, interactive)?
            .ok_or_else(|| anyhow!("{NAME} can't tell how it was installed, so update it the way you installed it, as {GUIDE} says"))?;
        let now = version_of(&installed)?;
        if now != latest {
            bail!("{latest} isn't available that way yet, so {NAME} is still {now}. Try again later");
        }
        program = installed;
    }
    if restart {
        if service::installed() {
            run(&program, &["service", "restart"])?;
        } else {
            println!("the queue runs in a terminal: restart `{NAME} serve` there to run {latest}");
        }
    }
    if plugins::any_installed() && (newer || plugins_behind) {
        run(&program, &["plugin", "update"])?;
    }
    // Last, and on its own: it replaces the panel, which may be the one that asked for this update.
    if newer && platform::panel_installed() {
        let reopen = panel::running().is_some();
        let mut command = Command::new(&program);
        command.args(["panel", "install"]);
        if !reopen {
            command.arg("--no-start");
        }
        platform::spawn_detached(command)?;
        if reopen {
            println!("the panel reopens at {latest} in a moment");
        }
    }
    println!("up to date: {NAME} {}", if newer { latest.as_str() } else { VERSION });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(install: &'a dyn Fn() -> Install) -> Facts<'a> {
        Facts {
            off: false,
            latest: Some("0.3.3".into()),
            this: "0.3.3",
            queue: Queue::Runs(Some("0.3.3".into())),
            plugins_behind: Vec::new(),
            install,
        }
    }

    fn known() -> Install {
        Install::Known
    }

    #[test]
    fn nothing_is_said_while_everything_runs_the_latest() {
        assert!(make(&facts(&known)).is_none());
        let never_asked = Facts { latest: None, ..facts(&known) };
        assert!(make(&never_asked).is_none());
        let ahead = Facts { latest: Some("0.3.2".into()), ..facts(&known) };
        assert!(make(&ahead).is_none());
    }

    #[test]
    fn a_newer_release_is_one_command_away_whatever_installed_it() {
        let notice = make(&Facts { latest: Some("0.3.4".into()), ..facts(&known) }).unwrap();
        assert_eq!(notice.kind, Kind::Release);
        assert_eq!(notice.line, "verifique 0.3.4 is out (this is 0.3.3): run `verifique update`, which restarts the queue");
        assert_eq!(notice.short, "verifique 0.3.4 is out");
        assert!(notice.updatable && notice.restarts);
        assert_eq!(notice.url.as_deref(), Some("https://github.com/smnkgv/verifique/releases/tag/v0.3.4"));
        let alone = make(&Facts { latest: Some("0.3.4".into()), queue: Queue::Stopped, ..facts(&known) }).unwrap();
        assert_eq!(alone.line, "verifique 0.3.4 is out (this is 0.3.3): run `verifique update`");
        assert!(!alone.restarts, "no queue runs, so nothing restarts");
    }

    #[test]
    fn a_build_from_source_or_an_unknown_install_is_updated_by_hand() {
        let source = || Install::Source(PathBuf::from("/src/verifique"));
        let notice = make(&Facts { latest: Some("0.3.4".into()), ..facts(&source) }).unwrap();
        assert_eq!(notice.line, "verifique 0.3.4 is out (this is 0.3.3): pull and build again in /src/verifique");
        assert!(!notice.updatable && !notice.restarts);
        let unknown = || Install::Unknown;
        let notice = make(&Facts { latest: Some("0.3.4".into()), ..facts(&unknown) }).unwrap();
        assert!(notice.line.contains(GUIDE), "{}", notice.line);
        assert!(!notice.updatable);
    }

    #[test]
    fn turning_updates_off_silences_the_release_but_not_what_this_machine_knows() {
        let off = Facts { off: true, latest: Some("0.3.4".into()), ..facts(&known) };
        assert!(make(&off).is_none());
        let off_and_old_queue = Facts { off: true, this: "0.3.4", latest: Some("0.3.4".into()), ..facts(&known) };
        assert_eq!(make(&off_and_old_queue).unwrap().kind, Kind::Restart);
    }

    #[test]
    fn a_queue_older_than_this_verifique_needs_restarting() {
        let updated = Facts { this: "0.3.4", latest: Some("0.3.4".into()), ..facts(&known) };
        let notice = make(&updated).unwrap();
        assert_eq!(notice.kind, Kind::Restart);
        assert_eq!(
            notice.line,
            "the queue still runs 0.3.3, and verifique 0.3.4 is installed: run `verifique update` to finish, which restarts the queue"
        );
        assert_eq!(notice.short, "The queue still runs 0.3.3");
        assert!(notice.updatable && notice.restarts);
        let unrecorded = Facts { queue: Queue::Runs(None), ..updated };
        assert_eq!(make(&unrecorded).unwrap().short, "The queue still runs an older version");
    }

    #[test]
    fn add_ons_behind_the_queue_are_named() {
        let behind = Facts { plugins_behind: vec!["postgres".into(), "redis".into()], ..facts(&known) };
        let notice = make(&behind).unwrap();
        assert_eq!(notice.kind, Kind::Plugins);
        assert_eq!(notice.line, "the postgres and redis add-ons are behind verifique 0.3.3: run `verifique update`");
        assert_eq!(notice.short, "The postgres and redis add-ons are behind 0.3.3");
        assert!(notice.updatable && !notice.restarts);
        let stopped = Facts { plugins_behind: vec!["postgres".into()], queue: Queue::Stopped, ..facts(&known) };
        assert!(make(&stopped).is_none(), "a stopped queue's record may be stale");
    }

    #[test]
    fn a_build_from_source_is_told_by_its_target_folder() {
        let root = env::temp_dir().join(format!("verifique-source-{}", std::process::id()));
        std::fs::create_dir_all(root.join("target/release")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        assert_eq!(source_checkout(&root.join("target/release/verifique")), Some(root.clone()));
        assert_eq!(source_checkout(&root.join("bin/verifique")), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn names_read_as_a_list() {
        assert_eq!(and_list(&["postgres".into()]), "postgres");
        assert_eq!(and_list(&["postgres".into(), "mongo".into(), "redis".into()]), "postgres, mongo and redis");
    }

    #[test]
    fn counts_read_as_words() {
        assert_eq!(plural(1, "started stack stops", "started stacks stop"), "1 started stack stops");
        assert_eq!(plural(2, "started stack stops", "started stacks stop"), "2 started stacks stop");
    }
}
