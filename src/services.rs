use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::Vars;
use crate::paths;

/// Everything this tool creates carries this prefix, and it never drops anything without it, so a
/// slot's database can never be a real one.
const PREFIX: &str = "verifique_";
const MAX_NAME: usize = 63;

pub const KINDS: [&str; 3] = ["postgres", "mongo", "redis"];

pub fn default_url(kind: &str) -> &'static str {
    match kind {
        "postgres" => "postgres://127.0.0.1:5432",
        "mongo" => "mongodb://127.0.0.1:27017/?directConnection=true",
        _ => "redis://127.0.0.1:6379",
    }
}

fn client(kind: &str) -> &'static str {
    match kind {
        "postgres" => "psql",
        "mongo" => "mongosh",
        _ => "redis-cli",
    }
}

fn database_name(repo: &str, slot: u32) -> String {
    let label: String = paths::repo_label(repo)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let suffix = format!("_s{slot}");
    let room = MAX_NAME - PREFIX.len() - suffix.len();
    let label: String = label.chars().take(room).collect();
    format!("{PREFIX}{label}{suffix}")
}

/// `scheme://user:password@host:port/path?query`, taken apart without assuming the optional parts.
struct Url {
    scheme: String,
    user: String,
    password: String,
    host: String,
    port: String,
    query: String,
}

impl Url {
    fn parse(raw: &str) -> Result<Self> {
        let (scheme, rest) = raw.split_once("://").with_context(|| format!("not a URL: {raw}"))?;
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

    fn authority(&self) -> String {
        let credentials = match (self.user.is_empty(), self.password.is_empty()) {
            (true, _) => String::new(),
            (false, true) => format!("{}@", self.user),
            (false, false) => format!("{}:{}@", self.user, self.password),
        };
        let port = if self.port.is_empty() { String::new() } else { format!(":{}", self.port) };
        format!("{credentials}{}{port}", self.host)
    }

    fn with_path(&self, path: &str) -> String {
        let query = if self.query.is_empty() { String::new() } else { format!("?{}", self.query) };
        format!("{}://{}/{path}{query}", self.scheme, self.authority())
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

const DATABASE_SCHEMES: [&str; 10] = [
    "postgres", "postgresql", "mysql", "mariadb", "mongodb", "mongodb+srv", "redis", "rediss", "amqp", "amqps",
];
const LOCAL_HOSTS: [&str; 5] = ["localhost", "127.0.0.1", "::1", "0.0.0.0", "host.docker.internal"];

/// The host of a setting that is a database URL pointing away from this machine.
pub fn remote_database_host(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    // A replica set lists several hosts, each with its own port.
    let first = url.host.split(',').next().unwrap_or("");
    let host = first.split(':').next().unwrap_or("").to_string();
    let database = DATABASE_SCHEMES.contains(&url.scheme.as_str());
    (database && !host.is_empty() && !LOCAL_HOSTS.contains(&host.as_str())).then_some(host)
}

/// One slot's share of a database server.
pub struct Service {
    kind: String,
    server: Url,
    database: String,
    slot: u32,
}

impl Service {
    pub fn new(kind: &str, url: Option<&str>, repo: &str, slot: u32) -> Result<Self> {
        Ok(Self {
            kind: kind.to_string(),
            server: Url::parse(url.unwrap_or(default_url(kind)))?,
            database: database_name(repo, slot),
            slot,
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    fn url(&self) -> String {
        match self.kind.as_str() {
            "redis" => self.server.with_path(&self.slot.to_string()),
            _ => self.server.with_path(&self.database),
        }
    }

    pub fn tokens(&self) -> Vars {
        let kind = &self.kind;
        let mut tokens = Vars::from([
            (format!("{kind}.url"), self.url()),
            (format!("{kind}.server"), self.server.with_path("")),
            (format!("{kind}.database"), self.database.clone()),
            (format!("{kind}.host"), self.server.host.clone()),
            (format!("{kind}.port"), self.server.port.clone()),
            (format!("{kind}.user"), self.server.user.clone()),
            (format!("{kind}.password"), self.server.password.clone()),
        ]);
        if kind == "redis" {
            tokens.insert("redis.db".to_string(), self.slot.to_string());
            tokens.insert("redis.prefix".to_string(), format!("{}:", self.database));
        }
        tokens
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let tool = client(&self.kind);
        let out = Command::new(tool)
            .args(args)
            .env("PGCONNECT_TIMEOUT", "5")
            .output()
            .map_err(|_| anyhow!("{tool} is not installed, and {} needs it", self.kind))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // redis-cli reports a refused connection on stdout and still exits 0 for some commands.
        let refused = stdout.contains("Connection refused") || stdout.starts_with("ERR");
        if !out.status.success() || refused {
            let detail = String::from_utf8_lossy(&out.stderr);
            let detail = detail.lines().next().unwrap_or(&stdout);
            let detail = detail.replace(&self.server.password, "***");
            bail!(
                "{} at {} is not usable: {detail}. A running database server is a prerequisite.",
                self.kind,
                self.server.address()
            );
        }
        Ok(stdout)
    }

    fn psql(&self, database: &str, sql: &str) -> Result<String> {
        let url = self.server.with_path(database);
        self.run(&[&url, "-v", "ON_ERROR_STOP=1", "-Atc", sql])
    }

    fn mongosh(&self, script: &str) -> Result<String> {
        let mut server = Url::parse(&self.server.with_path(""))?;
        let timeout = "serverSelectionTimeoutMS=5000";
        server.query = if server.query.is_empty() { timeout.into() } else { format!("{}&{timeout}", server.query) };
        self.run(&[&server.with_path(""), "--quiet", "--eval", script])
    }

    fn redis(&self, database: u32, args: &[&str]) -> Result<String> {
        let url = self.server.with_path("");
        let number = database.to_string();
        let mut all = vec!["-u", url.as_str(), "--no-auth-warning", "-n", number.as_str()];
        all.extend(args);
        self.run(&all)
    }

    fn drop_data(&self) -> Result<()> {
        let name = &self.database;
        match self.kind.as_str() {
            "postgres" => {
                self.psql("postgres", &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))?;
            }
            "mongo" => {
                self.mongosh(&format!("db.getSiblingDB('{name}').dropDatabase()"))?;
            }
            _ => {
                self.redis(self.slot, &["FLUSHDB"])?;
                let script = "for _,k in ipairs(redis.call('keys',ARGV[1])) do redis.call('del',k) end";
                self.redis(0, &["EVAL", script, "0", &format!("{name}:*")])?;
            }
        }
        Ok(())
    }

    /// Start this slot from nothing. Mongo and Redis create what they need on first write.
    pub fn reset(&self) -> Result<()> {
        self.drop_data()?;
        if self.kind == "postgres" {
            self.psql("postgres", &format!("CREATE DATABASE \"{}\"", self.database))?;
        }
        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        self.drop_data()
    }

    /// Whether anything has really reached this slot's data. An app that ignored its settings and
    /// used another database leaves this empty, which is the mistake `doctor` exists to catch.
    pub fn in_use(&self) -> Result<bool> {
        let name = &self.database;
        let count = match self.kind.as_str() {
            "postgres" => {
                let sql = format!("SELECT count(*) FROM pg_stat_activity WHERE datname = '{name}' AND pid <> pg_backend_pid()");
                let connected: u64 = self.psql("postgres", &sql)?.parse().unwrap_or(0);
                let sql = "SELECT count(*) FROM information_schema.tables \
                           WHERE table_schema NOT IN ('pg_catalog', 'information_schema')";
                connected + self.psql(name, sql)?.parse::<u64>().unwrap_or(0)
            }
            "mongo" => self
                .mongosh(&format!("print(db.getSiblingDB('{name}').getCollectionNames().length)"))?
                .parse()
                .unwrap_or(0),
            _ => {
                let keys: u64 = self.redis(self.slot, &["DBSIZE"])?.parse().unwrap_or(0);
                let script = "return #redis.call('keys',ARGV[1])";
                let prefixed = self.redis(0, &["EVAL", script, "0", &format!("{name}:*")])?;
                keys + prefixed.parse::<u64>().unwrap_or(0)
            }
        };
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_gets_its_own_name_and_the_prefix_is_never_lost() {
        let name = database_name("/src/Light-Box.TV", 4);
        assert_eq!(name, "verifique_light_box_tv_s4");
        let long = database_name(&format!("/src/{}", "a".repeat(200)), 12);
        assert!(long.starts_with(PREFIX) && long.ends_with("_s12") && long.len() <= MAX_NAME);
    }

    #[test]
    fn only_a_database_url_on_another_machine_is_remote() {
        assert_eq!(remote_database_host("postgres://u:p@db.prod.example:5432/app"), Some("db.prod.example".into()));
        assert_eq!(remote_database_host("mongodb+srv://u:p@cluster.mongodb.net/x"), Some("cluster.mongodb.net".into()));
        assert_eq!(remote_database_host("mongodb://a.example:27017,b.example:27017/x"), Some("a.example".into()));
        assert_eq!(remote_database_host("postgres://postgres:postgres@127.0.0.1:5432/app"), None);
        assert_eq!(remote_database_host("https://api.example.com"), None);
        assert_eq!(remote_database_host("just text"), None);
    }

    #[test]
    fn urls_keep_their_credentials_and_query() {
        let postgres = Service::new("postgres", Some("postgres://u:p@db:5433"), "/x/app", 4).unwrap();
        assert_eq!(postgres.url(), "postgres://u:p@db:5433/verifique_app_s4");
        let mongo = Service::new("mongo", None, "/x/app", 5).unwrap();
        assert_eq!(mongo.url(), "mongodb://127.0.0.1:27017/verifique_app_s5?directConnection=true");
        let tokens = mongo.tokens();
        assert_eq!(tokens["mongo.server"], "mongodb://127.0.0.1:27017/?directConnection=true");
        let redis = Service::new("redis", None, "/x/app", 6).unwrap();
        assert_eq!(redis.url(), "redis://127.0.0.1:6379/6");
        assert_eq!(redis.tokens()["redis.prefix"], "verifique_app_s6:");
    }
}
