//! The status-bar panel, an add-on on top of the CLI. `verifique panel install` fetches the build
//! for this machine from the release this verifique came from, checks it against the release's
//! checksums and puts it where this OS keeps apps. The panel only reads the CLI's JSON and acts
//! through the CLI, so it is the same version as the CLI that installed it.

use std::fs;

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
    let placed = platform::install_panel(&part, start);
    let _ = fs::remove_file(&part);
    println!("installed the panel {VERSION} in {}", placed?.display());
    if start {
        println!("it opens now, and whenever you log in");
    }
    Ok(())
}

pub fn remove() -> Result<()> {
    platform::remove_panel()?;
    println!("removed the panel");
    Ok(())
}
