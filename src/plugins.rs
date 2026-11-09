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
use sha2::{Digest, Sha256};
use verifique_plugin::{Description, Reply, DESCRIBE, PROGRAM_PREFIX, PROTOCOL};

use crate::paths::{self, NAME};
use crate::platform;

/// The plugins verifique's own releases carry.
pub const PUBLISHED: [&str; 3] = ["postgres", "mongo", "redis"];

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/smnkgv/verifique/releases/download";
const CHECKSUMS: &str = "SHA256SUMS";

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

/// The plugins folder first, then the folder holding verifique itself, where a build from source
/// leaves its plugins, then PATH.
pub fn find(name: &str) -> Option<PathBuf> {
    let program = program(name);
    let names = platform::program_file_names(&program);
    // Through a symlink, such as ~/.local/bin/verifique, the plugins are beside the real binary.
    let exe = std::env::current_exe().ok();
    let real = exe.as_deref().and_then(|exe| fs::canonicalize(exe).ok());
    let own: Vec<PathBuf> = exe.into_iter().chain(real).filter_map(|exe| exe.parent().map(Path::to_path_buf)).collect();
    std::iter::once(dir())
        .chain(own)
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
        let mut child = Command::new(&self.program)
            .arg(operation)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("running {}", self.program.display()))?;
        let body = serde_json::to_vec(request)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&body)?;
        }
        let out = child.wait_with_output()?;
        let failed = || anyhow!("{} {operation} failed: {}", self.program.display(), String::from_utf8_lossy(&out.stderr).trim());
        let reply: Reply = serde_json::from_slice(&out.stdout).map_err(|_| failed())?;
        match (&reply.error, out.status.success()) {
            (Some(error), _) => bail!("{error}"),
            (None, true) => Ok(reply),
            (None, false) => Err(failed()),
        }
    }

    /// A line for people: the plugin's version, and what to do when it isn't verifique's.
    pub fn version_note(&self) -> String {
        let version = &self.description.version;
        if version == VERSION {
            format!("{version} at {}", self.program.display())
        } else {
            format!(
                "{version} at {}, while {NAME} is {VERSION}: run `{NAME} plugin update`",
                self.program.display()
            )
        }
    }
}

/// This machine as release files name it, such as `macos-arm64` or `windows-x64`.
fn target() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => bail!("{NAME} publishes no plugins for {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => bail!("{NAME} publishes no plugins for {other} processors"),
    };
    Ok(format!("{os}-{arch}"))
}

fn release() -> String {
    std::env::var("VERIFIQUE_RELEASES").unwrap_or_else(|_| format!("{RELEASES}/v{VERSION}"))
}

/// curl ships with macOS, every common Linux and Windows 10 and later.
fn download(url: &str, to: &Path) -> Result<()> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .context("running curl, which downloads plugins")?;
    if !out.status.success() {
        bail!("downloading {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Fetch each plugin from the release verifique itself came from, check it against the release's
/// checksums, and put it in the plugins folder, replacing any older one.
pub fn add(names: &[String]) -> Result<()> {
    let folder = dir();
    fs::create_dir_all(&folder)?;
    let release = release();
    let sums_file = folder.join(format!(".{CHECKSUMS}.part"));
    download(&format!("{release}/{CHECKSUMS}"), &sums_file)
        .with_context(|| format!("{NAME} {VERSION} has no published plugins to fetch"))?;
    let sums = fs::read_to_string(&sums_file)?;
    let _ = fs::remove_file(&sums_file);
    let expected: BTreeMap<&str, &str> = sums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .map(|(sum, file)| (file.trim().trim_start_matches('*'), sum.trim()))
        .collect();
    let target = target()?;
    for name in names {
        let asset = format!("{}-{target}{}", program(name), std::env::consts::EXE_SUFFIX);
        let Some(sum) = expected.get(asset.as_str()) else {
            let published = PUBLISHED.join(", ");
            bail!("{NAME} {VERSION} publishes no plugin called {name} for {target}. It publishes {published}");
        };
        let part = folder.join(format!(".{asset}.part"));
        download(&format!("{release}/{asset}"), &part)?;
        let got = hex(&Sha256::digest(fs::read(&part)?));
        if got != *sum {
            let _ = fs::remove_file(&part);
            bail!("{asset} doesn't match the release's checksum, so it was not installed");
        }
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
    Ok(())
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

/// Bring every installed plugin to verifique's own version.
pub fn update() -> Result<()> {
    let names = installed();
    if names.is_empty() {
        println!("no plugins installed in {}", dir().display());
        return Ok(());
    }
    add(&names)
}

pub fn remove(names: &[String]) -> Result<()> {
    for name in names {
        let file = installed_file(name);
        if file.is_file() {
            fs::remove_file(&file)?;
            println!("removed {name}");
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
