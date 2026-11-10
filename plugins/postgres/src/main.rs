//! Gives each verifique slot its own Postgres database on a server that is already running.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Duration;

use postgres::{Client, Config, NoTls};
use verifique_plugin::{run_database, unusable, Database, DatabaseRequest, Url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

struct Postgres;

fn connect(server: &Url, database: &str) -> Result<Client, String> {
    let url = server.with_path(database);
    let mut config: Config = url.parse().map_err(|error| unusable("postgres", server, &format!("{error}")))?;
    config.connect_timeout(CONNECT_TIMEOUT);
    // A URL without a user means what it means to psql: PGUSER, or else the user's own name.
    if config.get_user().is_none() {
        let user = ["PGUSER", "USER", "USERNAME"].iter().find_map(|name| std::env::var(name).ok().filter(|user| !user.is_empty()));
        config.user(user.as_deref().unwrap_or("postgres"));
    }
    if config.get_password().is_none() {
        if let Ok(password) = std::env::var("PGPASSWORD") {
            config.password(password.as_bytes());
        }
    }
    config.connect(NoTls).map_err(|error| unusable("postgres", server, &first_line(&error)))
}

fn first_line(error: &postgres::Error) -> String {
    let text = match error.as_db_error() {
        Some(db) => db.message().to_string(),
        None => error.to_string(),
    };
    text.lines().next().unwrap_or("").to_string()
}

fn execute(server: &Url, database: &str, sql: &str) -> Result<(), String> {
    connect(server, database)?.batch_execute(sql).map_err(|error| unusable("postgres", server, &first_line(&error)))
}

fn count(client: &mut Client, server: &Url, sql: &str) -> Result<i64, String> {
    let row = client.query_one(sql, &[]).map_err(|error| unusable("postgres", server, &first_line(&error)))?;
    Ok(row.get(0))
}

fn drop_database(server: &Url, name: &str) -> Result<(), String> {
    execute(server, "postgres", &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
}

impl Database for Postgres {
    fn name(&self) -> &str {
        "postgres"
    }

    fn default_url(&self) -> Option<&str> {
        Some("postgres://127.0.0.1:5432")
    }

    fn settings(&self, request: &DatabaseRequest) -> Result<BTreeMap<String, String>, String> {
        let server = Url::parse(&request.server)?;
        Ok(server.settings(server.with_path(&request.database), &request.database))
    }

    fn reset(&self, request: &DatabaseRequest) -> Result<(), String> {
        let server = Url::parse(&request.server)?;
        drop_database(&server, &request.database)?;
        execute(&server, "postgres", &format!("CREATE DATABASE \"{}\"", request.database))
    }

    fn remove(&self, request: &DatabaseRequest) -> Result<(), String> {
        drop_database(&Url::parse(&request.server)?, &request.database)
    }

    fn in_use(&self, request: &DatabaseRequest) -> Result<bool, String> {
        let server = Url::parse(&request.server)?;
        let name = &request.database;
        let mut maintenance = connect(&server, "postgres")?;
        let connected = count(
            &mut maintenance,
            &server,
            &format!("SELECT count(*) FROM pg_stat_activity WHERE datname = '{name}' AND pid <> pg_backend_pid()"),
        )?;
        drop(maintenance);
        let tables = count(
            &mut connect(&server, name)?,
            &server,
            "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog', 'information_schema')",
        )?;
        Ok(connected + tables > 0)
    }
}

fn main() -> ExitCode {
    run_database(Postgres, env!("CARGO_PKG_VERSION"))
}
