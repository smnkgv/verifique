use anyhow::{anyhow, bail, Result};
use verifique_plugin::{DatabaseRequest, Provides, Url, DATABASE_PREFIX, IN_USE, PROTOCOL, REMOVE, RESET, SETTINGS};

use crate::config::Vars;
use crate::paths;
use crate::plugins::{self, Plugin};

const MAX_NAME: usize = 63;

/// Names a project can't give a service, because a stack's settings already use them.
pub const RESERVED: [&str; 3] = ["app", "port", "slot"];

fn database_name(repo: &str, slot: u32) -> String {
    let label: String = paths::repo_label(repo)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let suffix = format!("_s{slot}");
    let room = MAX_NAME - DATABASE_PREFIX.len() - suffix.len();
    let label: String = label.chars().take(room).collect();
    format!("{DATABASE_PREFIX}{label}{suffix}")
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

/// One slot's share of a database server, kept by the plugin named after the service.
pub struct Service {
    kind: String,
    plugin: Plugin,
    request: DatabaseRequest,
    tokens: Vars,
}

impl Service {
    pub fn new(kind: &str, url: Option<&str>, repo: &str, slot: u32) -> Result<Self> {
        let plugin = plugins::load(kind)?;
        if plugin.description.provides != Provides::Database {
            bail!("{} gives no databases, so it can't serve services.{kind}", plugin.program.display());
        }
        let server = url
            .map(str::to_string)
            .or_else(|| plugin.description.default_url.clone())
            .ok_or_else(|| anyhow!("services.{kind} needs a url: its plugin knows no usual place for the server"))?;
        let request = DatabaseRequest { protocol: PROTOCOL, server, database: database_name(repo, slot), slot };
        let settings = plugin.call(SETTINGS, &request)?.settings.unwrap_or_default();
        let tokens = settings.into_iter().map(|(name, value)| (format!("{kind}.{name}"), value)).collect();
        Ok(Self { kind: kind.to_string(), plugin, request, tokens })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn tokens(&self) -> Vars {
        self.tokens.clone()
    }

    /// Start this slot from nothing.
    pub fn reset(&self) -> Result<()> {
        self.plugin.call(RESET, &self.request).map(drop)
    }

    pub fn remove(&self) -> Result<()> {
        self.plugin.call(REMOVE, &self.request).map(drop)
    }

    /// Whether anything has really reached this slot's data. An app that ignored its settings and
    /// used another database leaves this empty, which is the mistake `doctor` exists to catch.
    pub fn in_use(&self) -> Result<bool> {
        Ok(self.plugin.call(IN_USE, &self.request)?.in_use.unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_gets_its_own_name_and_the_prefix_is_never_lost() {
        let name = database_name("/src/Shop-Front.App", 4);
        assert_eq!(name, "verifique_shop_front_app_s4");
        let long = database_name(&format!("/src/{}", "a".repeat(200)), 12);
        assert!(long.starts_with(DATABASE_PREFIX) && long.ends_with("_s12") && long.len() <= MAX_NAME);
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
}
