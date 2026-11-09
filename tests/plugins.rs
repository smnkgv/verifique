//! `verifique plugin`: fetching add-ons from a release, checked against its checksums.

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

struct Release {
    root: PathBuf,
}

impl Release {
    /// A release holding one plugin, `redis`, whose checksum is listed as `listed` says.
    fn new(listed_correctly: bool) -> Self {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-plugins-{unique}-{listed_correctly}"));
        let release = root.join("release");
        fs::create_dir_all(&release).unwrap();
        let asset = format!("verifique-redis-{}{}", target(), std::env::consts::EXE_SUFFIX);
        let body = b"a stand-in for the plugin";
        fs::write(release.join(&asset), body).unwrap();
        let sum: String = Sha256::digest(if listed_correctly { &body[..] } else { b"something else" })
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::write(release.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
        Self { root }
    }

    fn plugins(&self) -> PathBuf {
        self.root.join("plugins")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .env("VERIFIQUE_RELEASES", file_url(&self.root.join("release")))
            .env("VERIFIQUE_PLUGINS", self.plugins())
            .env("VERIFIQUE_HOME", self.root.join("home"))
            .output()
            .unwrap()
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_plugin_is_added_from_the_release_and_removed_again() {
    let release = Release::new(true);
    let added = release.run(&["plugin", "add", "redis"]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let file = release.plugins().join(format!("verifique-redis{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(fs::read(&file).unwrap(), b"a stand-in for the plugin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o111, 0o111);
    }
    let again = release.run(&["plugin", "add", "redis"]);
    assert!(again.status.success(), "adding twice replaces: {}", String::from_utf8_lossy(&again.stderr));

    let removed = release.run(&["plugin", "remove", "redis"]);
    assert!(removed.status.success());
    assert!(!file.exists());
}

#[test]
fn a_plugin_that_doesnt_match_its_checksum_is_not_installed() {
    let release = Release::new(false);
    let added = release.run(&["plugin", "add", "redis"]);
    assert!(!added.status.success());
    assert!(String::from_utf8_lossy(&added.stderr).contains("doesn't match the release's checksum"));
    assert!(fs::read_dir(release.plugins()).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().starts_with("verifique-redis")));
}

#[test]
fn a_plugin_the_release_lacks_is_named_with_those_it_has() {
    let release = Release::new(true);
    let added = release.run(&["plugin", "add", "cassandra"]);
    assert!(!added.status.success());
    let said = String::from_utf8_lossy(&added.stderr);
    assert!(said.contains("no plugin called cassandra") && said.contains("postgres, mongo, redis"), "{said}");
}
