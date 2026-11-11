//! The release this verifique came from: the files published with this version, each checked against
//! the release's checksums before anything is installed from it. Plugins and the panel come from here.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::paths::NAME;
use crate::platform;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/smnkgv/verifique/releases/download";
const CHECKSUMS: &str = "SHA256SUMS";
const LATEST: &str = "https://api.github.com/repos/smnkgv/verifique/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/smnkgv/verifique/releases/tag";

/// This machine as release files name it, such as `macos-arm64` or `windows-x64`.
pub fn target() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => bail!("{NAME} publishes nothing for {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => bail!("{NAME} publishes nothing for {other} processors"),
    };
    Ok(format!("{os}-{arch}"))
}

/// curl ships with macOS, every common Linux and Windows 10 and later. Files it fetches aren't
/// marked as downloaded from the internet, as a browser's are.
fn download(url: &str, to: &Path) -> Result<()> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .context("running curl, which downloads from the release")?;
    if !out.status.success() {
        bail!("downloading {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// The version of the newest published release, asked of GitHub, where the releases are. GitHub
/// leaves out drafts and pre-releases, so a release counts only once it is out.
/// `VERIFIQUE_LATEST` points elsewhere, such as at a file, for tests.
pub fn latest() -> Result<String> {
    let url = std::env::var("VERIFIQUE_LATEST").unwrap_or_else(|_| LATEST.to_string());
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "-H", "Accept: application/vnd.github+json"])
        .arg(&url)
        .output()
        .context("running curl, which asks GitHub for the latest release")?;
    if !out.status.success() {
        bail!("asking {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let release: serde_json::Value = serde_json::from_slice(&out.stdout).with_context(|| format!("reading {url}"))?;
    let tag = release["tag_name"].as_str().ok_or_else(|| anyhow!("{url} names no release"))?;
    let version = tag.trim_start_matches('v');
    if parse(version).is_none() {
        bail!("{url} names {tag}, which isn't a version");
    }
    Ok(version.to_string())
}

/// Where people read what a release changed.
pub fn page(version: &str) -> String {
    format!("{RELEASE_PAGE}/v{version}")
}

/// `0.3.10` as `[0, 3, 10]`, so versions compare by number, not by text.
fn parse(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

/// Whether `version` comes after `than`. Anything that isn't a version never does.
pub fn is_newer(version: &str, than: &str) -> bool {
    match (parse(version), parse(than)) {
        (Some(version), Some(than)) => version > than,
        _ => false,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct Release {
    base: String,
    sums: BTreeMap<String, String>,
}

impl Release {
    /// The release of this version, with its checksums, which are fetched into `scratch`.
    pub fn open(scratch: &Path) -> Result<Self> {
        Self::of(VERSION, scratch)
    }

    /// The release of `version`. `VERIFIQUE_RELEASES` points at another one, such as a folder, for tests.
    fn of(version: &str, scratch: &Path) -> Result<Self> {
        let base = std::env::var("VERIFIQUE_RELEASES").unwrap_or_else(|_| format!("{RELEASES}/v{version}"));
        fs::create_dir_all(scratch)?;
        let file = scratch.join(format!(".{CHECKSUMS}.part"));
        download(&format!("{base}/{CHECKSUMS}"), &file).with_context(|| format!("{NAME} {version} has no published release to fetch from"))?;
        let text = fs::read_to_string(&file)?;
        let _ = fs::remove_file(&file);
        let sums = text
            .lines()
            .filter_map(|line| line.split_once(char::is_whitespace))
            .map(|(sum, name)| (name.trim().trim_start_matches('*').to_string(), sum.trim().to_string()))
            .collect();
        Ok(Self { base, sums })
    }

    pub fn has(&self, asset: &str) -> bool {
        self.sums.contains_key(asset)
    }

    /// Fetch `asset` to `to`, which is left behind only when it matches the release's checksum.
    pub fn fetch(&self, asset: &str, to: &Path) -> Result<()> {
        let Some(expected) = self.sums.get(asset) else {
            bail!("the release publishes no {asset}");
        };
        download(&format!("{}/{asset}", self.base), to)?;
        let got = hex(&Sha256::digest(fs::read(to)?));
        if &got != expected {
            let _ = fs::remove_file(to);
            bail!("{asset} doesn't match the release's checksum, so it was not installed");
        }
        Ok(())
    }
}

/// Put `version` in place of the program at `program`, as the install scripts do: this machine's
/// archive from that release, checked against its checksums, unpacked beside the program and moved
/// into place file by file. A file in use is moved aside first, which Windows allows where it won't
/// let a running program be replaced. Returns the program, now at `version`.
pub fn install_over(program: &Path, version: &str) -> Result<std::path::PathBuf> {
    let folder = program.parent().ok_or_else(|| anyhow!("{} is in no folder", program.display()))?;
    // Beside the program, so moving into place stays on one disk.
    let scratch = folder.join(format!(".{NAME}-update"));
    let _ = fs::remove_dir_all(&scratch);
    let release = Release::of(version, &scratch)?;
    let asset = format!("{NAME}-{}{}", target()?, platform::RELEASE_ARCHIVE);
    let archive = scratch.join(&asset);
    release.fetch(&asset, &archive)?;
    let unpacked = scratch.join("unpacked");
    fs::create_dir_all(&unpacked)?;
    let out = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&unpacked)
        .output()
        .context("running tar, which unpacks the release")?;
    if !out.status.success() {
        bail!("unpacking {asset}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    for entry in fs::read_dir(&unpacked)? {
        let entry = entry?;
        let file = folder.join(entry.file_name());
        let aside = file.with_extension("old");
        let _ = fs::remove_file(&aside);
        if file.exists() {
            fs::rename(&file, &aside).with_context(|| format!("moving {} aside", file.display()))?;
        }
        fs::rename(entry.path(), &file).with_context(|| format!("putting {} in place", file.display()))?;
        platform::make_executable(&file)?;
        // Still running on Windows, so it goes the next time.
        let _ = fs::remove_file(&aside);
    }
    let _ = fs::remove_dir_all(&scratch);
    Ok(program.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.3.4", "0.3.3"));
        assert!(is_newer("0.3.10", "0.3.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.3.3", "0.3.3"));
        assert!(!is_newer("0.3.2", "0.3.3"));
        assert!(!is_newer("0.4.0-beta", "0.3.3"));
        assert!(!is_newer("", "0.3.3"));
    }
}
