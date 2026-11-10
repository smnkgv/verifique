//! Needs a Postgres server at 127.0.0.1:5432 that the postgres user can reach with password postgres.

use std::io::Write;
use std::process::{Command, Stdio};

const PLUGIN: &str = env!("CARGO_BIN_EXE_verifique-postgres");
const SERVER: &str = "postgres://postgres:postgres@127.0.0.1:5432";

fn call(operation: &str, database: &str) -> (bool, serde_json::Value) {
    let mut child = Command::new(PLUGIN).arg(operation).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let request = serde_json::json!({ "protocol": 1, "server": SERVER, "database": database, "slot": 7 });
    child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), serde_json::from_slice(&out.stdout).unwrap())
}

#[test]
fn a_slot_starts_empty_shows_use_and_is_dropped() {
    let name = "verifique_plugin_test_postgres_s7";
    assert_eq!(call("reset", name), (true, serde_json::json!({})));
    assert_eq!(call("in-use", name).1["in_use"], false);
    let url = call("settings", name).1["settings"]["url"].as_str().unwrap().to_string();
    assert_eq!(url, format!("{SERVER}/{name}"));
    let mut client = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    client.batch_execute("create table items (id int)").unwrap();
    drop(client);
    assert_eq!(call("in-use", name).1["in_use"], true);
    assert_eq!(call("remove", name), (true, serde_json::json!({})));
}

#[test]
fn a_database_without_the_prefix_is_never_touched() {
    let (ok, reply) = call("reset", "postgres");
    assert!(!ok);
    assert!(reply["error"].as_str().unwrap().contains("only ever touches databases named verifique_"));
}
