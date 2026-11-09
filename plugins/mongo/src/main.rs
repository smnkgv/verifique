//! Gives each verifique slot its own MongoDB database on a server that is already running. Mongo
//! creates a database on its first write, so starting a slot from nothing only drops it.

use std::collections::BTreeMap;
use std::process::ExitCode;

use mongodb::sync::Client;
use verifique_plugin::{run_database, unusable, Database, DatabaseRequest, Url};

const SELECTION_TIMEOUT: &str = "serverSelectionTimeoutMS=5000";

struct Mongo;

fn connect(server: &Url) -> Result<Client, String> {
    let mut server = server.clone();
    server.query = if server.query.is_empty() { SELECTION_TIMEOUT.into() } else { format!("{}&{SELECTION_TIMEOUT}", server.query) };
    Client::with_uri_str(server.with_path("")).map_err(|error| unusable("mongo", &server, &first_line(&error)))
}

fn first_line(error: &mongodb::error::Error) -> String {
    error.to_string().lines().next().unwrap_or("").to_string()
}

fn drop_database(request: &DatabaseRequest) -> Result<(), String> {
    let server = Url::parse(&request.server)?;
    connect(&server)?
        .database(&request.database)
        .drop()
        .run()
        .map_err(|error| unusable("mongo", &server, &first_line(&error)))
}

impl Database for Mongo {
    fn name(&self) -> &str {
        "mongo"
    }

    fn default_url(&self) -> Option<&str> {
        Some("mongodb://127.0.0.1:27017/?directConnection=true")
    }

    fn settings(&self, request: &DatabaseRequest) -> Result<BTreeMap<String, String>, String> {
        let server = Url::parse(&request.server)?;
        Ok(server.settings(server.with_path(&request.database), &request.database))
    }

    fn reset(&self, request: &DatabaseRequest) -> Result<(), String> {
        drop_database(request)
    }

    fn remove(&self, request: &DatabaseRequest) -> Result<(), String> {
        drop_database(request)
    }

    fn in_use(&self, request: &DatabaseRequest) -> Result<bool, String> {
        let server = Url::parse(&request.server)?;
        let names = connect(&server)?
            .database(&request.database)
            .list_collection_names()
            .run()
            .map_err(|error| unusable("mongo", &server, &first_line(&error)))?;
        Ok(!names.is_empty())
    }
}

fn main() -> ExitCode {
    run_database(Mongo, env!("CARGO_PKG_VERSION"))
}
