use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::paths::{self, NAME};
use crate::platform;

const QUIET_PERIOD: Duration = Duration::from_secs(60);

/// A broken setup can send every branch of a repository to `needs-you` at once, so each repository
/// raises at most one notification in a quiet period, naming how many followed the first.
#[derive(Default)]
pub struct Notifier {
    last: BTreeMap<String, Instant>,
    waiting: BTreeMap<String, u32>,
}

impl Notifier {
    pub fn needs_you(&mut self, repo: &str, branch: &str) {
        let label = paths::repo_label(repo);
        match self.last.get(&label) {
            Some(when) if when.elapsed() < QUIET_PERIOD => {
                *self.waiting.entry(label).or_default() += 1;
            }
            _ => {
                send(&format!("{label}: {branch} needs you"));
                self.last.insert(label, Instant::now());
            }
        }
    }

    pub fn flush(&mut self) {
        let due: Vec<String> = self
            .waiting
            .keys()
            .filter(|label| self.last.get(*label).is_none_or(|when| when.elapsed() >= QUIET_PERIOD))
            .cloned()
            .collect();
        for label in due {
            let count = self.waiting.remove(&label).unwrap_or(0);
            send(&format!("{label}: {count} more need you"));
            self.last.insert(label, Instant::now());
        }
    }
}

/// `VERIFIQUE_NOTIFY=off` keeps a queue quiet, as a test run's must be.
fn send(message: &str) {
    if std::env::var("VERIFIQUE_NOTIFY").is_ok_and(|value| value == "off") {
        return;
    }
    platform::notify(NAME, message);
}
