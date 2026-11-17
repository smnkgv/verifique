use std::{env, fs};

fn main() {
    // The panel ships in verifique's releases, so it carries their version, which the root
    // Cargo.toml holds. Tauri stamps it on the Windows program.
    let version = release_version(&fs::read_to_string("../Cargo.toml").expect("reading verifique's Cargo.toml"));
    println!("cargo:rerun-if-changed=../Cargo.toml");
    env::set_var("TAURI_CONFIG", format!(r#"{{"version":"{version}"}}"#));
    tauri_build::build()
}

fn release_version(manifest: &str) -> String {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .expect("a version under [workspace.package] in verifique's Cargo.toml")
        .to_string()
}
