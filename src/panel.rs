//! The status-bar panel, an add-on on top of the CLI. `verifique panel install` fetches the build
//! for this machine from the release this verifique came from, checks it against the release's
//! checksums and puts it where this OS keeps apps. The panel only reads the CLI's JSON and acts
//! through the CLI, so it is the same version as the CLI that installed it.

use std::fs::{self, File, TryLockError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::paths::{self, NAME};
use crate::platform;
use crate::release::{self, Release, VERSION};

pub fn install(start: bool) -> Result<()> {
    let scratch = paths::home().join("tmp");
    let release = Release::open(&scratch)?;
    let target = release::target()?;
    let asset = format!("{NAME}-panel-{target}{}", platform::PANEL_PACKAGE);
    if !release.has(&asset) {
        bail!("{NAME} {VERSION} publishes no panel for {target}");
    }
    let part = scratch.join(format!(".{asset}.part"));
    release.fetch(&asset, &part)?;
    stop_running();
    let placed = platform::install_panel(&part, start);
    let _ = fs::remove_file(&part);
    println!("installed the panel {VERSION} in {}", placed?.display());
    if start {
        println!("it opens now, and whenever you log in");
    }
    Ok(())
}

/// The panel this queue's home runs, by the process id it wrote, while it holds its lock: an id on
/// its own may long since belong to another program.
pub fn running() -> Option<u32> {
    let lock = File::options().read(true).write(true).open(paths::home().join("panel.lock")).ok()?;
    if !matches!(lock.try_lock(), Err(TryLockError::WouldBlock)) {
        return None;
    }
    fs::read_to_string(paths::home().join("panel.pid")).ok()?.trim().parse().ok()
}

/// One panel runs per queue, so a new one would quit at once beside the old. The old one goes first.
fn stop_running() {
    let Some(pid) = running() else { return };
    platform::stop_process(pid, Duration::from_secs(3));
    let started = Instant::now();
    while running().is_some() && started.elapsed() < Duration::from_secs(5) {
        sleep(Duration::from_millis(100));
    }
}

pub fn remove() -> Result<()> {
    platform::remove_panel()?;
    println!("removed the panel");
    Ok(())
}
