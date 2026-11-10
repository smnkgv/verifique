//! What verifique and its plugins say to each other.
//!
//! A plugin is a program called `verifique-<name>` (`verifique-<name>.exe` on Windows). verifique
//! runs it with one operation as its only argument, writes a JSON request to its stdin and reads a
//! JSON reply from its stdout. Exit 0 means the reply holds the answer, and anything else means it
//! holds `{"error": "..."}`, written for the person who will read it. `docs/PLUGINS.md` describes
//! the protocol for plugins written in any language. This crate is that protocol for plugins
//! written in Rust, and verifique uses it too, so the two sides agree by construction.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

/// Raised whenever a request or reply changes in a way an older side would misread.
pub const PROTOCOL: u32 = 1;

/// A plugin's program is this followed by its name.
pub const PROGRAM_PREFIX: &str = "verifique-";

/// Every database verifique creates carries this prefix, and a plugin refuses any other name, so a
/// slot's database can never be a real one.
pub const DATABASE_PREFIX: &str = "verifique_";

pub const DESCRIBE: &str = "describe";
pub const SETTINGS: &str = "settings";
pub const RESET: &str = "reset";
pub const REMOVE: &str = "remove";
pub const IN_USE: &str = "in-use";

/// What `describe` answers, the first thing verifique asks a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    pub protocol: u32,
    pub name: String,
    pub version: String,
    pub provides: Provides,
    /// Where the server usually is, for a project that names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provides {
    /// Gives each slot a database of its own on a server that is already running.
    Database,
}

/// One slot's share of a database server: every database operation is about one of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRequest {
    pub protocol: u32,
    /// The server, as the project's `services` names it or the plugin's default.
    pub server: String,
    /// The slot's database, always starting with `DATABASE_PREFIX`.
    pub database: String,
    pub slot: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// From `settings`: what a stack's settings may use, as `{<service>.<name>}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<BTreeMap<String, String>>,
    /// From `in-use`: whether anything has reached the slot's database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_use: Option<bool>,
}

/// A database plugin. Each method answers for one slot, and an error is a sentence for the user.
pub trait Database {
    fn name(&self) -> &str;

    fn default_url(&self) -> Option<&str>;

    /// What the slot's apps and seeds need to reach its database. `url` and `database` are
    /// expected; `server`, `host`, `port`, `user` and `password` are customary.
    fn settings(&self, request: &DatabaseRequest) -> Result<BTreeMap<String, String>, String>;

    /// Start the slot from nothing, as every check does.
    fn reset(&self, request: &DatabaseRequest) -> Result<(), String>;

    /// Drop whatever the slot holds.
    fn remove(&self, request: &DatabaseRequest) -> Result<(), String>;

    /// Whether anything has really reached the slot's database. An app that ignored its settings
    /// and used another database leaves it empty, which is the mistake `verifique doctor` catches.
    fn in_use(&self, request: &DatabaseRequest) -> Result<bool, String>;
}

fn answer(operation: &str, plugin: &impl Database, version: &str) -> Result<String, String> {
    if operation == DESCRIBE {
        let description = Description {
            protocol: PROTOCOL,
            name: plugin.name().to_string(),
            version: version.to_string(),
            provides: Provides::Database,
            default_url: plugin.default_url().map(str::to_string),
        };
        return serde_json::to_string(&description).map_err(|error| error.to_string());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|error| format!("reading the request: {error}"))?;
    let request: DatabaseRequest = serde_json::from_str(&input).map_err(|error| format!("reading the request: {error}"))?;
    if request.protocol != PROTOCOL {
        return Err(format!(
            "this plugin speaks protocol {PROTOCOL} and verifique sent {}: install the {}{} that matches your verifique",
            request.protocol,
            PROGRAM_PREFIX,
            plugin.name()
        ));
    }
    if !request.database.starts_with(DATABASE_PREFIX) {
        return Err(format!("refusing {}: verifique only ever touches databases named {DATABASE_PREFIX}*", request.database));
    }
    let reply = match operation {
        SETTINGS => Reply { settings: Some(plugin.settings(&request)?), ..Reply::default() },
        RESET => plugin.reset(&request).map(|()| Reply::default())?,
        REMOVE => plugin.remove(&request).map(|()| Reply::default())?,
        IN_USE => Reply { in_use: Some(plugin.in_use(&request)?), ..Reply::default() },
        other => return Err(format!("unknown operation {other:?}")),
    };
    serde_json::to_string(&reply).map_err(|error| error.to_string())
}

/// The whole of a database plugin's `main`: read the operation and request, answer, exit.
pub fn run_database(plugin: impl Database, version: &str) -> ExitCode {
    let operation = std::env::args().nth(1).unwrap_or_default();
    match answer(&operation, &plugin, version) {
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let reply = Reply { error: Some(error), ..Reply::default() };
            println!("{}", serde_json::to_string(&reply).unwrap_or_default());
            ExitCode::FAILURE
        }
    }
}

/// `scheme://user:password@host:port/path?query`, taken apart without assuming the optional parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub query: String,
}

impl Url {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (scheme, rest) = raw.split_once("://").ok_or_else(|| format!("not a URL: {raw}"))?;
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let authority = rest.split('/').next().unwrap_or("");
        let (credentials, address) = authority.rsplit_once('@').unwrap_or(("", authority));
        let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
        let (host, port) = address.rsplit_once(':').unwrap_or((address, ""));
        Ok(Self {
            scheme: scheme.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            host: host.to_string(),
            port: port.to_string(),
            query: query.to_string(),
        })
    }

    pub fn authority(&self) -> String {
        // Redis URLs often carry a password and no user, as `redis://:secret@host`.
        let credentials = match (self.user.is_empty(), self.password.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!("{}@", self.user),
            (_, false) => format!("{}:{}@", self.user, self.password),
        };
        let port = if self.port.is_empty() { String::new() } else { format!(":{}", self.port) };
        format!("{credentials}{}{port}", self.host)
    }

    pub fn with_path(&self, path: &str) -> String {
        let query = if self.query.is_empty() { String::new() } else { format!("?{}", self.query) };
        format!("{}://{}/{path}{query}", self.scheme, self.authority())
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The settings every database plugin gives: the slot's URL, the server's and their parts.
    pub fn settings(&self, url: String, database: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("url".to_string(), url),
            ("server".to_string(), self.with_path("")),
            ("database".to_string(), database.to_string()),
            ("host".to_string(), self.host.clone()),
            ("port".to_string(), self.port.clone()),
            ("user".to_string(), self.user.clone()),
            ("password".to_string(), self.password.clone()),
        ])
    }
}

/// An error that names the server without its password, which error text from a client may echo.
pub fn unusable(kind: &str, server: &Url, detail: &str) -> String {
    let detail = if server.password.is_empty() { detail.to_string() } else { detail.replace(&server.password, "***") };
    format!("{kind} at {} is not usable: {detail}. A running database server is a prerequisite.", server.address())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_keep_their_credentials_and_query() {
        let url = Url::parse("postgres://u:p@db:5433?sslmode=disable").unwrap();
        assert_eq!(url.with_path("x"), "postgres://u:p@db:5433/x?sslmode=disable");
        assert_eq!(url.address(), "db:5433");
        let bare = Url::parse("redis://127.0.0.1:6379").unwrap();
        assert_eq!(bare.with_path("6"), "redis://127.0.0.1:6379/6");
        let password_only = Url::parse("redis://:secret@127.0.0.1:6379").unwrap();
        assert_eq!(password_only.with_path("6"), "redis://:secret@127.0.0.1:6379/6");
    }

    #[test]
    fn a_password_never_reaches_an_error() {
        let url = Url::parse("postgres://u:secret@db:5432").unwrap();
        assert!(!unusable("postgres", &url, "auth failed for secret").contains("secret"));
    }

    #[test]
    fn replies_carry_only_what_they_answer() {
        assert_eq!(serde_json::to_string(&Reply::default()).unwrap(), "{}");
        let description = Description {
            protocol: PROTOCOL,
            name: "x".into(),
            version: "1".into(),
            provides: Provides::Database,
            default_url: None,
        };
        assert_eq!(serde_json::to_string(&description).unwrap(), r#"{"protocol":1,"name":"x","version":"1","provides":"database"}"#);
    }
}
