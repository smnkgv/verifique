use std::collections::BTreeMap;

use crate::config::Needs;

pub struct Candidate<'a> {
    pub id: i64,
    pub repo: &'a str,
    pub retry: bool,
    pub priority: bool,
    pub needs: &'a Needs,
}

pub fn fits(needs: &Needs, used: &Needs, resources: &Needs) -> bool {
    needs.iter().all(|(name, count)| {
        used.get(name).copied().unwrap_or(0) + count <= resources.get(name).copied().unwrap_or(0)
    })
}

/// The first thing a job needs that this machine can never give it, however long it waits.
pub fn unavailable<'a>(needs: &'a Needs, resources: &Needs) -> Option<&'a str> {
    needs
        .iter()
        .find(|(name, count)| **count > resources.get(*name).copied().unwrap_or(0))
        .map(|(name, _)| name.as_str())
}

/// Which job takes the next free slot. `last_served` holds when each repository last got one.
///
/// A job with `next` goes first. Otherwise repositories take turns, so one busy repository cannot
/// starve the others, and within a repository a retry goes ahead of new work because that agent's
/// context is freshest. A job waiting on a busy resource never holds up one that fits.
pub fn pick(
    queued: &[Candidate],
    used: &Needs,
    resources: &Needs,
    last_served: &BTreeMap<String, u64>,
) -> Option<i64> {
    let ready = queued.iter().filter(|job| fits(job.needs, used, resources));
    if let Some(urgent) = ready.clone().filter(|job| job.priority).min_by_key(|job| job.id) {
        return Some(urgent.id);
    }
    ready
        .min_by_key(|job| (last_served.get(job.repo).copied().unwrap_or(0), !job.retry, job.id))
        .map(|job| job.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job<'a>(id: i64, repo: &'a str, needs: &'a Needs) -> Candidate<'a> {
        Candidate { id, repo, retry: false, priority: false, needs }
    }

    fn served(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(repo, turn)| (repo.to_string(), *turn)).collect()
    }

    #[test]
    fn repositories_take_turns() {
        let none = Needs::new();
        let queued = [job(1, "a", &none), job(2, "a", &none), job(3, "b", &none)];
        let turns = served(&[("a", 5)]);
        assert_eq!(pick(&queued, &none, &none, &turns), Some(3));
    }

    #[test]
    fn a_repository_never_served_is_not_behind_one_that_was() {
        let none = Needs::new();
        let queued = [job(1, "a", &none), job(2, "b", &none)];
        assert_eq!(pick(&queued, &none, &none, &served(&[("a", 1), ("b", 2)])), Some(1));
        assert_eq!(pick(&queued, &none, &none, &served(&[("a", 3), ("b", 2)])), Some(2));
    }

    #[test]
    fn within_a_repository_a_retry_goes_first() {
        let none = Needs::new();
        let mut retry = job(2, "a", &none);
        retry.retry = true;
        let queued = [job(1, "a", &none), retry];
        assert_eq!(pick(&queued, &none, &none, &BTreeMap::new()), Some(2));
    }

    #[test]
    fn a_retry_does_not_beat_another_repositorys_turn() {
        let none = Needs::new();
        let mut retry = job(1, "a", &none);
        retry.retry = true;
        let queued = [retry, job(2, "b", &none)];
        assert_eq!(pick(&queued, &none, &none, &served(&[("a", 4)])), Some(2));
    }

    #[test]
    fn next_overrides_everything_else() {
        let none = Needs::new();
        let mut urgent = job(3, "a", &none);
        urgent.priority = true;
        let queued = [job(1, "b", &none), job(2, "c", &none), urgent];
        assert_eq!(pick(&queued, &none, &none, &served(&[("a", 9)])), Some(3));
    }

    #[test]
    fn a_job_waiting_on_a_resource_lets_others_through() {
        let none = Needs::new();
        let simulator = Needs::from([("simulator".to_string(), 1)]);
        let queued = [job(1, "a", &simulator), job(2, "a", &none)];
        assert_eq!(pick(&queued, &simulator, &simulator, &BTreeMap::new()), Some(2));
    }

    #[test]
    fn nothing_fits_means_nothing_starts() {
        let none = Needs::new();
        let simulator = Needs::from([("simulator".to_string(), 1)]);
        let queued = [job(1, "a", &simulator)];
        assert_eq!(pick(&queued, &simulator, &simulator, &BTreeMap::new()), None);
        assert_eq!(pick(&[], &none, &none, &BTreeMap::new()), None);
    }

    #[test]
    fn a_machine_that_lacks_a_resource_is_told_which() {
        let two = Needs::from([("simulator".to_string(), 2)]);
        let one = Needs::from([("simulator".to_string(), 1)]);
        assert_eq!(unavailable(&two, &one), Some("simulator"));
        assert_eq!(unavailable(&one, &one), None);
    }
}
