use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Params, params};

use crate::error::Result;
use crate::model::{ProviderKind, Run, RunStatus};
use crate::notify::{Change, detect_changes};

const KEEP_PER_REPO: i64 = 50;
// Per repo rather than overall, so busy repos can't push quieter repos' failures out.
const LIST_PER_REPO: i64 = 20;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    key TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    repo TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_by_repo ON runs (provider, repo, created_at);
CREATE INDEX IF NOT EXISTS runs_by_time ON runs (created_at);
CREATE TABLE IF NOT EXISTS polled_repos (
    provider TEXT NOT NULL,
    repo TEXT NOT NULL,
    PRIMARY KEY (provider, repo)
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Kept apart from runs, which are pruned to the newest 50 per repo, so monthly totals survive.
CREATE TABLE IF NOT EXISTS build_time (
    key TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    repo TEXT NOT NULL,
    month TEXT NOT NULL,
    secs INTEGER NOT NULL,
    -- The run's updated_at when timed, so a re-run is timed again.
    updated TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS build_time_by_month ON build_time (month);
";

#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildTime {
    pub repo_id: String,
    pub secs: u64,
    pub runs: u64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // Saves one poll of a repo and returns the status changes worth notifying about.
    // The first poll of a repo is a baseline and never notifies.
    pub fn apply_poll(&mut self, kind: ProviderKind, repo: &str, runs: &[Run]) -> Result<Vec<Change>> {
        let tx = self.conn.transaction()?;
        let baseline = tx
            .query_row(
                "SELECT 1 FROM polled_repos WHERE provider = ?1 AND repo = ?2",
                params![kind.as_str(), repo],
                |_| Ok(()),
            )
            .optional()?
            .is_none();

        let mut old = HashMap::new();
        let mut failed_before = HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT data FROM runs WHERE key = ?1")?;
            for run in runs {
                let data: Option<String> = stmt.query_row([&run.key], |r| r.get(0)).optional()?;
                let Some(prev) = data.and_then(|d| serde_json::from_str::<Run>(&d).ok()) else { continue };
                // A GitHub re-run keeps the run's id, so this is the only trace of the failed attempt.
                if prev.failed_before || (prev.status == RunStatus::Failed && run.status != RunStatus::Failed) {
                    failed_before.insert(run.key.clone());
                }
                old.insert(run.key.clone(), prev.status);
            }
            let mut upsert = tx.prepare(
                "INSERT INTO runs (key, provider, repo, status, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (key) DO UPDATE SET status = excluded.status, created_at = excluded.created_at, data = excluded.data",
            )?;
            let mut time = tx.prepare(
                "INSERT INTO build_time (key, provider, repo, month, secs, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (key) DO UPDATE SET secs = excluded.secs, updated = excluded.updated",
            )?;
            for run in runs {
                let mut stored = run.clone();
                stored.failed_before = failed_before.contains(&run.key);
                upsert.execute(params![
                    run.key,
                    kind.as_str(),
                    repo,
                    run.status.as_str(),
                    timestamp(run.created_at),
                    serde_json::to_string(&stored)?
                ])?;
                if let Some(secs) = run.build_secs {
                    time.execute(params![run.key, kind.as_str(), repo, month(run.created_at), secs as i64, timestamp(run.updated_at)])?;
                }
            }
        }
        tx.execute("INSERT OR IGNORE INTO polled_repos (provider, repo) VALUES (?1, ?2)", params![kind.as_str(), repo])?;
        tx.execute(
            "DELETE FROM runs WHERE provider = ?1 AND repo = ?2 AND key NOT IN
             (SELECT key FROM runs WHERE provider = ?1 AND repo = ?2 ORDER BY created_at DESC LIMIT ?3)",
            params![kind.as_str(), repo, KEEP_PER_REPO],
        )?;
        let history = query_runs(&tx, "SELECT data FROM runs WHERE provider = ?1 AND repo = ?2", params![kind.as_str(), repo])?;
        tx.commit()?;
        self.prune_watched()?;
        Ok(detect_changes(baseline, &old, runs, &history))
    }

    pub fn recent_runs(&self, since: DateTime<Utc>) -> Result<Vec<Run>> {
        query_runs(
            &self.conn,
            "SELECT data FROM (
                SELECT data, created_at,
                       ROW_NUMBER() OVER (PARTITION BY provider, repo ORDER BY created_at DESC) AS rank
                FROM runs WHERE created_at >= ?1
             ) WHERE rank <= ?2 ORDER BY created_at DESC",
            params![timestamp(since), LIST_PER_REPO],
        )
    }

    pub fn needs_build_time(&self, run: &Run) -> Result<bool> {
        let updated: Option<String> =
            self.conn.query_row("SELECT updated FROM build_time WHERE key = ?1", [&run.key], |r| r.get(0)).optional()?;
        Ok(updated.as_deref() != Some(timestamp(run.updated_at).as_str()))
    }

    pub fn set_build_time(&self, run: &Run, secs: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO build_time (key, provider, repo, month, secs, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (key) DO UPDATE SET secs = excluded.secs, updated = excluded.updated",
            params![run.key, run.provider.as_str(), run.repo, month(run.created_at), secs as i64, timestamp(run.updated_at)],
        )?;
        Ok(())
    }

    // After a GitHub re-run, which keeps the run's id: shows it queued at once, and makes
    // the next poll see Queued → Failed if the retry fails, so that failure still notifies.
    pub fn mark_rerun(&self, key: &str) -> Result<()> {
        let Some(mut run) = self.get_run(key)? else { return Ok(()) };
        run.status = RunStatus::Queued;
        run.failed_before = true;
        self.conn.execute(
            "UPDATE runs SET status = ?2, data = ?3 WHERE key = ?1",
            params![key, run.status.as_str(), serde_json::to_string(&run)?],
        )?;
        Ok(())
    }

    // A watched run that never shows up finished (pruned, or its provider disconnected) would stay forever.
    fn prune_watched(&self) -> Result<()> {
        let watched = self.repo_flags("watched");
        let mut stmt = self.conn.prepare("SELECT 1 FROM runs WHERE key = ?1")?;
        let mut keep = Vec::new();
        for key in &watched {
            if stmt.exists([key])? {
                keep.push(key.clone());
            }
        }
        if keep.len() < watched.len() {
            keep.sort();
            self.set_setting("watched", Some(&serde_json::to_string(&keep)?))?;
        }
        Ok(())
    }

    pub fn all_runs(&self) -> Result<Vec<Run>> {
        query_runs(&self.conn, "SELECT data FROM runs", [])
    }

    pub fn repo_runs(&self, kind: ProviderKind, repo: &str) -> Result<Vec<Run>> {
        query_runs(
            &self.conn,
            "SELECT data FROM runs WHERE provider = ?1 AND repo = ?2 ORDER BY created_at DESC",
            params![kind.as_str(), repo],
        )
    }

    // `month` is "YYYY-MM" in UTC.
    pub fn build_time(&self, month: &str) -> Result<Vec<BuildTime>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider || ':' || repo, SUM(secs), COUNT(*) FROM build_time WHERE month = ?1
             GROUP BY provider, repo ORDER BY SUM(secs) DESC",
        )?;
        let rows = stmt.query_map([month], |r| {
            Ok(BuildTime { repo_id: r.get(0)?, secs: r.get::<_, i64>(1)? as u64, runs: r.get::<_, i64>(2)? as u64 })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn clients(&self) -> HashMap<String, String> {
        self.setting("clients").ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    pub fn set_client(&self, repo_id: &str, client: Option<&str>) -> Result<()> {
        let mut clients = self.clients();
        match client.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => clients.insert(repo_id.to_string(), c.to_string()),
            None => clients.remove(repo_id),
        };
        self.set_setting("clients", Some(&serde_json::to_string(&clients)?))
    }

    pub fn get_run(&self, key: &str) -> Result<Option<Run>> {
        Ok(query_runs(&self.conn, "SELECT data FROM runs WHERE key = ?1", [key])?.pop())
    }

    pub fn clear_provider(&self, kind: ProviderKind) -> Result<()> {
        self.conn.execute("DELETE FROM runs WHERE provider = ?1", [kind.as_str()])?;
        self.conn.execute("DELETE FROM polled_repos WHERE provider = ?1", [kind.as_str()])?;
        self.conn.execute("DELETE FROM build_time WHERE provider = ?1", [kind.as_str()])?;
        self.prune_watched()
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, v],
            )?,
            None => self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    // `flag` is "pinned" or "muted" with `Run::repo_id`s, or "watched" with run keys.
    pub fn repo_flags(&self, flag: &str) -> HashSet<String> {
        self.setting(flag).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    pub fn set_repo_flag(&self, flag: &str, repo_id: &str, on: bool) -> Result<()> {
        let mut ids = self.repo_flags(flag);
        if on {
            ids.insert(repo_id.to_string());
        } else {
            ids.remove(repo_id);
        }
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        self.set_setting(flag, Some(&serde_json::to_string(&ids)?))
    }

    pub fn notify_all(&self) -> bool {
        matches!(self.setting("notify_all"), Ok(Some(v)) if v == "1")
    }
}

fn query_runs(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Run>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
    let mut runs = Vec::new();
    for row in rows {
        runs.push(serde_json::from_str(&row?)?);
    }
    Ok(runs)
}

pub fn month(t: DateTime<Utc>) -> String {
    t.format("%Y-%m").to_string()
}

// Fixed-width UTC so text comparison in SQL orders correctly.
fn timestamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::run_key;
    use chrono::TimeZone;

    fn run(id: &str, minute: u32, status: RunStatus) -> Run {
        let t = Utc.with_ymd_and_hms(2026, 11, 9, 1, minute, 0).unwrap();
        Run {
            key: run_key(ProviderKind::Github, "o/r", id),
            provider: ProviderKind::Github,
            repo: "o/r".into(),
            id: id.into(),
            number: minute as u64,
            title: "CI".into(),
            subtitle: None,
            branch: Some("main".into()),
            commit: None,
            status,
            actor: None,
            mine: true,
            created_at: t,
            started_at: None,
            updated_at: t,
            web_url: String::new(),
            author: None,
            build_secs: None,
            failed_before: false,
            flaky: false,
            flaky_workflow: false,
            watched: false,
        }
    }

    #[test]
    fn first_poll_is_baseline_then_changes_notify() {
        let mut store = Store::in_memory().unwrap();
        let changes = store.apply_poll(ProviderKind::Github, "o/r", &[run("1", 0, RunStatus::Failed)]).unwrap();
        assert!(changes.is_empty());

        let changes = store
            .apply_poll(ProviderKind::Github, "o/r", &[run("2", 5, RunStatus::Running), run("1", 0, RunStatus::Failed)])
            .unwrap();
        assert!(changes.is_empty());

        let changes = store
            .apply_poll(ProviderKind::Github, "o/r", &[run("2", 5, RunStatus::Success), run("1", 0, RunStatus::Failed)])
            .unwrap();
        assert!(matches!(changes.as_slice(), [Change::Recovered(r)] if r.id == "2"));

        let runs = store.recent_runs(Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap()).unwrap();
        assert_eq!(runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["2", "1"]);
    }

    #[test]
    fn recent_runs_caps_each_repo_separately() {
        let mut store = Store::in_memory().unwrap();
        let busy: Vec<Run> = (0..30).map(|i| run(&format!("b{i}"), i, RunStatus::Success)).collect();
        store.apply_poll(ProviderKind::Github, "o/r", &busy).unwrap();
        let mut quiet = run("q", 0, RunStatus::Failed);
        quiet.repo = "o/quiet".into();
        quiet.key = run_key(ProviderKind::Github, "o/quiet", "q");
        store.apply_poll(ProviderKind::Github, "o/quiet", &[quiet]).unwrap();

        let runs = store.recent_runs(Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap()).unwrap();
        assert_eq!(runs.iter().filter(|r| r.repo == "o/r").count(), LIST_PER_REPO as usize);
        assert!(runs.iter().any(|r| r.repo == "o/quiet"));
    }

    #[test]
    fn remembers_a_failure_through_a_rerun_and_totals_build_time() {
        let mut store = Store::in_memory().unwrap();
        let mut failed = run("1", 0, RunStatus::Failed);
        failed.build_secs = Some(60);
        store.apply_poll(ProviderKind::Github, "o/r", std::slice::from_ref(&failed)).unwrap();
        store.apply_poll(ProviderKind::Github, "o/r", &[run("1", 0, RunStatus::Running)]).unwrap();
        let mut passed = run("1", 0, RunStatus::Success);
        passed.build_secs = Some(90);
        store.apply_poll(ProviderKind::Github, "o/r", &[passed, run("2", 1, RunStatus::Success)]).unwrap();

        let runs = store.repo_runs(ProviderKind::Github, "o/r").unwrap();
        assert!(runs.iter().find(|r| r.id == "1").unwrap().failed_before);
        assert!(!runs.iter().find(|r| r.id == "2").unwrap().failed_before);
        // A later timing of the same run replaces the earlier one; GitHub's cover every attempt.
        assert_eq!(
            store.build_time("2026-11").unwrap(),
            [BuildTime { repo_id: "github:o/r".into(), secs: 90, runs: 1 }]
        );
        assert!(store.build_time("2026-10").unwrap().is_empty());
    }

    #[test]
    fn rerun_marks_and_watch_pruning() {
        let mut store = Store::in_memory().unwrap();
        store.apply_poll(ProviderKind::Github, "o/r", &[run("1", 0, RunStatus::Failed)]).unwrap();
        let key = run_key(ProviderKind::Github, "o/r", "1");
        store.mark_rerun(&key).unwrap();
        let marked = store.get_run(&key).unwrap().unwrap();
        assert_eq!(marked.status, RunStatus::Queued);
        assert!(marked.failed_before);

        let changes = store.apply_poll(ProviderKind::Github, "o/r", &[run("1", 0, RunStatus::Failed)]).unwrap();
        assert!(matches!(changes.as_slice(), [Change::Failed(_)]), "a retry that fails again notifies");

        store.set_repo_flag("watched", &key, true).unwrap();
        store.set_repo_flag("watched", "github:o/r:gone", true).unwrap();
        store.apply_poll(ProviderKind::Github, "o/r", &[run("1", 0, RunStatus::Failed)]).unwrap();
        assert_eq!(store.repo_flags("watched"), HashSet::from([key]));
    }

    #[test]
    fn clients_round_trip() {
        let store = Store::in_memory().unwrap();
        store.set_client("github:o/r", Some(" Acme ")).unwrap();
        assert_eq!(store.clients().get("github:o/r").map(String::as_str), Some("Acme"));
        store.set_client("github:o/r", Some("")).unwrap();
        assert!(store.clients().is_empty());
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::in_memory().unwrap();
        assert!(!store.notify_all());
        store.set_setting("notify_all", Some("1")).unwrap();
        assert!(store.notify_all());
        store.set_setting("notify_all", None).unwrap();
        assert_eq!(store.setting("notify_all").unwrap(), None);

        store.set_repo_flag("muted", "github:o/r", true).unwrap();
        store.set_repo_flag("muted", "github:o/s", true).unwrap();
        store.set_repo_flag("muted", "github:o/r", false).unwrap();
        assert_eq!(store.repo_flags("muted"), HashSet::from(["github:o/s".to_string()]));
        assert!(store.repo_flags("pinned").is_empty());
    }
}
