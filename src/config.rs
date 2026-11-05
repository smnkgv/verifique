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
    /// The project's own stack command, for a project that isolates itself.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub apps: BTreeMap<String, App>,
    #[serde(default)]
    pub ready: Vec<String>,
    #[serde(default)]
    pub ports: BTreeMap<String, u32>,
    #[serde(default)]
    pub needs: Needs,
    #[serde(default)]
    pub env: Vars,
    #[serde(default, rename = "envFiles", skip_serializing_if = "Vec::is_empty")]
    pub env_files: Vec<String>,
    #[serde(default)]
    pub seed: Option<String>,
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
pub enum Browser {
    #[default]
    Headless,
    Headed,
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
    pub browser: Browser,
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
    /// The most slots this project takes at once, so it can't crowd out the others on a machine.
    #[serde(default, rename = "maxSlots")]
    pub max_slots: Option<usize>,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub check: Check,
    pub stack: Option<Stack>,
    pub services: BTreeMap<String, ServiceSpec>,
    pub timeouts: Timeouts,
    pub retries: u32,
    pub headed: bool,
    /// Set for a hold: a stack kept up for an investigation, with no check and no verdict.
    pub hold_minutes: Option<f64>,
    #[serde(default)]
    pub max_slots: Option<usize>,
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
            if stack.start.is_some() == !stack.apps.is_empty() {
                bail!("stack {name} needs exactly one of \"start\" or \"apps\"");
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
            if check.slots.is_some_and(|slots| slots > 1) {
                let own_apps = check.stack.as_ref().is_some_and(|stack| !self.stacks[stack].apps.is_empty());
                if !own_apps {
                    bail!("check {name}: several slots need a stack with \"apps\"");
                }
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
            headed: check.browser == Browser::Headed,
            hold_minutes: None,
            max_slots: self.max_slots,
        };
        Ok((chosen.to_string(), plan, needs))
    }

    /// A plan that only brings a stack up, for someone to work with by hand.
    pub fn plan_hold(&self, name: Option<&str>, minutes: f64) -> Result<(Plan, Needs)> {
        let names: Vec<&str> = self.stacks.keys().map(String::as_str).collect();
        let chosen = match (name, names.as_slice()) {
            (Some(name), _) => name,
            (None, [only]) => only,
            (None, _) => bail!("pick a stack with --stack: {}", names.join(", ")),
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
            hold_minutes: Some(minutes),
            max_slots: self.max_slots,
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
        let mut ports: BTreeMap<String, u32> =
            stack.iter().flat_map(|stack| &stack.ports).map(|(name, base)| (name.clone(), base + slot)).collect();
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
            "stacks: { ios: { start: x, needs: { simulator: 1 } } }\n\
             checks: { mobile: { stack: ios, run: y, needs: { licence: 1 } } }",
        )
        .unwrap();
        let (_, _, needs) = config.plan(None).unwrap();
        assert_eq!(needs.get("simulator"), Some(&1));
        assert_eq!(needs.get("licence"), Some(&1));
    }

    #[test]
    fn a_slot_shifts_every_port() {
        let stack = Stack {
            start: Some("x".to_string()),
            ports: BTreeMap::from([("web".to_string(), 3500)]),
            ..Stack::default()
        };
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
    fn a_stack_is_either_a_command_or_a_set_of_apps() {
        assert!(parse("stacks: { a: {} }\nchecks: { b: { run: x } }").unwrap_err().to_string().contains("exactly one"));
        assert!(parse("stacks: { a: { start: x, apps: { b: { run: y } } } }\nchecks: { b: { run: x } }").is_err());
    }

    #[test]
    fn using_a_service_means_declaring_it() {
        let yaml = "stacks: { a: { apps: { b: { run: y, env: { D: \"{postgres.url}\" } } } } }\nchecks: { c: { run: x } }";
        assert!(parse(yaml).unwrap_err().to_string().contains("does not declare"));
    }

    #[test]
    fn several_slots_need_apps_because_only_apps_can_share_one_build() {
        let yaml = "stacks: { a: { start: y } }\nchecks: { c: { run: x, stack: a, slots: 2 } }";
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
