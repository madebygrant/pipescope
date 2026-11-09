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
";

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
        {
            let mut stmt = tx.prepare("SELECT status FROM runs WHERE key = ?1")?;
            for run in runs {
                let status: Option<String> = stmt.query_row([&run.key], |r| r.get(0)).optional()?;
                if let Some(status) = status.as_deref().and_then(RunStatus::parse) {
                    old.insert(run.key.clone(), status);
                }
            }
            let mut upsert = tx.prepare(
                "INSERT INTO runs (key, provider, repo, status, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (key) DO UPDATE SET status = excluded.status, created_at = excluded.created_at, data = excluded.data",
            )?;
            for run in runs {
                upsert.execute(params![
                    run.key,
                    kind.as_str(),
                    repo,
                    run.status.as_str(),
                    timestamp(run.created_at),
                    serde_json::to_string(run)?
                ])?;
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

    pub fn get_run(&self, key: &str) -> Result<Option<Run>> {
        Ok(query_runs(&self.conn, "SELECT data FROM runs WHERE key = ?1", [key])?.pop())
    }

    pub fn clear_provider(&self, kind: ProviderKind) -> Result<()> {
        self.conn.execute("DELETE FROM runs WHERE provider = ?1", [kind.as_str()])?;
        self.conn.execute("DELETE FROM polled_repos WHERE provider = ?1", [kind.as_str()])?;
        Ok(())
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

    // `flag` is "pinned" or "muted"; values are `Run::repo_id`s.
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
