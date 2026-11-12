//! Add-ons: separate programs called `verifique-<name>` that verifique finds and talks to through
//! the protocol in `plugins/kit`. People install only the ones their projects use, with
//! `verifique plugin add`, which fetches the build from the same release as verifique itself.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use verifique_plugin::{Description, Reply, DESCRIBE, PROGRAM_PREFIX, PROTOCOL};

use crate::paths::{self, NAME};
use crate::platform;
use crate::release::{self, Release, VERSION};

/// The plugins verifique's own releases carry.
pub const PUBLISHED: [&str; 3] = ["postgres", "mongo", "redis"];


/// Where `plugin add` puts plugins: one folder per user, whichever queue they serve.
pub fn dir() -> PathBuf {
    std::env::var_os("VERIFIQUE_PLUGINS")
        .map_or_else(|| paths::user_home().join(format!(".{NAME}")).join("plugins"), PathBuf::from)
}

fn program(name: &str) -> String {
    format!("{PROGRAM_PREFIX}{name}")
}

/// The file `plugin add` writes for `name`.
fn installed_file(name: &str) -> PathBuf {
    dir().join(format!("{}{}", program(name), std::env::consts::EXE_SUFFIX))
}

/// The folder holding verifique itself first, where a build leaves the plugins built with it, then
/// the plugins folder, then PATH.
pub fn find(name: &str) -> Option<PathBuf> {
    let program = program(name);
    let names = platform::program_file_names(&program);
    // Through a symlink, such as ~/.local/bin/verifique, the plugins are beside the real binary.
    let exe = std::env::current_exe().ok();
    let real = exe.as_deref().and_then(|exe| fs::canonicalize(exe).ok());
    let own: Vec<PathBuf> = exe.into_iter().chain(real).filter_map(|exe| exe.parent().map(Path::to_path_buf)).collect();
    own.into_iter()
        .chain(std::iter::once(dir()))
        .find_map(|folder| names.iter().map(|file| folder.join(file)).find(|file| file.is_file()))
        .or_else(|| platform::find_program(&program, &std::env::var_os("PATH").unwrap_or_default()))
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub program: PathBuf,
    pub description: Description,
}

fn describe(program: &Path) -> Result<Description> {
    let out = Command::new(program)
        .arg(DESCRIBE)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {}", program.display()))?;
    serde_json::from_slice(&out.stdout).with_context(|| format!("{} is not a {NAME} plugin", program.display()))
}

/// The plugin called `name`, checked to speak this verifique's protocol. Asked once per process.
pub fn load(name: &str) -> Result<Plugin> {
    static LOADED: OnceLock<Mutex<BTreeMap<String, Plugin>>> = OnceLock::new();
    let loaded = LOADED.get_or_init(Default::default);
    if let Some(plugin) = loaded.lock().map_err(|_| anyhow!("plugin cache poisoned"))?.get(name) {
        return Ok(plugin.clone());
    }
    let program = find(name).ok_or_else(|| {
        anyhow!("{name} needs the {} plugin, which isn't installed. Run `{NAME} plugin add {name}`", program(name))
    })?;
    let description = describe(&program)?;
    if description.protocol != PROTOCOL {
        bail!(
            "{} speaks plugin protocol {}, and this {NAME} speaks {PROTOCOL}. Run `{NAME} plugin add {name}` for the one that matches",
            program.display(),
            description.protocol
        );
    }
    let plugin = Plugin { program, description };
    loaded.lock().map_err(|_| anyhow!("plugin cache poisoned"))?.insert(name.to_string(), plugin.clone());
    Ok(plugin)
}

impl Plugin {
    /// One operation, answered in full. The plugin's error is already a sentence for the user.
    pub fn call(&self, operation: &str, request: &impl Serialize) -> Result<Reply> {
        // The request, a few hundred bytes, goes into the pipe before the plugin starts, so a plugin
        // that never reads it can't break the pipe under verifique.
        let (stdin, mut writer) = std::io::pipe()?;
        writer.write_all(&serde_json::to_vec(request)?)?;
        drop(writer);
        let out = Command::new(&self.program)
            .arg(operation)
            .stdin(stdin)
            .output()
            .with_context(|| format!("running {}", self.program.display()))?;
        let failed = || anyhow!("{} {operation} failed: {}", self.program.display(), String::from_utf8_lossy(&out.stderr).trim());
        let reply: Reply = serde_json::from_slice(&out.stdout).map_err(|_| failed())?;
        match (&reply.error, out.status.success()) {
            (Some(error), _) => bail!("{error}"),
            (None, true) => Ok(reply),
            (None, false) => Err(failed()),
        }
    }

    /// A line for people: the plugin's version, and what to do when one verifique publishes isn't
    /// verifique's own.
    pub fn version_note(&self) -> String {
        let version = &self.description.version;
        if version == VERSION || !PUBLISHED.contains(&self.description.name.as_str()) {
            format!("{version} at {}", self.program.display())
        } else {
            format!(
                "{version} at {}, while {NAME} is {VERSION}: run `{NAME} plugin update`",
                self.program.display()
            )
        }
    }
}

/// Fetch each plugin from the release verifique itself came from, check it against the release's
/// checksums, and put it in the plugins folder, replacing any older one.
pub fn add(names: &[String]) -> Result<()> {
    let folder = dir();
    let release = Release::open(&folder)?;
    let target = release::target()?;
    for name in names {
        let asset = format!("{}-{target}{}", program(name), std::env::consts::EXE_SUFFIX);
        if !release.has(&asset) {
            let published = PUBLISHED.join(", ");
            bail!("{NAME} {VERSION} publishes no plugin called {name} for {target}. It publishes {published}");
        }
        let part = folder.join(format!(".{asset}.part"));
        release.fetch(&asset, &part)?;
        platform::make_executable(&part)?;
        let file = installed_file(name);
        // A program that is running can't be replaced on Windows, but it can be moved aside.
        let aside = file.with_extension("old");
        let _ = fs::remove_file(&aside);
        if file.exists() {
            fs::rename(&file, &aside)?;
        }
        fs::rename(&part, &file)?;
        let _ = fs::remove_file(&aside);
        println!("added {name} {VERSION}");
    }
    record();
    Ok(())
}

/// Tell the queue which add-ons are behind now, so its reminder goes away once they aren't.
fn record() {
    if let Ok(store) = crate::store::Store::open() {
        crate::updates::record_plugins(&store);
    }
}

/// The plugins `plugin add` installed, by name.
fn installed() -> Vec<String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut names: Vec<String> = fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let name = file.strip_prefix(PROGRAM_PREFIX)?.strip_suffix(suffix)?.to_string();
            (!name.is_empty() && !name.contains('.')).then_some(name)
        })
        .collect();
    names.sort();
    names
}

/// Bring every installed plugin verifique publishes to its own version. Others came from
/// somewhere else, so they are left as they are.
pub fn update() -> Result<()> {
    let (names, others): (Vec<String>, Vec<String>) = installed().into_iter().partition(|name| PUBLISHED.contains(&name.as_str()));
    for name in &others {
        println!("leaving {name} alone: {NAME} doesn't publish it");
    }
    if names.is_empty() {
        println!("no {NAME} plugins installed in {}", dir().display());
        return Ok(());
    }
    add(&names)
}

/// The add-ons verifique publishes that the queue would use at another version than this one,
/// by name. Each is asked its version, so this runs a program per add-on found.
pub fn behind() -> Vec<String> {
    PUBLISHED
        .iter()
        .filter(|name| find(name).and_then(|program| describe(&program).ok()).is_some_and(|description| description.version != VERSION))
        .map(|name| name.to_string())
        .collect()
}

/// Whether `plugin add` has installed any add-on, which `plugin update` would then bring along.
pub fn any_installed() -> bool {
    !installed().is_empty()
}

pub fn remove(names: &[String]) -> Result<()> {
    for name in names {
        let file = installed_file(name);
        if file.is_file() {
            fs::remove_file(&file)?;
            println!("removed {name}");
            record();
        } else if let Some(elsewhere) = find(name) {
            bail!("{name} is at {}, which `{NAME} plugin add` didn't put there. Remove it there", elsewhere.display());
        } else {
            bail!("{name} is not installed");
        }
    }
    Ok(())
}

pub fn list() -> Result<()> {
    let mut names: Vec<String> = PUBLISHED.iter().map(|name| name.to_string()).collect();
    for name in installed() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    for name in names {
        match find(name.as_str()).map(|program| describe(&program).map(|description| Plugin { program, description })) {
            Some(Ok(plugin)) => println!("{name}: {}", plugin.version_note()),
            Some(Err(error)) => println!("{name}: {error:#}"),
            None => println!("{name}: not installed (`{NAME} plugin add {name}`)"),
        }
    }
    Ok(())
}
