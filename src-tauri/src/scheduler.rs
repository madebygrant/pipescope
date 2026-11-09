use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, sleep_until};

use crate::error::{Error, Result};
use crate::http::Fetched;
use crate::model::{ConnState, ProviderKind, Repo, Run};
use crate::notify::Change;
use crate::providers::Client;
use crate::{AppState, flaky, notify, set_status_quiet, tray, update_status};

const REPO_LIST_EVERY: Duration = Duration::from_secs(30 * 60);
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(60);
const WARM_WINDOW: chrono::TimeDelta = chrono::TimeDelta::hours(24);

pub enum Control {
    RefreshAll,
    PollRepo(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Hot,
    Warm,
    Cold,
}

pub fn tier(active: bool, last_activity: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Tier {
    if active {
        Tier::Hot
    } else if last_activity.is_some_and(|t| now - t < WARM_WINDOW) {
        Tier::Warm
    } else {
        Tier::Cold
    }
}

pub fn interval(kind: ProviderKind, tier: Tier) -> Duration {
    match (tier, kind) {
        (Tier::Hot, ProviderKind::Github) => Duration::from_secs(10),
        // Bitbucket's budget is a fifth of GitHub's and it has no free 304s.
        (Tier::Hot, ProviderKind::Bitbucket) => Duration::from_secs(20),
        (Tier::Warm, _) => Duration::from_secs(60),
        (Tier::Cold, _) => Duration::from_secs(15 * 60),
    }
}

struct Tracked {
    repo: Repo,
    etag: Option<String>,
    next_due: Instant,
    active: bool,
    newest_run: Option<DateTime<Utc>>,
}

impl Tracked {
    fn tier(&self) -> Tier {
        tier(self.active, self.repo.last_activity.max(self.newest_run), Utc::now())
    }
}

struct Handle {
    task: JoinHandle<()>,
    tx: UnboundedSender<Control>,
}

#[derive(Default)]
pub struct Scheduler {
    loops: Mutex<HashMap<ProviderKind, Handle>>,
}

impl Scheduler {
    pub fn start(&self, app: AppHandle, client: Client) {
        let (tx, rx) = unbounded_channel();
        let kind = client.kind();
        let task = tauri::async_runtime::spawn(run_loop(app, client, rx));
        if let Some(old) = self.loops.lock().unwrap().insert(kind, Handle { task, tx }) {
            old.task.abort();
        }
    }

    pub fn stop(&self, kind: ProviderKind) {
        if let Some(handle) = self.loops.lock().unwrap().remove(&kind) {
            handle.task.abort();
        }
    }

    pub fn send(&self, kind: ProviderKind, control: Control) {
        if let Some(handle) = self.loops.lock().unwrap().get(&kind) {
            let _ = handle.tx.send(control);
        }
    }
}

async fn run_loop(app: AppHandle, client: Client, mut rx: UnboundedReceiver<Control>) {
    let kind = client.kind();
    let mut repos: Vec<Tracked> = Vec::new();
    let mut repos_due = Instant::now();
    loop {
        while let Ok(control) = rx.try_recv() {
            apply(&mut repos, &mut repos_due, control);
        }
        let now = Instant::now();

        if now >= repos_due {
            match client.list_repos().await {
                Ok(list) => {
                    merge(&mut repos, list, now);
                    repos_due = now + REPO_LIST_EVERY;
                    let count = repos.len();
                    update_status(&app, kind, |s| {
                        s.state = ConnState::Connected;
                        s.message = None;
                        s.repos = count;
                    });
                }
                Err(Error::Unauthorized) => return token_rejected(&app, kind),
                Err(e) if e.is_network() => {
                    set_offline(&app, kind, true);
                    repos_due = now + RETRY_AFTER_ERROR;
                }
                Err(e) => {
                    update_status(&app, kind, |s| {
                        s.state = ConnState::Error;
                        s.message = Some(e.to_string());
                    });
                    repos_due = now + RETRY_AFTER_ERROR;
                }
            }
            continue;
        }

        let due = repos
            .iter_mut()
            .filter(|t| t.next_due <= now)
            .min_by_key(|t| (t.next_due, Reverse(t.repo.last_activity)));
        let Some(tracked) = due else {
            let wake = repos.iter().map(|t| t.next_due).min().map_or(repos_due, |d| d.min(repos_due));
            tokio::select! {
                _ = sleep_until(wake) => {}
                control = rx.recv() => match control {
                    Some(control) => apply(&mut repos, &mut repos_due, control),
                    None => return,
                },
            }
            continue;
        };

        match poll(&app, &client, tracked).await {
            Ok(()) => {}
            Err(Error::Unauthorized) => return token_rejected(&app, kind),
            Err(e) if e.is_network() => set_offline(&app, kind, true),
            Err(e) => eprintln!("{} poll of {} failed: {e}", kind.as_str(), tracked.repo.full_name),
        }
    }
}

async fn poll(app: &AppHandle, client: &Client, tracked: &mut Tracked) -> Result<()> {
    let kind = client.kind();
    let fetched = match client.list_runs(&tracked.repo.full_name, tracked.etag.as_deref()).await {
        Ok(fetched) => fetched,
        Err(e) => {
            tracked.next_due = Instant::now() + interval(kind, tracked.tier()).min(RETRY_AFTER_ERROR);
            return Err(e);
        }
    };
    set_offline(app, kind, false);
    let rate = client.rate().await;
    set_status_quiet(app, kind, |s| {
        s.last_poll = Some(Utc::now());
        s.rate = Some(rate);
    });
    if let Fetched::Fresh { body: runs, etag, .. } = fetched {
        tracked.etag = etag;
        tracked.active = runs.iter().any(|r| r.status.is_active());
        tracked.newest_run = runs.iter().map(|r| r.created_at).max();
        let state = app.state::<AppState>();
        let changes = state.store.lock().unwrap().apply_poll(kind, &tracked.repo.full_name, &runs);
        match changes {
            Ok(changes) => {
                let changes = finish_watched(app, &runs, changes);
                let changes = auto_rerun(app, client, &tracked.repo.full_name, changes).await;
                notify::send_all(app, changes);
                time_finished(app, client, &runs).await;
            }
            Err(e) => eprintln!("saving runs for {} failed: {e}", tracked.repo.full_name),
        }
        let _ = app.emit("runs-updated", ());
        tray::refresh(app);
    }
    tracked.next_due = Instant::now() + interval(kind, tracked.tier());
    Ok(())
}

// A finished watched run gets one "Finished" notice in place of any other, then stops being watched.
fn finish_watched(app: &AppHandle, runs: &[Run], mut changes: Vec<Change>) -> Vec<Change> {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    let watched = store.repo_flags("watched");
    for run in runs.iter().filter(|r| r.is_finished() && watched.contains(&r.key)) {
        changes.retain(|c| c.run().key != run.key);
        changes.push(Change::Finished(run.clone()));
        if let Err(e) = store.set_repo_flag("watched", &run.key, false) {
            eprintln!("unwatching {} failed: {e}", run.key);
        }
    }
    changes
}

// Re-runs first failures of workflows that recently passed on retry, when the setting is on,
// and holds back their failure notice: a retry that fails again notifies as usual.
async fn auto_rerun(app: &AppHandle, client: &Client, repo: &str, changes: Vec<Change>) -> Vec<Change> {
    let state = app.state::<AppState>();
    let (history, rules) = {
        let store = state.store.lock().unwrap();
        if store.setting("auto_rerun_flaky").ok().flatten().as_deref() != Some("1") {
            return changes;
        }
        let rules = notify::Rules::load(&store);
        match store.repo_runs(client.kind(), repo) {
            Ok(h) if changes.iter().any(|c| matches!(c, Change::Failed(r) if rules.wants(r))) => (h, rules),
            _ => return changes,
        }
    };
    let mut kept = Vec::new();
    for change in changes {
        let Change::Failed(run) = &change else {
            kept.push(change);
            continue;
        };
        // The stored copy carries `failed_before`, which the fresh one doesn't.
        let stored = history.iter().find(|r| r.key == run.key).unwrap_or(run);
        if !rules.wants(run) || !flaky::should_auto_rerun(&history, stored) {
            kept.push(change);
            continue;
        }
        match client.rerun(run, true).await {
            Ok(()) => {
                eprintln!("re-running flaky {} #{} in {repo}", run.title, run.number);
                after_rerun(app, run);
            }
            Err(e) => {
                eprintln!("automatic re-run of {} failed: {e}", run.key);
                kept.push(change);
            }
        }
    }
    kept
}

// Polls straight away so the retry shows, and for GitHub marks the run queued so a fast
// second failure is still seen as a change.
pub fn after_rerun(app: &AppHandle, run: &Run) {
    let state = app.state::<AppState>();
    if run.provider == ProviderKind::Github
        && let Err(e) = state.store.lock().unwrap().mark_rerun(&run.key)
    {
        eprintln!("marking {} re-run failed: {e}", run.key);
    }
    state.scheduler.send(run.provider, Control::PollRepo(run.repo.clone()));
}

// Only runs that finished recently, so a first sweep doesn't spend an API call on every old run.
const TIME_RUNS_FINISHED_WITHIN: chrono::TimeDelta = chrono::TimeDelta::hours(6);

async fn time_finished(app: &AppHandle, client: &Client, runs: &[Run]) {
    if client.kind() != ProviderKind::Github {
        return;
    }
    let cutoff = Utc::now() - TIME_RUNS_FINISHED_WITHIN;
    let state = app.state::<AppState>();
    let due: Vec<&Run> = {
        let store = state.store.lock().unwrap();
        runs.iter()
            .filter(|r| r.is_finished() && r.updated_at > cutoff && store.needs_build_time(r).unwrap_or(false))
            .collect()
    };
    for run in due {
        match client.build_secs(run).await {
            Ok(Some(secs)) => {
                if let Err(e) = state.store.lock().unwrap().set_build_time(run, secs) {
                    eprintln!("saving build time for {} failed: {e}", run.key);
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("timing {} failed: {e}", run.key),
        }
    }
}

// Only transitions emit, so a long outage doesn't spam the UI.
fn set_offline(app: &AppHandle, kind: ProviderKind, offline: bool) {
    let current = app.state::<AppState>().status.lock().unwrap().get(&kind).map(|s| s.state);
    match (offline, current) {
        (true, Some(ConnState::Connected)) => update_status(app, kind, |s| {
            s.state = ConnState::Offline;
            s.message = Some("Offline. Retrying every minute.".into());
        }),
        (false, Some(ConnState::Offline)) => update_status(app, kind, |s| {
            s.state = ConnState::Connected;
            s.message = None;
        }),
        _ => {}
    }
}

fn token_rejected(app: &AppHandle, kind: ProviderKind) {
    update_status(app, kind, |s| {
        s.state = ConnState::Error;
        s.message = Some("Token rejected. Update it in Settings.".into());
    });
}

fn apply(repos: &mut [Tracked], repos_due: &mut Instant, control: Control) {
    let now = Instant::now();
    match control {
        Control::RefreshAll => {
            *repos_due = now;
            repos.iter_mut().for_each(|t| t.next_due = now);
        }
        Control::PollRepo(name) => {
            if let Some(t) = repos.iter_mut().find(|t| t.repo.full_name == name) {
                t.next_due = now;
                // Keep it hot while a re-run or cancel takes effect.
                t.active = true;
            }
        }
    }
}

fn merge(repos: &mut Vec<Tracked>, list: Vec<Repo>, now: Instant) {
    let mut old: HashMap<String, Tracked> = repos.drain(..).map(|t| (t.repo.full_name.clone(), t)).collect();
    *repos = list
        .into_iter()
        .map(|repo| match old.remove(&repo.full_name) {
            Some(mut t) => {
                // A push since the last listing: poll now rather than waiting out a cold interval.
                if repo.last_activity > t.repo.last_activity {
                    t.next_due = now;
                }
                t.repo = repo;
                t
            }
            None => Tracked { repo, etag: None, next_due: now, active: false, newest_run: None },
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn assigns_tiers() {
        let now = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
        assert_eq!(tier(true, None, now), Tier::Hot);
        assert_eq!(tier(false, Some(now - chrono::TimeDelta::hours(2)), now), Tier::Warm);
        assert_eq!(tier(false, Some(now - chrono::TimeDelta::days(3)), now), Tier::Cold);
        assert_eq!(tier(false, None, now), Tier::Cold);
    }

    #[test]
    fn merge_keeps_state_and_wakes_pushed_repos() {
        let t0 = Utc.with_ymd_and_hms(2026, 11, 9, 0, 0, 0).unwrap();
        let start = Instant::now();
        let later = start + Duration::from_secs(600);
        let repo = |name: &str, at| Repo { full_name: name.into(), last_activity: Some(at) };
        let mut repos = vec![
            Tracked { repo: repo("a/quiet", t0), etag: Some("e1".into()), next_due: later, active: false, newest_run: None },
            Tracked { repo: repo("a/pushed", t0), etag: None, next_due: later, active: false, newest_run: None },
        ];
        let pushed_at = t0 + chrono::TimeDelta::minutes(5);
        merge(&mut repos, vec![repo("a/quiet", t0), repo("a/pushed", pushed_at), repo("a/new", t0)], start);

        assert_eq!(repos[0].etag.as_deref(), Some("e1"));
        assert_eq!(repos[0].next_due, later);
        assert_eq!(repos[1].next_due, start);
        assert_eq!(repos[2].next_due, start);
    }
}
