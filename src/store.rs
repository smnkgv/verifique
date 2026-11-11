use std::fs;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Row};
use serde::Serialize;

use crate::config::{Needs, Plan};
use crate::paths;

// `released` is what a started stack ended as before it was called `stopped`.
pub const TERMINAL: [&str; 8] = ["passed", "failed", "blocked", "error", "superseded", "cancelled", "stopped", "released"];

const LOCK_WAIT: Duration = Duration::from_secs(10);

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS jobs (
        id INTEGER PRIMARY KEY,
        repo TEXT NOT NULL,
        branch TEXT,
        commit_sha TEXT NOT NULL,
        tree TEXT NOT NULL,
        base TEXT,
        check_name TEXT NOT NULL,
        request TEXT NOT NULL,
        brief TEXT NOT NULL,
        plan TEXT NOT NULL,
        needs TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'queued',
        retry INTEGER NOT NULL DEFAULT 0,
        priority INTEGER NOT NULL DEFAULT 0,
        slots TEXT NOT NULL DEFAULT '',
        pgids TEXT NOT NULL DEFAULT '',
        hold_until INTEGER,
        release_requested INTEGER NOT NULL DEFAULT 0,
        summary TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        started_at TEXT,
        finished_at TEXT
    );
    CREATE TABLE IF NOT EXISTS branches (
        repo TEXT NOT NULL,
        branch TEXT NOT NULL,
        check_name TEXT NOT NULL,
        failures INTEGER NOT NULL DEFAULT 0,
        needs_you INTEGER NOT NULL DEFAULT 0,
        reason TEXT,
        PRIMARY KEY (repo, branch, check_name)
    );
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS rebuilds (
        job INTEGER PRIMARY KEY,
        commit_sha TEXT NOT NULL,
        tree TEXT NOT NULL,
        fresh INTEGER NOT NULL,
        minutes REAL,
        status TEXT NOT NULL DEFAULT 'requested',
        summary TEXT
    );
";

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: i64,
    pub repo: String,
    pub branch: Option<String>,
    pub commit: String,
    pub tree: String,
    pub base: Option<String>,
    pub check: String,
    pub request: String,
    pub brief: String,
    #[serde(skip)]
    pub plan: Plan,
    /// False for a plan written by another version, which the server ends instead of running.
    #[serde(skip)]
    pub plan_readable: bool,
    pub needs: Needs,
    pub status: String,
    pub retry: bool,
    pub priority: bool,
    pub slots: Vec<u32>,
    /// Process trees the job started, so a later server can end what a crashed one left behind.
    pub trees: Vec<u32>,
    pub hold_until: Option<u64>,
    pub summary: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NeedsYou {
    pub repo: String,
    pub branch: String,
    pub check: String,
    pub failures: u32,
    pub reason: String,
}

/// A started stack asked to take newer work. `status` goes requested, running, then done or failed.
#[derive(Debug)]
pub struct Rebuild {
    pub commit: String,
    pub tree: String,
    pub fresh: bool,
    pub minutes: Option<f64>,
    pub status: String,
    pub summary: Option<String>,
}

pub struct Submission<'a> {
    pub repo: &'a str,
    pub branch: Option<&'a str>,
    pub commit: &'a str,
    pub tree: &'a str,
    pub base: Option<&'a str>,
    pub check: &'a str,
    pub request: &'a str,
    pub brief: &'a str,
    pub plan: &'a Plan,
    pub needs: &'a Needs,
}

fn numbers<T: std::str::FromStr>(list: &str) -> Vec<T> {
    list.split(',').filter_map(|item| item.parse().ok()).collect()
}

fn job_from(row: &Row) -> rusqlite::Result<Job> {
    let json = |index: usize, error: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
    };
    let plan = serde_json::from_str::<Plan>(&row.get::<_, String>("plan")?);
    let needs: String = row.get("needs")?;
    Ok(Job {
        id: row.get("id")?,
        repo: row.get("repo")?,
        branch: row.get("branch")?,
        commit: row.get("commit_sha")?,
        tree: row.get("tree")?,
        base: row.get("base")?,
        check: row.get("check_name")?,
        request: row.get("request")?,
        brief: row.get("brief")?,
        plan_readable: plan.is_ok(),
        plan: plan.unwrap_or_default(),
        needs: serde_json::from_str(&needs).map_err(|error| json(0, error))?,
        status: row.get("status")?,
        retry: row.get("retry")?,
        priority: row.get("priority")?,
        slots: numbers(&row.get::<_, String>("slots")?),
        trees: numbers(&row.get::<_, String>("pgids")?),
        hold_until: row.get::<_, Option<i64>>("hold_until")?.map(|until| until as u64),
        summary: row.get("summary")?,
        created_at: row.get("created_at")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
    })
}

// Switching a new database to WAL takes an exclusive lock and reports busy at once, without
// honouring the busy timeout, so a swarm opening it for the first time has to retry by hand.
fn prepare(db: &Connection) -> Result<()> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let result = db
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
            .and_then(|_| db.execute_batch(SCHEMA));
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy) && Instant::now() < deadline => {
                sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub struct Store {
    db: Connection,
}

impl Store {
    pub fn open() -> Result<Self> {
        let home = paths::home();
        fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
        let db = Connection::open(home.join("queue.db"))?;
        // Agents submit from many processes while the server claims and finishes jobs.
        db.busy_timeout(LOCK_WAIT)?;
        prepare(&db)?;
        Ok(Self { db })
    }

    /// A newer submission makes a queued run of the same check on that branch pointless, so it
    /// takes over. Different checks of the same work all stay.
    pub fn submit(&mut self, job: Submission) -> Result<i64> {
        let retry = job.branch.is_some_and(|branch| self.failures(job.repo, branch, job.check).unwrap_or(0) > 0);
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO jobs (repo, branch, commit_sha, tree, base, check_name, request, brief, plan, needs, retry)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                job.repo,
                job.branch,
                job.commit,
                job.tree,
                job.base,
                job.check,
                job.request,
                job.brief,
                serde_json::to_string(job.plan)?,
                serde_json::to_string(job.needs)?,
                retry,
            ],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(branch) = job.branch {
            tx.execute(
                "UPDATE jobs SET status = 'superseded', summary = ?1, finished_at = datetime('now')
                 WHERE repo = ?2 AND branch = ?3 AND check_name = ?4 AND status = 'queued' AND id != ?5",
                params![format!("replaced by job {id}"), job.repo, branch, job.check, id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    fn failures(&self, repo: &str, branch: &str, check: &str) -> Result<u32> {
        Ok(self
            .db
            .query_row(
                "SELECT failures FROM branches WHERE repo = ?1 AND branch = ?2 AND check_name = ?3",
                params![repo, branch, check],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn needs_you_reason(&self, repo: &str, branch: &str, check: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT reason FROM branches
                 WHERE repo = ?1 AND branch = ?2 AND check_name = ?3 AND needs_you = 1",
                params![repo, branch, check],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn jobs(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<Job>> {
        let mut statement = self.db.prepare(&format!("SELECT * FROM jobs {filter}"))?;
        let rows = statement.query_map(args, job_from)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn queued(&self) -> Result<Vec<Job>> {
        self.jobs("WHERE status = 'queued' ORDER BY id", [])
    }

    pub fn running(&self) -> Result<Vec<Job>> {
        self.jobs("WHERE status = 'running' ORDER BY id", [])
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<Job>> {
        self.jobs("ORDER BY id DESC LIMIT ?1", [limit])
    }

    pub fn finished(&self, limit: u32) -> Result<Vec<Job>> {
        self.jobs("WHERE finished_at IS NOT NULL AND status != 'superseded' ORDER BY finished_at DESC, id DESC LIMIT ?1", [limit])
    }

    pub fn get(&self, id: i64) -> Result<Option<Job>> {
        Ok(self.jobs("WHERE id = ?1", [id])?.pop())
    }

    pub fn claim(&self, id: i64, slots: &[u32]) -> Result<()> {
        let slots: Vec<String> = slots.iter().map(u32::to_string).collect();
        self.db.execute(
            "UPDATE jobs SET status = 'running', slots = ?1, started_at = datetime('now') WHERE id = ?2",
            params![slots.join(","), id],
        )?;
        Ok(())
    }

    /// Remember a process tree, so a later server can clear what a crashed one left behind.
    pub fn add_tree(&self, id: i64, tree: u32) -> Result<()> {
        self.db.execute(
            "UPDATE jobs SET pgids = CASE pgids WHEN '' THEN ?1 ELSE pgids || ',' || ?1 END WHERE id = ?2",
            params![tree.to_string(), id],
        )?;
        Ok(())
    }

    pub fn requeue(&self, id: i64) -> Result<()> {
        self.db.execute(
            "UPDATE jobs SET status = 'queued', slots = '', pgids = '', started_at = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Says a held stack is up, and until when.
    pub fn set_hold_until(&self, id: i64, until: u64) -> Result<()> {
        self.db.execute("UPDATE jobs SET hold_until = ?1 WHERE id = ?2", params![until as i64, id])?;
        Ok(())
    }

    pub fn request_release(&self, id: i64) -> Result<()> {
        self.db.execute("UPDATE jobs SET release_requested = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn release_requested(&self, id: i64) -> Result<bool> {
        Ok(self
            .db
            .query_row("SELECT release_requested FROM jobs WHERE id = ?1", [id], |row| row.get(0))?)
    }

    /// False while an earlier rebuild of the same stack is still waiting or running.
    pub fn request_rebuild(&mut self, id: i64, commit: &str, tree: &str, fresh: bool, minutes: Option<f64>) -> Result<bool> {
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM rebuilds WHERE job = ?1 AND status IN ('done', 'failed')", [id])?;
        let added = tx.execute(
            "INSERT OR IGNORE INTO rebuilds (job, commit_sha, tree, fresh, minutes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, commit, tree, fresh, minutes],
        )?;
        tx.commit()?;
        Ok(added == 1)
    }

    pub fn rebuild(&self, id: i64) -> Result<Option<Rebuild>> {
        Ok(self
            .db
            .query_row(
                "SELECT commit_sha, tree, fresh, minutes, status, summary FROM rebuilds WHERE job = ?1",
                [id],
                |row| {
                    Ok(Rebuild {
                        commit: row.get(0)?,
                        tree: row.get(1)?,
                        fresh: row.get(2)?,
                        minutes: row.get(3)?,
                        status: row.get(4)?,
                        summary: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// The waiting rebuild of a stack, now marked as running, if there is one.
    pub fn take_rebuild(&self, id: i64) -> Result<Option<Rebuild>> {
        let taken = self
            .db
            .execute("UPDATE rebuilds SET status = 'running' WHERE job = ?1 AND status = 'requested'", [id])?;
        if taken == 0 {
            return Ok(None);
        }
        self.rebuild(id)
    }

    pub fn end_rebuild(&self, id: i64, status: &str, summary: &str) -> Result<()> {
        self.db
            .execute("UPDATE rebuilds SET status = ?1, summary = ?2 WHERE job = ?3", params![status, summary, id])?;
        Ok(())
    }

    /// The content a started stack now runs, after a rebuild put newer work in its slot.
    pub fn set_content(&self, id: i64, commit: &str, tree: &str) -> Result<()> {
        self.db
            .execute("UPDATE jobs SET commit_sha = ?1, tree = ?2 WHERE id = ?3", params![commit, tree, id])?;
        Ok(())
    }

    pub fn prioritise(&self, id: i64) -> Result<bool> {
        let changed = self
            .db
            .execute("UPDATE jobs SET priority = 1 WHERE id = ?1 AND status = 'queued'", [id])?;
        Ok(changed > 0)
    }

    /// Record a job's end and what it means for its branch. Returns the reason when the branch
    /// has just landed in `needs-you`.
    pub fn finish(&mut self, job: &Job, status: &str, summary: &str) -> Result<Option<String>> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE jobs SET status = ?1, summary = ?2, pgids = '', finished_at = datetime('now')
             WHERE id = ?3",
            params![status, summary, job.id],
        )?;
        let mut reason = None;
        if let Some(branch) = &job.branch {
            let key = params![job.repo, branch, job.check];
            tx.execute(
                "INSERT OR IGNORE INTO branches (repo, branch, check_name) VALUES (?1, ?2, ?3)",
                key,
            )?;
            let failures: u32 = tx.query_row(
                "SELECT failures FROM branches WHERE repo = ?1 AND branch = ?2 AND check_name = ?3",
                key,
                |row| row.get(0),
            )?;
            let (failures, needs_you) = match status {
                "passed" => (0, None),
                "failed" if failures + 1 >= job.plan.retries => {
                    (failures + 1, Some(format!("failed {} times in a row: {summary}", failures + 1)))
                }
                "failed" => (failures + 1, None),
                "blocked" => (failures, Some(format!("blocked: {summary}"))),
                // A crash says nothing about the agent's work, so it leaves the branch as it was.
                _ => {
                    tx.commit()?;
                    return Ok(None);
                }
            };
            tx.execute(
                "UPDATE branches SET failures = ?1, needs_you = ?2, reason = ?3
                 WHERE repo = ?4 AND branch = ?5 AND check_name = ?6",
                params![failures, needs_you.is_some(), needs_you, job.repo, branch, job.check],
            )?;
            reason = needs_you;
        }
        tx.commit()?;
        Ok(reason)
    }

    pub fn needs_you(&self) -> Result<Vec<NeedsYou>> {
        let mut statement = self.db.prepare(
            "SELECT repo, branch, check_name, failures, reason FROM branches
             WHERE needs_you = 1 ORDER BY repo, branch, check_name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(NeedsYou {
                repo: row.get(0)?,
                branch: row.get(1)?,
                check: row.get(2)?,
                failures: row.get(3)?,
                reason: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Hand a branch back to its agents, with a fresh count. Returns how many checks changed.
    pub fn clear(&self, repo: &str, branch: &str, check: Option<&str>) -> Result<usize> {
        Ok(self.db.execute(
            "UPDATE branches SET failures = 0, needs_you = 0, reason = NULL
             WHERE repo = ?1 AND branch = ?2 AND (?3 IS NULL OR check_name = ?3)",
            params![repo, branch, check],
        )?)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .optional()?)
    }

    pub fn unset(&self, key: &str) -> Result<()> {
        self.db.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}
