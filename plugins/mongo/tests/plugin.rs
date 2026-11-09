//! Needs a MongoDB server at 127.0.0.1:27017.

use std::io::Write;
use std::process::{Command, Stdio};

const PLUGIN: &str = env!("CARGO_BIN_EXE_verifique-mongo");
const SERVER: &str = "mongodb://127.0.0.1:27017/?directConnection=true";

fn call(operation: &str, database: &str) -> (bool, serde_json::Value) {
    let mut child = Command::new(PLUGIN).arg(operation).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let request = serde_json::json!({ "protocol": 1, "server": SERVER, "database": database, "slot": 7 });
    child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), serde_json::from_slice(&out.stdout).unwrap())
}

#[test]
fn a_slot_starts_empty_shows_use_and_is_dropped() {
    let name = "verifique_plugin_test_mongo_s7";
    assert_eq!(call("reset", name), (true, serde_json::json!({})));
    assert_eq!(call("in-use", name).1["in_use"], false);
    let settings = call("settings", name).1["settings"].clone();
    assert_eq!(settings["url"], format!("mongodb://127.0.0.1:27017/{name}?directConnection=true"));
    let client = mongodb::sync::Client::with_uri_str(SERVER).unwrap();
    client.database(name).collection::<mongodb::bson::Document>("items").insert_one(mongodb::bson::doc! { "a": 1 }).run().unwrap();
    assert_eq!(call("in-use", name).1["in_use"], true);
    assert_eq!(call("remove", name), (true, serde_json::json!({})));
    assert_eq!(call("in-use", name).1["in_use"], false);
}
