//! The release this verifique came from: the files published with this version, each checked against
//! the release's checksums before anything is installed from it. Plugins and the panel come from here.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::paths::NAME;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/smnkgv/verifique/releases/download";
const CHECKSUMS: &str = "SHA256SUMS";

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
        let base = std::env::var("VERIFIQUE_RELEASES").unwrap_or_else(|_| format!("{RELEASES}/v{VERSION}"));
        fs::create_dir_all(scratch)?;
        let file = scratch.join(format!(".{CHECKSUMS}.part"));
        download(&format!("{base}/{CHECKSUMS}"), &file).with_context(|| format!("{NAME} {VERSION} has no published release to fetch from"))?;
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
            bail!("{NAME} {VERSION} publishes no {asset}");
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
