//! Needs a Redis server at 127.0.0.1:6379 with the usual 16 databases.

use std::io::Write;
use std::process::{Command, Stdio};

const PLUGIN: &str = env!("CARGO_BIN_EXE_verifique-redis");
const SERVER: &str = "redis://127.0.0.1:6379";

fn call(operation: &str, database: &str, slot: u32) -> (bool, serde_json::Value) {
    let mut child = Command::new(PLUGIN).arg(operation).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let request = serde_json::json!({ "protocol": 1, "server": SERVER, "database": database, "slot": slot });
    child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), serde_json::from_slice(&out.stdout).unwrap())
}

#[test]
fn a_slot_owns_its_numbered_database_and_its_prefix_in_database_0() {
    let name = "verifique_plugin_test_redis_s13";
    assert_eq!(call("reset", name, 13), (true, serde_json::json!({})));
    assert_eq!(call("in-use", name, 13).1["in_use"], false);
    let settings = call("settings", name, 13).1["settings"].clone();
    assert_eq!(settings["url"], "redis://127.0.0.1:6379/13");
    assert_eq!(settings["prefix"], format!("{name}:"));
    let mut shared = redis::Client::open(format!("{SERVER}/0")).unwrap().get_connection().unwrap();
    redis::cmd("SET").arg(format!("{name}:seen")).arg(1).query::<()>(&mut shared).unwrap();
    assert_eq!(call("in-use", name, 13).1["in_use"], true);
    assert_eq!(call("remove", name, 13), (true, serde_json::json!({})));
    let left: Option<String> = redis::cmd("GET").arg(format!("{name}:seen")).query(&mut shared).unwrap();
    assert_eq!(left, None);
}

#[test]
fn slot_0_and_a_slot_the_server_lacks_are_refused() {
    let (ok, reply) = call("settings", "verifique_x_s0", 0);
    assert!(!ok && reply["error"].as_str().unwrap().contains("database 0"));
    let (ok, reply) = call("reset", "verifique_x_s99", 99);
    assert!(!ok && reply["error"].as_str().unwrap().contains("has no database for slot 99"));
}
