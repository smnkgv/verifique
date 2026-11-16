//! The real logon service on macOS, in a home of its own and with a user folder of its own, so the
//! machine's own queue is never touched. It loads a LaunchAgent into this user's session, so it runs
//! only when asked: `cargo test --test service -- --ignored`.

#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_verifique");

fn ok(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap();
    assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn uid() -> String {
    ok("id", &["-u"]).trim().to_string()
}

struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("verifique-service-{unique}-{name}"));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("user")).unwrap();
        let machine = Self { root };
        machine.replace_program(&fs::read(BIN).unwrap());
        machine
    }

    fn program(&self) -> PathBuf {
        self.root.join("bin/verifique")
    }

    /// As an update replaces it: a new file renamed over the old one.
    fn replace_program(&self, contents: &[u8]) {
        let next = self.root.join("bin/verifique.next");
        fs::write(&next, contents).unwrap();
        fs::set_permissions(&next, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(&next, self.program()).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(self.program())
            .args(args)
            .env("VERIFIQUE_HOME", self.root.join("queue"))
            .env("VERIFIQUE_NOTIFY", "off")
            .env("HOME", self.root.join("user"))
            .output()
            .unwrap()
    }

    fn serving(&self) -> bool {
        String::from_utf8_lossy(&self.run(&["status", "--json"]).stdout).contains(r#""server": true"#)
    }

    fn wait_until_serving(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.serving() {
            assert!(Instant::now() < deadline, "the service never came up");
            sleep(Duration::from_millis(250));
        }
    }

    fn plist(&self) -> PathBuf {
        let agents = self.root.join("user/Library/LaunchAgents");
        let mut plists: Vec<_> = fs::read_dir(agents).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(plists.len(), 1, "one LaunchAgent: {plists:?}");
        plists.remove(0)
    }

    fn label(&self) -> String {
        ok("plutil", &["-extract", "Label", "raw", &self.plist().to_string_lossy()]).trim().to_string()
    }

    /// What launchd does to a job it loads at login: hold it to the program it found then. Here the
    /// job says so itself, and the file on disk is put back as verifique wrote it.
    fn pin_program_in_launchd(&self) {
        let plist = self.plist();
        let written = fs::read(&plist).unwrap();
        let path = plist.to_string_lossy();
        let cdhash = ok("sh", &["-c", &format!("printf %s {} | xxd -r -p | base64", cdhash(&self.program()))]);
        ok("plutil", &["-insert", "SpawnConstraint", "-dictionary", &path]);
        ok("plutil", &["-insert", "SpawnConstraint.cdhash", "-data", cdhash.trim(), &path]);
        let job = format!("gui/{}/{}", uid(), self.label());
        ok("launchctl", &["bootout", &job]);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !Command::new("launchctl").args(["bootstrap", &format!("gui/{}", uid()), &path]).output().unwrap().status.success() {
            assert!(Instant::now() < deadline, "launchd never took the pinned job back");
            sleep(Duration::from_millis(500));
        }
        assert!(ok("launchctl", &["print", &job]).contains("has spawn constraint"));
        fs::write(&plist, written).unwrap();
        self.wait_until_serving();
    }
}

/// What launchd recognises a program by.
fn cdhash(program: &Path) -> String {
    let out = Command::new("codesign").arg("-dvvv").arg(program).output().unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    said.lines().find_map(|line| line.strip_prefix("CDHash=")).expect("the program is signed").to_string()
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.run(&["service", "uninstall"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn said(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
#[ignore = "loads a LaunchAgent into this user's session"]
fn a_restart_brings_the_queue_back_after_an_update_replaced_its_program() {
    let machine = Machine::new("updated");
    assert!(machine.run(&["updates", "off"]).status.success());
    let installed = machine.run(&["service", "install", "--slots", "1", "--first-slot", "60", "--min-free-gb", "0"]);
    assert!(installed.status.success(), "{}", said(&installed));
    machine.wait_until_serving();

    machine.pin_program_in_launchd();
    // Signed again, so launchd sees a different program, as it does after an update.
    let update = machine.root.join("update");
    fs::copy(BIN, &update).unwrap();
    ok("codesign", &["-f", "-s", "-", &update.to_string_lossy()]);
    assert_ne!(cdhash(&update), cdhash(&machine.program()));
    machine.replace_program(&fs::read(&update).unwrap());
    let restarted = machine.run(&["service", "restart"]);
    assert!(restarted.status.success(), "{}", said(&restarted));
    assert_eq!(String::from_utf8_lossy(&restarted.stdout).trim(), "restarted");
    assert!(machine.serving(), "the queue is back");
}

#[test]
#[ignore = "loads a LaunchAgent into this user's session"]
fn a_restart_says_so_when_the_queue_does_not_come_back() {
    let machine = Machine::new("unstartable");
    assert!(machine.run(&["updates", "off"]).status.success());
    let installed = machine.run(&["service", "install", "--slots", "1", "--first-slot", "62", "--min-free-gb", "0"]);
    assert!(installed.status.success(), "{}", said(&installed));
    machine.wait_until_serving();

    let program = fs::read(machine.program()).unwrap();
    machine.replace_program(b"#!/bin/sh\nexit 1\n");
    let restarted = Command::new(BIN)
        .args(["service", "restart"])
        .env("VERIFIQUE_HOME", machine.root.join("queue"))
        .env("HOME", machine.root.join("user"))
        .output()
        .unwrap();
    machine.replace_program(&program);
    assert_eq!(restarted.status.code(), Some(64), "{}", said(&restarted));
    assert!(!String::from_utf8_lossy(&restarted.stdout).contains("restarted"));
    assert!(said(&restarted).contains("did not come back"), "{}", said(&restarted));
}
