use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::io::AsyncReadExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::error::{Error, Result};
use crate::model::{LogTail, Run, RunStatus, StepPick};
use crate::scheduler::Control;
use crate::{AppState, commands, tray};

// Matches the tray: an older failure no longer needs attention.
const FAILURE_WINDOW: TimeDelta = TimeDelta::hours(48);
const WATCH_TICK: Duration = Duration::from_secs(5);
// Until a run appears, asks for a poll this often. After that the scheduler's hot tier keeps it fresh.
const WATCH_POLL: Duration = Duration::from_secs(15);
// Runs that haven't changed since the watch began are only trusted once a poll has had time to land,
// so a re-run the store hasn't seen yet can't report the previous result.
const FIRST_POLL_GRACE: Duration = Duration::from_secs(20);
// A request is one short JSON line.
const MAX_REQUEST: u64 = 64 * 1024;
// sun_path on macOS holds 104 bytes, including the trailing NUL.
const MAX_SOCKET_PATH: usize = 103;

// The app's data folder, which only this user can read. The CLI finds it without Tauri.
pub fn socket_path(home: &Path) -> PathBuf {
    home.join("Library/Application Support").join(crate::IDENTIFIER).join("cli.sock")
}

pub fn socket_path_fits(path: &Path) -> bool {
    path.as_os_str().len() <= MAX_SOCKET_PATH
}

pub const PATH_TOO_LONG: &str =
    "Your home folder's path is too long for Pipescope's command line socket, so the command line can't work.";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Failures {
        all: bool,
        repo: Option<String>,
    },
    // With no run, the newest failure in `repo`, preferring `branch`.
    Log {
        run: Option<String>,
        repo: Option<String>,
        branch: Option<String>,
        step: Option<String>,
    },
    // Matches runs of `commit`, on `branch` too if given; with no commit, runs on `branch` from now on.
    Watch {
        repo: String,
        branch: Option<String>,
        commit: Option<String>,
        timeout_secs: u64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Reply {
    Progress { message: String },
    Done { data: serde_json::Value },
    Error { message: String },
}

#[derive(Serialize, Deserialize)]
pub struct Failure {
    #[serde(flatten)]
    pub run: Run,
    pub checkout: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct LogReply {
    pub run: Run,
    pub log: LogTail,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Passed,
    Failed,
    // Paused on an approval or manual step, which could take hours.
    Waiting,
}

#[derive(Serialize, Deserialize)]
pub struct WatchReply {
    pub outcome: Outcome,
    pub runs: Vec<Run>,
}

pub fn serve(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = listen(app).await {
            eprintln!("the command line socket stopped: {e}");
        }
    });
}

async fn listen(app: AppHandle) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = socket_path(&app.path().home_dir().map_err(std::io::Error::other)?);
    if !socket_path_fits(&path) {
        return Err(std::io::Error::other(PATH_TOO_LONG));
    }
    // Left over from a crash, or from another copy of the app; the newest one answers.
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle(&app, stream).await;
        });
    }
}

async fn send(out: &mut OwnedWriteHalf, reply: &Reply) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(reply)?;
    line.push(b'\n');
    out.write_all(&line).await
}

async fn handle(app: &AppHandle, stream: UnixStream) -> std::io::Result<()> {
    let (read, mut out) = stream.into_split();
    let mut read = BufReader::new(read);
    let mut line = String::new();
    (&mut read).take(MAX_REQUEST).read_line(&mut line).await?;
    let result = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Failures { all, repo }) => failures(app, all, repo.as_deref()).await.and_then(json),
        Ok(Request::Log { run, repo, branch, step }) => log(app, run, repo, branch, step).await.and_then(json),
        Ok(Request::Watch { repo, branch, commit, timeout_secs }) => {
            watch(app, &mut read, &mut out, &repo, branch, commit, Duration::from_secs(timeout_secs)).await.and_then(json)
        }
        Err(e) => Err(Error::Other(format!("Pipescope didn't understand that request ({e}). Is the app older than this command?"))),
    };
    let reply = match result {
        Ok(data) => Reply::Done { data },
        Err(e) => Reply::Error { message: e.to_string() },
    };
    send(&mut out, &reply).await
}

fn json<T: Serialize>(value: T) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(value)?)
}

fn same_repo(run: &Run, repo: &str) -> bool {
    run.repo.eq_ignore_ascii_case(repo)
}

fn listed(app: &AppHandle) -> Result<Vec<Run>> {
    commands::listed_runs(&app.state::<AppState>().store.lock().unwrap())
}

// Naming a repo shows everyone's failures there, muted or not; otherwise it's like the tray.
pub fn pick_failures(mut runs: Vec<Run>, muted: &std::collections::HashSet<String>, all: bool, repo: Option<&str>) -> Vec<Run> {
    runs.retain(|r| match repo {
        Some(repo) => same_repo(r, repo),
        None => !muted.contains(&r.repo_id()),
    });
    let since = Utc::now() - FAILURE_WINDOW;
    let only_mine = !all && repo.is_none();
    tray::failing(&runs, only_mine).into_iter().filter(|r| r.created_at >= since).cloned().collect()
}

async fn failures(app: &AppHandle, all: bool, repo: Option<&str>) -> Result<Vec<Failure>> {
    let runs = listed(app)?;
    let muted = app.state::<AppState>().store.lock().unwrap().repo_flags("muted");
    let checkouts = commands::checkout_map(app).await.unwrap_or_default();
    Ok(pick_failures(runs, &muted, all, repo)
        .into_iter()
        .map(|run| Failure { checkout: checkouts.get(&run.repo_id().to_lowercase()).cloned(), run })
        .collect())
}

// Accepts a run key, the run's web address or one of its job pages, or "owner/repo#number".
pub fn find_run<'a>(runs: &'a [Run], wanted: &str) -> Option<&'a Run> {
    let wanted = wanted.trim().trim_end_matches('/');
    if let Some(run) = runs.iter().find(|r| r.key == wanted) {
        return Some(run);
    }
    if wanted.starts_with("http") {
        return runs.iter().find(|r| !r.web_url.is_empty() && (wanted == r.web_url || wanted.starts_with(&format!("{}/", r.web_url))));
    }
    let (repo, number) = wanted.rsplit_once('#')?;
    let number: u64 = number.parse().ok()?;
    runs.iter().filter(|r| same_repo(r, repo) && r.number == number).max_by_key(|r| r.created_at)
}

// The newest failed run in the repo, on the branch if one has failed there.
pub fn latest_failure<'a>(runs: &'a [Run], repo: &str, branch: Option<&str>) -> Option<&'a Run> {
    let failed = || runs.iter().filter(|r| same_repo(r, repo) && r.status == RunStatus::Failed);
    failed()
        .filter(|r| branch.is_some() && r.branch.as_deref() == branch)
        .max_by_key(|r| r.created_at)
        .or_else(|| failed().max_by_key(|r| r.created_at))
}

fn followed(app: &AppHandle, repo: &str) -> Result<(crate::model::ProviderKind, String)> {
    app.state::<AppState>()
        .store
        .lock()
        .unwrap()
        .followed_repo(repo)?
        .ok_or_else(|| Error::Other(format!("Pipescope doesn't follow {repo}. Is it ignored, or not visible to your account?")))
}

// Only reads the runs a lookup needs: one key, one repo, or for a web address all of them.
fn resolve_run(app: &AppHandle, wanted: &str) -> Result<Option<Run>> {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    if let Some(run) = store.get_run(wanted.trim())? {
        return Ok(Some(run));
    }
    let candidates = match wanted.rsplit_once('#') {
        Some((repo, _)) if !wanted.starts_with("http") => match store.followed_repo(repo)? {
            Some((provider, exact)) => store.repo_runs(provider, &exact)?,
            None => return Ok(None),
        },
        _ => store.all_runs()?,
    };
    Ok(find_run(&candidates, wanted).cloned())
}

async fn log(app: &AppHandle, run: Option<String>, repo: Option<String>, branch: Option<String>, step: Option<String>) -> Result<LogReply> {
    let key = match (&run, &repo) {
        (Some(wanted), _) => resolve_run(app, wanted)?.ok_or_else(|| Error::Other(format!("No run matches {wanted}.")))?.key,
        (None, Some(repo)) => {
            let (provider, exact) = followed(app, repo)?;
            let runs = app.state::<AppState>().store.lock().unwrap().repo_runs(provider, &exact)?;
            latest_failure(&runs, &exact, branch.as_deref())
                .ok_or_else(|| Error::Other(format!("{exact} has no failed runs in Pipescope's history.")))?
                .key
                .clone()
        }
        (None, None) => {
            return Err(Error::Other("Name a run, or run this inside a clone of a repo Pipescope follows.".into()));
        }
    };
    let (run, client) = commands::lookup(app, &key)?;
    let log = client.log_tail(&run, step.as_deref().map_or(StepPick::Auto, StepPick::Name)).await?;
    Ok(LogReply { run, log })
}

// The newest run of each workflow that matches what's being watched.
pub fn watched_runs(runs: &[Run], repo: &str, branch: Option<&str>, commit: Option<&str>, since: chrono::DateTime<Utc>) -> Vec<Run> {
    let matching: Vec<Run> = runs
        .iter()
        .filter(|r| same_repo(r, repo))
        .filter(|r| match commit {
            // The branch too, so a commit fast-forwarded onto main isn't judged by its feature-branch runs.
            Some(sha) => {
                r.commit.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(sha)) && (branch.is_none() || r.branch.as_deref() == branch)
            }
            None => r.branch.as_deref() == branch && (r.created_at >= since || r.status.is_active()),
        })
        .cloned()
        .collect();
    latest_per_workflow(matching)
}

fn latest_per_workflow(runs: Vec<Run>) -> Vec<Run> {
    let mut latest: std::collections::HashMap<String, Run> = std::collections::HashMap::new();
    for run in runs {
        match latest.get(&run.title) {
            Some(cur) if cur.created_at >= run.created_at => {}
            _ => {
                latest.insert(run.title.clone(), run);
            }
        }
    }
    let mut out: Vec<Run> = latest.into_values().collect();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

// None while anything is still queued or running.
pub fn outcome(runs: &[Run]) -> Option<Outcome> {
    if runs.is_empty() || runs.iter().any(|r| r.status.is_active()) {
        return None;
    }
    Some(if runs.iter().any(|r| matches!(r.status, RunStatus::Failed | RunStatus::Cancelled)) {
        Outcome::Failed
    } else if runs.iter().any(|r| r.status == RunStatus::Waiting) {
        Outcome::Waiting
    } else {
        Outcome::Passed
    })
}

fn spoken(d: Duration) -> String {
    let secs = d.as_secs();
    let (n, unit) = if secs.is_multiple_of(3600) { (secs / 3600, "hour") } else if secs.is_multiple_of(60) { (secs / 60, "minute") } else { (secs, "second") };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

fn describe(runs: &[Run]) -> String {
    runs.iter().map(|r| format!("{} #{} {}", r.title, r.number, r.status.as_str())).collect::<Vec<_>>().join(", ")
}

async fn watch(
    app: &AppHandle,
    input: &mut BufReader<OwnedReadHalf>,
    out: &mut OwnedWriteHalf,
    repo: &str,
    branch: Option<String>,
    commit: Option<String>,
    timeout: Duration,
) -> Result<WatchReply> {
    if branch.is_none() && commit.is_none() {
        return Err(Error::Other("Watch needs a branch or a commit.".into()));
    }
    let state = app.state::<AppState>();
    let (provider, exact) = followed(app, repo)?;
    // A run created a little before the watch started is most likely the push being watched.
    let since = Utc::now() - TimeDelta::minutes(2);
    let started_at = Utc::now();
    let started = Instant::now();
    let mut last_poll: Option<Instant> = None;
    let mut last_said = String::new();
    // A finished set must stay finished for two looks, in case a slower workflow is about to appear.
    let mut settled: Option<Outcome> = None;
    loop {
        let runs = state.store.lock().unwrap().repo_runs(provider, &exact)?;
        let watched = watched_runs(&runs, &exact, branch.as_deref(), commit.as_deref(), since);
        if last_poll.is_none() || (watched.is_empty() && last_poll.is_some_and(|t| t.elapsed() >= WATCH_POLL)) {
            state.scheduler.send(provider, Control::PollRepo(exact.clone()));
            last_poll = Some(Instant::now());
        }
        let said = if watched.is_empty() { "Waiting for a run to start…".to_string() } else { describe(&watched) };
        if said != last_said {
            send(out, &Reply::Progress { message: said.clone() }).await?;
            last_said = said;
        }
        let fresh = started.elapsed() >= FIRST_POLL_GRACE || watched.iter().any(|r| r.updated_at >= started_at);
        match (outcome(&watched).filter(|_| fresh), settled) {
            (Some(now), Some(before)) if now == before => return Ok(WatchReply { outcome: now, runs: watched }),
            (now, _) => settled = now,
        }
        if started.elapsed() >= timeout {
            let what = if watched.is_empty() { "No run started" } else { "The runs didn't finish" };
            return Err(Error::Other(format!("{what} within {}.", spoken(timeout))));
        }
        // The client sends nothing more, so a read only returns when it hangs up, e.g. on Ctrl-C.
        let mut byte = [0u8; 1];
        tokio::select! {
            _ = tokio::time::sleep(WATCH_TICK) => {}
            _ = input.read(&mut byte) => return Err(Error::Other("The watch was stopped.".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;
    use chrono::TimeZone;

    fn run(key: &str, minute: u32, title: &str, branch: &str, commit: &str, status: RunStatus) -> Run {
        let t = Utc.with_ymd_and_hms(2026, 11, 9, 1, minute, 0).unwrap();
        Run {
            key: key.into(),
            provider: ProviderKind::Github,
            repo: "Acme/Web".into(),
            id: key.into(),
            number: minute as u64,
            title: title.into(),
            subtitle: None,
            branch: Some(branch.into()),
            commit: Some(commit.into()),
            status,
            actor: None,
            mine: true,
            created_at: t,
            started_at: None,
            updated_at: t,
            web_url: format!("https://github.com/Acme/Web/actions/runs/{key}"),
            author: None,
            build_secs: None,
            failed_before: false,
            flaky: false,
            flaky_workflow: false,
            watched: false,
        }
    }

    #[test]
    fn requests_round_trip_as_tagged_json() {
        let req = Request::Watch { repo: "acme/web".into(), branch: Some("main".into()), commit: None, timeout_secs: 60 };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.starts_with(r#"{"cmd":"watch""#));
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
    }

    #[test]
    fn finds_runs_by_key_address_job_page_or_number() {
        let runs = vec![run("1", 10, "CI", "main", "a", RunStatus::Success), run("2", 20, "CI", "main", "b", RunStatus::Failed)];
        assert_eq!(find_run(&runs, "2").unwrap().key, "2");
        assert_eq!(find_run(&runs, "https://github.com/Acme/Web/actions/runs/1/").unwrap().key, "1");
        assert_eq!(find_run(&runs, "https://github.com/Acme/Web/actions/runs/2/job/99").unwrap().key, "2");
        assert_eq!(find_run(&runs, "acme/web#10").unwrap().key, "1");
        assert!(find_run(&runs, "https://github.com/Acme/Web/actions/runs/12").is_none());
        assert!(find_run(&runs, "acme/web#nope").is_none());
    }

    #[test]
    fn latest_failure_prefers_the_branch() {
        let runs = vec![
            run("1", 10, "CI", "feature", "a", RunStatus::Failed),
            run("2", 20, "CI", "main", "b", RunStatus::Failed),
            run("3", 30, "CI", "feature", "c", RunStatus::Success),
        ];
        assert_eq!(latest_failure(&runs, "acme/web", Some("feature")).unwrap().key, "1");
        assert_eq!(latest_failure(&runs, "acme/web", Some("other")).unwrap().key, "2");
        assert!(latest_failure(&runs, "acme/api", None).is_none());
    }

    #[test]
    fn watches_the_latest_run_of_each_workflow_for_a_commit() {
        let since = Utc.with_ymd_and_hms(2026, 11, 9, 1, 15, 0).unwrap();
        let runs = vec![
            run("1", 10, "CI", "main", "abc", RunStatus::Failed),
            run("2", 20, "CI", "main", "abc", RunStatus::Running),
            run("3", 21, "Lint", "main", "abc", RunStatus::Success),
            run("4", 22, "CI", "main", "other", RunStatus::Success),
        ];
        let by_commit = watched_runs(&runs, "acme/web", None, Some("ABC"), since);
        assert_eq!(by_commit.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["2", "3"]);
        assert_eq!(outcome(&by_commit), None);

        // By branch, only runs that started after the watch began, or are still going.
        let by_branch = watched_runs(&runs[..1], "acme/web", Some("main"), None, since);
        assert!(by_branch.is_empty());
    }

    #[test]
    fn a_fast_forwarded_commit_waits_for_its_own_branch() {
        let since = Utc.with_ymd_and_hms(2026, 11, 9, 1, 0, 0).unwrap();
        let runs = vec![run("1", 10, "CI", "feature", "abc", RunStatus::Success)];
        assert_eq!(watched_runs(&runs, "acme/web", None, Some("abc"), since).len(), 1);
        assert!(watched_runs(&runs, "acme/web", Some("main"), Some("abc"), since).is_empty());
    }

    #[test]
    fn picks_failures_like_the_tray_unless_a_repo_is_named() {
        let recent = |key: &str, repo: &str, mine: bool| {
            let mut r = run(key, 0, "CI", key, "a", RunStatus::Failed);
            r.created_at = Utc::now() - TimeDelta::hours(1);
            (r.repo, r.mine) = (repo.into(), mine);
            r
        };
        let mut old = recent("old", "acme/web", true);
        old.created_at = Utc::now() - TimeDelta::hours(49);
        let runs = vec![recent("mine", "acme/web", true), recent("theirs", "acme/web", false), recent("muted", "acme/docs", true), old];
        let muted = std::collections::HashSet::from(["github:acme/docs".to_string()]);
        let keys = |all, repo| {
            let mut k: Vec<String> = pick_failures(runs.clone(), &muted, all, repo).into_iter().map(|r| r.key).collect();
            k.sort();
            k
        };
        assert_eq!(keys(false, None), ["mine"]);
        assert_eq!(keys(true, None), ["mine", "theirs"]);
        assert_eq!(keys(false, Some("ACME/WEB")), ["mine", "theirs"]);
        assert_eq!(keys(false, Some("acme/docs")), ["muted"]);
    }

    #[test]
    fn long_socket_paths_are_caught() {
        assert!(socket_path_fits(&socket_path(Path::new("/Users/grant"))));
        assert!(!socket_path_fits(&socket_path(&Path::new("/Users").join("x".repeat(60)))));
    }

    #[test]
    fn speaks_timeouts() {
        assert_eq!(spoken(Duration::from_secs(60)), "1 minute");
        assert_eq!(spoken(Duration::from_secs(7200)), "2 hours");
        assert_eq!(spoken(Duration::from_secs(90)), "90 seconds");
    }

    #[test]
    fn outcome_needs_every_run_finished() {
        let done = |s| run("1", 1, "CI", "main", "a", s);
        assert_eq!(outcome(&[]), None);
        assert_eq!(outcome(&[done(RunStatus::Success)]), Some(Outcome::Passed));
        assert_eq!(outcome(&[done(RunStatus::Success), done(RunStatus::Cancelled)]), Some(Outcome::Failed));
        assert_eq!(outcome(&[done(RunStatus::Waiting)]), Some(Outcome::Waiting));
        assert_eq!(outcome(&[done(RunStatus::Queued)]), None);
    }
}
