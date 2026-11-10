//! `verifique panel`: installing the panel from a release and removing it, without opening it, in a
//! home of its own so nothing on this machine changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const BIN: &str = env!("CARGO_BIN_EXE_verifique");

fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "x64" };
    format!("{os}-{arch}")
}

fn file_url(path: &Path) -> String {
    format!("file:///{}", path.display().to_string().replace('\\', "/").trim_start_matches('/'))
}

const PROGRAM: &[u8] = b"a stand-in for the panel";

/// The panel as each OS's release carries it: an app bundle zipped on macOS, a program elsewhere.
fn package(release: &Path) -> String {
    let asset = format!("verifique-panel-{}", target());
    if cfg!(target_os = "macos") {
        let staging = release.join("staging");
        let macos = staging.join("verifique.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("verifique"), PROGRAM).unwrap();
        let zip = format!("{asset}.zip");
        let made = Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(staging.join("verifique.app"))
            .arg(release.join(&zip))
            .status()
            .unwrap();
        assert!(made.success());
        zip
    } else {
        let name = format!("{asset}{}", std::env::consts::EXE_SUFFIX);
        fs::write(release.join(&name), PROGRAM).unwrap();
        name
    }
}

struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-panel-{unique}"));
        let release = root.join("release");
        fs::create_dir_all(&release).unwrap();
        let asset = package(&release);
        let sum: String = Sha256::digest(fs::read(release.join(&asset)).unwrap()).iter().map(|byte| format!("{byte:02x}")).collect();
        fs::write(release.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
        Self { root }
    }

    fn user(&self) -> PathBuf {
        self.root.join("user")
    }

    fn run(&self, args: &[&str]) -> Output {
        let user = self.user();
        Command::new(BIN)
            .args(args)
            .env("VERIFIQUE_RELEASES", file_url(&self.root.join("release")))
            .env("VERIFIQUE_HOME", self.root.join("queue"))
            .env("HOME", &user)
            .env("XDG_DATA_HOME", user.join("data"))
            .env("XDG_CONFIG_HOME", user.join("config"))
            .env("USERPROFILE", &user)
            .env("LOCALAPPDATA", user.join("local"))
            .env("APPDATA", user.join("roaming"))
            .output()
            .unwrap()
    }

    /// Where each OS's install puts the panel's program, and its app menu entry where it has one.
    fn installed(&self) -> Vec<PathBuf> {
        let user = self.user();
        if cfg!(target_os = "macos") {
            vec![user.join("Applications/verifique.app/Contents/MacOS/verifique")]
        } else if cfg!(windows) {
            vec![
                user.join(r"local\Programs\verifique\verifique-panel.exe"),
                user.join(r"roaming\Microsoft\Windows\Start Menu\Programs\verifique.lnk"),
            ]
        } else {
            vec![user.join(".local/bin/verifique-panel"), user.join("data/applications/verifique-panel.desktop")]
        }
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_panel_is_installed_from_the_release_and_removed_again() {
    let machine = Machine::new();
    let installed = machine.run(&["panel", "install", "--no-start"]);
    assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));
    let places = machine.installed();
    for place in &places {
        assert!(place.exists(), "{} is missing", place.display());
    }
    assert_eq!(fs::read(&places[0]).unwrap(), PROGRAM);

    let again = machine.run(&["panel", "install", "--no-start"]);
    assert!(again.status.success(), "installing over it: {}", String::from_utf8_lossy(&again.stderr));

    let removed = machine.run(&["panel", "remove"]);
    assert!(removed.status.success(), "{}", String::from_utf8_lossy(&removed.stderr));
    for place in &places {
        assert!(!place.exists(), "{} is still there", place.display());
    }
}
