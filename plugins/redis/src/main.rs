//! Gives each verifique slot its own Redis database on a server that is already running.
//!
//! Redis has no named databases, so each slot owns the numbered database that matches it and
//! clears it for every check. Apps that only ever use database 0 get a key prefix instead, and
//! their keys are cleared from database 0 by that prefix.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Duration;

use redis::{Client, Connection};
use verifique_plugin::{run_database, unusable, Database, DatabaseRequest, Url};

const TIMEOUT: Duration = Duration::from_secs(5);
const DELETE_PREFIXED: &str = "for _,k in ipairs(redis.call('keys',ARGV[1])) do redis.call('del',k) end";
const COUNT_PREFIXED: &str = "return #redis.call('keys',ARGV[1])";

struct Redis;

fn refuse_slot_zero(request: &DatabaseRequest) -> Result<(), String> {
    if request.slot == 0 {
        return Err("slot 0 would use Redis database 0, which everything else on this machine shares. Start the queue with a --first-slot of 1 or more".to_string());
    }
    Ok(())
}

/// A connection to one numbered database, proved to be that one. Selecting a database the server
/// doesn't have fails, and carrying on in database 0 would let FLUSHDB wipe everything.
fn open(server: &Url, database: u32, slot: u32) -> Result<Connection, String> {
    let client = Client::open(server.with_path("")).map_err(|error| unusable("redis", server, &error.to_string()))?;
    let mut connection = client.get_connection_with_timeout(TIMEOUT).map_err(|error| unusable("redis", server, &error.to_string()))?;
    let selected: redis::RedisResult<()> = redis::cmd("SELECT").arg(database).query(&mut connection);
    if let Err(error) = selected {
        if error.to_string().contains("out of range") {
            return Err(format!(
                "Redis at {} has no database for slot {slot}. Raise `databases` in redis.conf, or start the queue with a lower --first-slot",
                server.address()
            ));
        }
        return Err(unusable("redis", server, &error.to_string()));
    }
    Ok(connection)
}

fn prefixed(request: &DatabaseRequest) -> String {
    format!("{}:*", request.database)
}

fn clear(request: &DatabaseRequest) -> Result<(), String> {
    refuse_slot_zero(request)?;
    let server = Url::parse(&request.server)?;
    let fail = |error: redis::RedisError| unusable("redis", &server, &error.to_string());
    let mut own = open(&server, request.slot, request.slot)?;
    redis::cmd("FLUSHDB").query::<()>(&mut own).map_err(fail)?;
    let mut shared = open(&server, 0, request.slot)?;
    redis::cmd("EVAL").arg(DELETE_PREFIXED).arg(0).arg(prefixed(request)).query::<()>(&mut shared).map_err(fail)
}

impl Database for Redis {
    fn name(&self) -> &str {
        "redis"
    }

    fn default_url(&self) -> Option<&str> {
        Some("redis://127.0.0.1:6379")
    }

    fn settings(&self, request: &DatabaseRequest) -> Result<BTreeMap<String, String>, String> {
        refuse_slot_zero(request)?;
        let server = Url::parse(&request.server)?;
        let mut settings = server.settings(server.with_path(&request.slot.to_string()), &request.database);
        settings.insert("db".to_string(), request.slot.to_string());
        settings.insert("prefix".to_string(), format!("{}:", request.database));
        Ok(settings)
    }

    fn reset(&self, request: &DatabaseRequest) -> Result<(), String> {
        clear(request)
    }

    fn remove(&self, request: &DatabaseRequest) -> Result<(), String> {
        clear(request)
    }

    fn in_use(&self, request: &DatabaseRequest) -> Result<bool, String> {
        refuse_slot_zero(request)?;
        let server = Url::parse(&request.server)?;
        let fail = |error: redis::RedisError| unusable("redis", &server, &error.to_string());
        let keys: u64 = redis::cmd("DBSIZE").query(&mut open(&server, request.slot, request.slot)?).map_err(fail)?;
        let shared: u64 = redis::cmd("EVAL")
            .arg(COUNT_PREFIXED)
            .arg(0)
            .arg(prefixed(request))
            .query(&mut open(&server, 0, request.slot)?)
            .map_err(fail)?;
        Ok(keys + shared > 0)
    }
}

fn main() -> ExitCode {
    run_database(Redis, env!("CARGO_PKG_VERSION"))
}
