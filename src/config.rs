use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::CONFIG_FILE;
use crate::services::KINDS;

pub type Needs = BTreeMap<String, u32>;
pub type Vars = BTreeMap<String, String>;

const DEFAULT_RETRIES: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Timeouts {
    pub prepare_sec: u64,
    pub ready_sec: u64,
    pub seed_sec: u64,
    pub setup_sec: u64,
    pub run_sec: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { prepare_sec: 1800, ready_sec: 600, seed_sec: 600, setup_sec: 300, run_sec: 1800 }
    }
}

/// A database server the project uses. It must already be running: nothing here installs one.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    #[serde(default)]
    pub url: Option<String>,
}

/// One process of a stack the tool runs itself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    #[serde(default)]
    pub cwd: Option<String>,
    pub run: String,
    #[serde(default)]
    pub port: Option<u32>,
    #[serde(default)]
    pub ready: Option<String>,
    #[serde(default)]
    pub env: Vars,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stack {
    #[serde(default)]
    pub prepare: Vec<String>,
    #[serde(default)]
    pub apps: BTreeMap<String, App>,
    #[serde(default)]
    pub needs: Needs,
    #[serde(default)]
    pub env: Vars,
    #[serde(default, rename = "envFiles", skip_serializing_if = "Vec::is_empty")]
    pub env_files: Vec<String>,
    #[serde(default)]
    pub seed: Option<String>,
    /// Extra data a person can ask for when starting the stack by hand, such as a real dump.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub seeds: BTreeMap<String, String>,
    #[serde(default)]
    pub setup: Option<String>,
}

impl Stack {
    /// The gitignored files a slot's worktree needs, as a user's own checkout would have them.
    pub fn env_files(&self) -> Vec<String> {
        if self.env_files.is_empty() {
            vec![".env".to_string()]
        } else {
            self.env_files.clone()
        }
    }

    /// Every setting the project file sets itself, which is what wins over `.env`.
    pub fn overridden(&self) -> impl Iterator<Item = &String> {
        self.env.keys().chain(self.apps.values().flat_map(|app| app.env.keys()))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckType {
    #[default]
    Command,
    Agent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerdictSource {
    #[default]
    Exit,
    File,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Agent {
    Preset(String),
    Custom { command: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    #[serde(default, rename = "type")]
    pub kind: CheckType,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub prepare: Vec<String>,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub verdict: VerdictSource,
    #[serde(default)]
    pub needs: Needs,
    /// How many stacks this check runs against, for a suite with several workers.
    #[serde(default)]
    pub slots: Option<usize>,
    #[serde(default)]
    pub agent: Option<Agent>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceSpec>,
    #[serde(default)]
    pub stacks: BTreeMap<String, Stack>,
    #[serde(default)]
    pub checks: BTreeMap<String, Check>,
}

/// Everything a job runs, frozen at submit so that later edits cannot change a queued job.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Plan {
    pub check: Check,
    pub stack: Option<Stack>,
    pub services: BTreeMap<String, ServiceSpec>,
    pub timeouts: Timeouts,
    pub retries: u32,
    pub headed: bool,
    /// What the submitter added after `--`, appended to the check's `run`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Set for a hold: a stack kept up for an investigation, with no check and no verdict.
    pub hold_minutes: Option<f64>,
    /// One of the stack's `seeds`, run after its usual seed, with `args` on the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_seed: Option<String>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        let raw = fs::read_to_string(&path).with_context(|| format!("no {CONFIG_FILE} in {}", root.display()))?;
        let config: Self = serde_yaml_ng::from_str(&raw).with_context(|| format!("reading {CONFIG_FILE}"))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.checks.is_empty() {
            bail!("{CONFIG_FILE} defines no checks");
        }
        if let Some(kind) = self.services.keys().find(|kind| !KINDS.contains(&kind.as_str())) {
            bail!("unknown service {kind}; have {}", KINDS.join(", "));
        }
        for (name, stack) in &self.stacks {
            if let Some((app, _)) = stack.apps.iter().find(|(_, app)| app.ready.is_some() && app.port.is_none()) {
                bail!("app {app} in stack {name} has \"ready\" but no \"port\" to check it on");
            }
            let settings = stack.env.values().chain(stack.apps.values().flat_map(|app| app.env.values()));
            for value in settings.chain(&stack.seed) {
                for kind in KINDS {
                    if value.contains(&format!("{{{kind}.")) && !self.services.contains_key(kind) {
                        bail!("stack {name} uses {kind}, which \"services\" does not declare");
                    }
                }
            }
        }
        for (name, check) in &self.checks {
            if let Some(stack) = &check.stack {
                if !self.stacks.contains_key(stack) {
                    bail!("check {name} uses unknown stack {stack}");
                }
            }
            if check.slots.is_some_and(|slots| slots == 0) {
                bail!("check {name} needs at least one slot");
            }
            if check.slots.is_some_and(|slots| slots > 1) && check.stack.is_none() {
                bail!("check {name}: several slots need a stack");
            }
            match check.kind {
                CheckType::Command if check.run.is_none() => bail!("check {name} needs \"run\""),
                CheckType::Agent if check.agent.is_none() => bail!("check {name} needs \"agent\""),
                _ => {}
            }
            if let Some(Agent::Preset(preset)) = &check.agent {
                if preset != "claude" {
                    bail!("check {name}: unknown agent {preset}; use claude or {{ command: ... }}");
                }
            }
        }
        Ok(())
    }

    /// The chosen check's plan and what it needs from the machine.
    pub fn plan(&self, name: Option<&str>) -> Result<(String, Plan, Needs)> {
        let names: Vec<&str> = self.checks.keys().map(String::as_str).collect();
        let chosen = match (name, names.as_slice()) {
            (Some(name), _) => name,
            (None, [only]) => only,
            (None, _) => bail!("pick a check with --check: {}", names.join(", ")),
        };
        let check = self
            .checks
            .get(chosen)
            .ok_or_else(|| anyhow!("no check {chosen}; have {}", names.join(", ")))?;
        let stack = check.stack.as_ref().map(|name| self.stacks[name].clone());
        let mut needs = stack.as_ref().map(|stack| stack.needs.clone()).unwrap_or_default();
        needs.extend(check.needs.clone());
        let plan = Plan {
            check: check.clone(),
            services: if stack.is_some() { self.services.clone() } else { BTreeMap::new() },
            stack,
            timeouts: self.timeouts.clone(),
            retries: self.retries.unwrap_or(DEFAULT_RETRIES),
            headed: false,
            args: Vec::new(),
            hold_minutes: None,
            extra_seed: None,
        };
        Ok((chosen.to_string(), plan, needs))
    }

    /// A plan that only brings a stack up, for someone to work with by hand.
    pub fn plan_hold(&self, name: Option<&str>, minutes: f64, seed: Option<&str>) -> Result<(Plan, Needs)> {
        let names: Vec<&str> = self.stacks.keys().map(String::as_str).collect();
        let chosen = match (name, names.as_slice()) {
            (Some(name), _) => name,
            (None, [only]) => only,
            (None, _) => bail!("name the stack to start: {}", names.join(", ")),
        };
        let stack = self
            .stacks
            .get(chosen)
            .ok_or_else(|| anyhow!("no stack {chosen}; have {}", names.join(", ")))?;
        let plan = Plan {
            check: Check::default(),
            stack: Some(stack.clone()),
            services: self.services.clone(),
            timeouts: self.timeouts.clone(),
            retries: 0,
            headed: false,
            args: Vec::new(),
            hold_minutes: Some(minutes),
            extra_seed: match seed {
                None => None,
                Some(seed) => Some(stack.seeds.get(seed).cloned().ok_or_else(|| {
                    let have: Vec<&str> = stack.seeds.keys().map(String::as_str).collect();
                    anyhow!("stack {chosen} has no seed {seed}; its seeds: {}", if have.is_empty() { "none".to_string() } else { have.join(", ") })
                })?),
            },
        };
        Ok((plan, stack.needs.clone()))
    }
}

/// What a slot gives a project's commands, as `{slot}`, `{port.web}` or `{postgres.url}`.
pub struct Values {
    pub slot: u32,
    pub ports: BTreeMap<String, u32>,
    tokens: Vars,
}

impl Values {
    pub fn new(stack: Option<&Stack>, slot: u32, service_tokens: Vars) -> Self {
        let mut ports = BTreeMap::new();
        let mut tokens = service_tokens;
        tokens.insert("slot".to_string(), slot.to_string());
        for (name, app) in stack.iter().flat_map(|stack| &stack.apps) {
            if let Some(base) = app.port {
                let port = base + slot;
                ports.insert(name.clone(), port);
                tokens.insert(format!("app.{name}.port"), port.to_string());
                tokens.insert(format!("app.{name}.url"), format!("http://127.0.0.1:{port}"));
            }
        }
        for (name, port) in &ports {
            tokens.insert(format!("port.{name}"), port.to_string());
        }
        Self { slot, ports, tokens }
    }

    pub fn tokens(&self) -> &Vars {
        &self.tokens
    }

    pub fn expand(&self, template: &str) -> String {
        self.tokens
            .iter()
            .fold(template.to_string(), |out, (name, value)| out.replace(&format!("{{{name}}}"), value))
    }

    /// As `expand`, where `{port}` also means this app's own port.
    pub fn expand_for(&self, app: &str, template: &str) -> String {
        let port = self.ports.get(app).map(u32::to_string).unwrap_or_default();
        self.expand(&template.replace("{port}", &port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Config> {
        let config: Config = serde_yaml_ng::from_str(yaml)?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn a_check_merges_what_its_stack_needs() {
        let config = parse(
            "stacks: { ios: { apps: { a: { run: x } }, needs: { simulator: 1 } } }\n\
             checks: { mobile: { stack: ios, run: y, needs: { licence: 1 } } }",
        )
        .unwrap();
        let (_, _, needs) = config.plan(None).unwrap();
        assert_eq!(needs.get("simulator"), Some(&1));
        assert_eq!(needs.get("licence"), Some(&1));
    }

    #[test]
    fn a_slot_shifts_every_port() {
        let web = App { run: "x".to_string(), port: Some(3500), ..App::default() };
        let stack = Stack { apps: BTreeMap::from([("web".to_string(), web)]), ..Stack::default() };
        let values = Values::new(Some(&stack), 4, Vars::new());
        assert_eq!(values.expand("up {slot} on {port.web}"), "up 4 on 3504");
    }

    #[test]
    fn an_app_can_reach_its_own_port_and_its_neighbours_url() {
        let api = App { run: "x".to_string(), port: Some(3600), ..App::default() };
        let stack = Stack { apps: BTreeMap::from([("api".to_string(), api)]), ..Stack::default() };
        let values = Values::new(Some(&stack), 4, Vars::new());
        assert_eq!(values.expand_for("api", "{port}"), "3604");
        assert_eq!(values.expand("{app.api.url}/x"), "http://127.0.0.1:3604/x");
    }

    #[test]
    fn a_ready_check_needs_a_port() {
        let yaml = "stacks: { s: { apps: { api: { run: x, ready: /health } } } }\nchecks: { c: { run: y, stack: s } }";
        assert!(parse(yaml).unwrap_err().to_string().contains("no \"port\""));
    }

    #[test]
    fn a_stack_needs_no_apps() {
        assert!(parse("stacks: { a: { seed: x } }\nchecks: { b: { stack: a, run: x } }").is_ok());
    }

    #[test]
    fn using_a_service_means_declaring_it() {
        let yaml = "stacks: { a: { apps: { b: { run: y, env: { D: \"{postgres.url}\" } } } } }\nchecks: { c: { run: x } }";
        assert!(parse(yaml).unwrap_err().to_string().contains("does not declare"));
    }

    #[test]
    fn several_slots_need_a_stack_to_copy() {
        let yaml = "checks: { c: { run: x, slots: 2 } }";
        assert!(parse(yaml).unwrap_err().to_string().contains("several slots"));
    }

    #[test]
    fn a_broken_config_names_the_problem() {
        assert!(parse("checks: {}").unwrap_err().to_string().contains("no checks"));
        assert!(parse("checks: { a: {} }").unwrap_err().to_string().contains("needs \"run\""));
        assert!(parse("checks: { a: { type: agent } }").unwrap_err().to_string().contains("needs \"agent\""));
        assert!(parse("checks: { a: { run: x, stack: gone } }").unwrap_err().to_string().contains("unknown stack"));
    }

    #[test]
    fn several_checks_need_an_explicit_choice() {
        let config = parse("checks: { a: { run: x }, b: { run: y } }").unwrap();
        assert!(config.plan(None).is_err());
        assert_eq!(config.plan(Some("b")).unwrap().0, "b");
    }
}
