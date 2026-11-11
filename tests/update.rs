//! `verifique update` on a copy installed the way the install scripts install it, from a release
//! made here, in a home of its own so nothing on this machine changes.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const BIN: &str = env!("CARGO_BIN_EXE_verifique");
const THIS: &str = env!("CARGO_PKG_VERSION");

fn target() -> String {
    let os = if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "x64" };
    format!("{os}-{arch}")
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

struct Machine {
    root: PathBuf,
}

impl Machine {
    /// This verifique, installed as install.sh would, and a release of `published` whose program
    /// says it is `says`.
    fn new(published: &str, says: &str) -> Self {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-update-{unique}-{says}"));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::copy(BIN, root.join("bin/verifique")).unwrap();
        let staging = root.join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("verifique"), format!("#!/bin/sh\necho 'verifique {says}'\n")).unwrap();
        fs::set_permissions(staging.join("verifique"), fs::Permissions::from_mode(0o755)).unwrap();
        let release = root.join("release");
        fs::create_dir_all(&release).unwrap();
        let asset = format!("verifique-{}.tar.gz", target());
        let packed = Command::new("tar").arg("-czf").arg(release.join(&asset)).arg("-C").arg(&staging).arg("verifique").status().unwrap();
        assert!(packed.success());
        let sum: String = Sha256::digest(fs::read(release.join(&asset)).unwrap()).iter().map(|byte| format!("{byte:02x}")).collect();
        fs::write(release.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
        fs::write(root.join("latest.json"), format!(r#"{{"tag_name": "v{published}"}}"#)).unwrap();
        Self { root }
    }

    fn installed(&self) -> PathBuf {
        self.root.join("bin/verifique")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(self.installed())
            .args(args)
            .env("VERIFIQUE_LATEST", file_url(&self.root.join("latest.json")))
            .env("VERIFIQUE_RELEASES", file_url(&self.root.join("release")))
            .env("VERIFIQUE_HOME", self.root.join("queue"))
            .env("VERIFIQUE_PLUGINS", self.root.join("plugins"))
            // No panel or service of this machine's own is in sight.
            .env("HOME", self.root.join("user"))
            .output()
            .unwrap()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_person_asks_for_an_update_and_an_agent_cannot() {
    let machine = Machine::new("99.0.0", "99.0.0");
    let refused = machine.run(&["update"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("so a person runs it"), "{}", String::from_utf8_lossy(&refused.stderr));
    assert_eq!(fs::read(machine.installed()).unwrap(), fs::read(BIN).unwrap(), "nothing changed");
}

#[test]
fn an_install_script_copy_is_swapped_for_the_new_release() {
    let machine = Machine::new("99.0.0", "99.0.0");
    let out = machine.run(&["update", "--yes"]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}{}", String::from_utf8_lossy(&out.stderr));
    assert!(said.contains(&format!("updating verifique {THIS} to 99.0.0")), "{said}");
    assert!(said.contains("up to date: verifique 99.0.0"), "{said}");
    let now = Command::new(machine.installed()).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&now.stdout).trim(), "verifique 99.0.0");
    let left: Vec<_> = fs::read_dir(machine.root.join("bin")).unwrap().flatten().map(|entry| entry.file_name()).collect();
    assert_eq!(left, ["verifique"], "nothing of the update is left behind");
}

#[test]
fn a_release_the_installer_does_not_have_yet_changes_nothing_it_claims() {
    // The release's files say they are this version: a package manager that hasn't caught up.
    let machine = Machine::new("99.0.0", THIS);
    let out = machine.run(&["update", "--yes"]);
    assert!(!out.status.success());
    let told = String::from_utf8_lossy(&out.stderr);
    assert!(told.contains("99.0.0 isn't available that way yet"), "{told}");
}

#[test]
fn nothing_to_do_says_so() {
    let machine = Machine::new(THIS, THIS);
    let out = machine.run(&["update", "--yes"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("verifique {THIS} is the latest")));
    assert_eq!(fs::read(machine.installed()).unwrap(), fs::read(BIN).unwrap());
}
