use chrono::{TimeDelta, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::error::{Error, Result};
use std::collections::HashMap;

use crate::model::{ChangedFile, LogTail, PendingDeployment, ProviderKind, ProviderStatus, Run, RunStatus, StepPick};
use crate::store::{BuildTime, month};
use crate::providers::{Client, Creds};
use crate::scheduler::Control;
use crate::{AppState, auth, checkouts, connect, flaky, notify, tray};

const LIST_WINDOW: TimeDelta = TimeDelta::days(7);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    notify_all: bool,
    bitbucket_email: Option<String>,
    pinned: Vec<String>,
    muted: Vec<String>,
    onboarded: bool,
    notify_branches: String,
    ignored_repos: String,
    quiet_hours: String,
    auto_rerun_flaky: bool,
    clients: HashMap<String, String>,
    code_folders: String,
    default_code_folders: &'static str,
    // None in dev builds, where the login item would point at the debug binary.
    launch_at_login: Option<bool>,
}

#[tauri::command]
pub fn get_runs(state: State<'_, AppState>) -> Result<Vec<Run>> {
    let store = state.store.lock().unwrap();
    let mut runs = store.recent_runs(Utc::now() - LIST_WINDOW)?;
    // Marked against every stored run, not just the listed ones, so get_run agrees.
    let mut all = store.all_runs()?;
    flaky::annotate(&mut all, &store.repo_flags("watched"));
    let marked: HashMap<String, Run> = all.into_iter().map(|r| (r.key.clone(), r)).collect();
    for run in &mut runs {
        if let Some(m) = marked.get(&run.key) {
            (run.flaky, run.flaky_workflow, run.watched) = (m.flaky, m.flaky_workflow, m.watched);
        }
    }
    Ok(runs)
}

#[tauri::command]
pub fn get_run(state: State<'_, AppState>, key: String) -> Result<Option<Run>> {
    let store = state.store.lock().unwrap();
    let Some(run) = store.get_run(&key)? else { return Ok(None) };
    // Flaky marks depend on the rest of the repo's history.
    let mut history = store.repo_runs(run.provider, &run.repo)?;
    flaky::annotate(&mut history, &store.repo_flags("watched"));
    Ok(history.into_iter().find(|r| r.key == key))
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Vec<ProviderStatus> {
    let status = state.status.lock().unwrap();
    ProviderKind::ALL.iter().filter_map(|k| status.get(k).cloned()).collect()
}

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Result<Settings> {
    let store = state.store.lock().unwrap();
    let sorted = |flag| {
        let mut ids: Vec<String> = store.repo_flags(flag).into_iter().collect();
        ids.sort();
        ids
    };
    Ok(Settings {
        notify_all: store.notify_all(),
        bitbucket_email: store.setting("bitbucket_email")?,
        pinned: sorted("pinned"),
        muted: sorted("muted"),
        onboarded: store.setting("onboarded")?.is_some(),
        notify_branches: store.setting("notify_branches")?.unwrap_or_default(),
        ignored_repos: store.setting("ignored_repos")?.unwrap_or_default(),
        quiet_hours: store.setting("quiet_hours")?.unwrap_or_default(),
        auto_rerun_flaky: store.setting("auto_rerun_flaky")?.as_deref() == Some("1"),
        clients: store.clients(),
        code_folders: store.setting("code_folders")?.unwrap_or_default(),
        default_code_folders: checkouts::DEFAULT_FOLDERS,
        launch_at_login: launch_at_login(&app)?,
    })
}

fn launch_at_login(app: &AppHandle) -> Result<Option<bool>> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    app.autolaunch().is_enabled().map(Some).map_err(|e| Error::Other(e.to_string()))
}

#[tauri::command]
pub fn set_notify_rules(state: State<'_, AppState>, branches: String, quiet_hours: String) -> Result<()> {
    let quiet_hours = quiet_hours.trim();
    if !quiet_hours.is_empty() && notify::parse_quiet_hours(quiet_hours).is_none() {
        return Err(Error::Other("Quiet hours should look like 22:00-07:00.".into()));
    }
    let store = state.store.lock().unwrap();
    let branches = branches.trim();
    store.set_setting("notify_branches", Some(branches).filter(|b| !b.is_empty()))?;
    store.set_setting("quiet_hours", Some(quiet_hours).filter(|q| !q.is_empty()))
}

#[tauri::command]
pub fn set_ignored_repos(app: AppHandle, state: State<'_, AppState>, patterns: String) -> Result<()> {
    let patterns = patterns.trim();
    {
        let store = state.store.lock().unwrap();
        store.set_setting("ignored_repos", Some(patterns).filter(|p| !p.is_empty()))?;
        store.forget_repos(|repo| crate::scheduler::repo_ignored(patterns, repo))?;
    }
    // Relisting applies the patterns both ways: newly ignored repos stop, un-ignored ones come back.
    for kind in ProviderKind::ALL {
        state.scheduler.send(kind, Control::RefreshAll);
    }
    let _ = app.emit("runs-updated", ());
    tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn set_auto_rerun(state: State<'_, AppState>, on: bool) -> Result<()> {
    state.store.lock().unwrap().set_setting("auto_rerun_flaky", on.then_some("1"))
}

#[tauri::command]
pub fn watch_run(app: AppHandle, key: String, on: bool) -> Result<()> {
    app.state::<AppState>().store.lock().unwrap().set_repo_flag("watched", &key, on)?;
    let _ = app.emit("runs-updated", ());
    Ok(())
}

#[tauri::command]
pub fn set_repo_client(app: AppHandle, repo_id: String, client: Option<String>) -> Result<()> {
    app.state::<AppState>().store.lock().unwrap().set_client(&repo_id, client.as_deref())
}

// Defaults to the current month, "YYYY-MM" in UTC.
#[tauri::command]
pub fn get_build_time(state: State<'_, AppState>, month: Option<String>) -> Result<Vec<BuildTime>> {
    let month = month.unwrap_or_else(|| self::month(Utc::now()));
    state.store.lock().unwrap().build_time(&month)
}

#[tauri::command]
pub async fn get_pending_deployments(app: AppHandle, key: String) -> Result<Vec<PendingDeployment>> {
    let (run, client) = lookup(&app, &key)?;
    client.pending_deployments(&run).await
}

// Reviews every environment you're allowed to; GitHub rejects the rest anyway.
#[tauri::command]
pub async fn review_deployments(app: AppHandle, key: String, approve: bool) -> Result<()> {
    let (run, client) = lookup(&app, &key)?;
    let ids: Vec<u64> =
        client.pending_deployments(&run).await?.into_iter().filter(|p| p.can_approve).map(|p| p.environment_id).collect();
    if ids.is_empty() {
        return Err(Error::Other("You're not a required reviewer for this deployment.".into()));
    }
    client.review_deployments(&run, &ids, approve).await?;
    app.state::<AppState>().scheduler.send(run.provider, Control::PollRepo(run.repo));
    Ok(())
}

#[tauri::command]
pub async fn manual_step_url(app: AppHandle, key: String) -> Result<String> {
    let (run, client) = lookup(&app, &key)?;
    client.manual_step_url(&run).await
}

#[derive(Serialize)]
pub struct Baseline {
    run: Run,
    log: LogTail,
}

// The newest earlier pass of the same workflow, preferring the same branch.
fn baseline_run(history: &[Run], run: &Run) -> Option<Run> {
    let passes = || {
        history
            .iter()
            .filter(|r| r.key != run.key && r.title == run.title && r.status == RunStatus::Success && r.created_at < run.created_at)
    };
    passes().filter(|r| r.branch == run.branch).max_by_key(|r| r.created_at).or_else(|| passes().max_by_key(|r| r.created_at)).cloned()
}

#[tauri::command]
pub async fn get_baseline_log(app: AppHandle, key: String, step: String) -> Result<Option<Baseline>> {
    let (run, client) = lookup(&app, &key)?;
    let history = app.state::<AppState>().store.lock().unwrap().repo_runs(run.provider, &run.repo)?;
    let Some(base) = baseline_run(&history, &run) else { return Ok(None) };
    let log = client.log_tail(&base, StepPick::Name(&step)).await?;
    Ok(Some(Baseline { run: base, log }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    // The last pass the files are compared with; without one, they're the run's own commit.
    base: Option<Run>,
    files: Vec<ChangedFile>,
}

// What to diff against: the last pass with a known commit, and whether it ran this same commit.
fn compare_base(history: &[Run], run: &Run) -> (Option<Run>, bool) {
    let base = baseline_run(history, run).filter(|b| b.commit.is_some());
    let same = base.as_ref().is_some_and(|b| b.commit == run.commit);
    (base, same)
}

#[tauri::command]
pub async fn get_changed_files(app: AppHandle, key: String) -> Result<Changes> {
    let (run, client) = lookup(&app, &key)?;
    let head = run.commit.clone().ok_or_else(|| Error::Other("This run has no commit.".into()))?;
    let history = app.state::<AppState>().store.lock().unwrap().repo_runs(run.provider, &run.repo)?;
    let (base, same) = compare_base(&history, &run);
    let files = if same {
        Vec::new()
    } else {
        client.changed_files(&run.repo, base.as_ref().and_then(|b| b.commit.as_deref()), &head).await?
    };
    Ok(Changes { base, files })
}

// Long enough that the run menu doesn't walk the disk on every right-click,
// short enough that a fresh clone shows up soon.
const CHECKOUTS_FRESH: std::time::Duration = std::time::Duration::from_secs(30);

// Lowercased repo id → local checkout.
#[tauri::command]
pub async fn get_checkouts(app: AppHandle) -> Result<HashMap<String, String>> {
    let state = app.state::<AppState>();
    if let Some((at, found)) = state.checkouts.lock().unwrap().as_ref()
        && at.elapsed() < CHECKOUTS_FRESH
    {
        return Ok(found.clone());
    }
    let setting = state.store.lock().unwrap().setting("code_folders")?.unwrap_or_default();
    let roots = checkouts::folders(&setting, &app.path().home_dir()?);
    let found = tauri::async_runtime::spawn_blocking(move || checkouts::scan(&roots))
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    *state.checkouts.lock().unwrap() = Some((std::time::Instant::now(), found.clone()));
    Ok(found)
}

#[tauri::command]
pub fn set_code_folders(state: State<'_, AppState>, folders: String) -> Result<()> {
    let folders = folders.trim();
    state.store.lock().unwrap().set_setting("code_folders", Some(folders).filter(|f| !f.is_empty()))?;
    *state.checkouts.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, on: bool) -> Result<()> {
    if cfg!(debug_assertions) {
        return Err(Error::Other("Launch at login only works in the built app.".into()));
    }
    let autolaunch = app.autolaunch();
    let result = if on { autolaunch.enable() } else { autolaunch.disable() };
    result.map_err(|e| Error::Other(format!("Couldn't change launch at login: {e}")))
}

#[tauri::command]
pub fn set_notify_all(app: AppHandle, value: bool) -> Result<()> {
    app.state::<AppState>().store.lock().unwrap().set_setting("notify_all", value.then_some("1"))?;
    tray::refresh(&app);
    Ok(())
}

// An empty token removes the saved one, which falls back to the gh CLI token.
#[tauri::command]
pub async fn set_github_token(app: AppHandle, token: String) -> Result<()> {
    let token = token.trim().to_string();
    if token.is_empty() {
        auth::delete_secret("github")?;
    } else {
        Client::connect(Creds::Github(token.clone())).await?;
        auth::set_secret("github", &token)?;
    }
    reset(&app, ProviderKind::Github)?;
    connect(&app, ProviderKind::Github).await;
    Ok(())
}

#[tauri::command]
pub async fn set_bitbucket_auth(app: AppHandle, email: String, token: String) -> Result<()> {
    let (email, token) = (email.trim().to_string(), token.trim().to_string());
    let state = app.state::<AppState>();
    if email.is_empty() && token.is_empty() {
        auth::delete_secret("bitbucket")?;
        state.store.lock().unwrap().set_setting("bitbucket_email", None)?;
    } else if email.is_empty() || token.is_empty() {
        return Err(Error::Other("Enter both your Atlassian email and API token.".into()));
    } else {
        Client::connect(Creds::Bitbucket { email: email.clone(), token: token.clone() }).await?;
        auth::set_secret("bitbucket", &token)?;
        state.store.lock().unwrap().set_setting("bitbucket_email", Some(&email))?;
    }
    reset(&app, ProviderKind::Bitbucket)?;
    connect(&app, ProviderKind::Bitbucket).await;
    Ok(())
}

// The account may have changed, so drop runs whose "mine" flag was computed for the old one.
fn reset(app: &AppHandle, kind: ProviderKind) -> Result<()> {
    let state = app.state::<AppState>();
    state.scheduler.stop(kind);
    state.clients.lock().unwrap().remove(&kind);
    state.store.lock().unwrap().clear_provider(kind)?;
    let _ = app.emit("runs-updated", ());
    tray::refresh(app);
    Ok(())
}

#[tauri::command]
pub fn refresh(state: State<'_, AppState>) {
    for kind in ProviderKind::ALL {
        state.scheduler.send(kind, Control::RefreshAll);
    }
}

fn lookup(app: &AppHandle, key: &str) -> Result<(Run, Client)> {
    let state = app.state::<AppState>();
    let run = state.store.lock().unwrap().get_run(key)?.ok_or_else(|| Error::Other("Run not found.".into()))?;
    let client = state
        .clients
        .lock()
        .unwrap()
        .get(&run.provider)
        .cloned()
        .ok_or_else(|| Error::Other("Not connected.".into()))?;
    Ok((run, client))
}

#[tauri::command]
pub async fn rerun(app: AppHandle, key: String, failed_only: bool) -> Result<()> {
    let (run, client) = lookup(&app, &key)?;
    client.rerun(&run, failed_only).await?;
    crate::scheduler::after_rerun(&app, &run);
    Ok(())
}

#[tauri::command]
pub async fn cancel(app: AppHandle, key: String) -> Result<()> {
    let (run, client) = lookup(&app, &key)?;
    client.cancel(&run).await?;
    app.state::<AppState>().scheduler.send(run.provider, Control::PollRepo(run.repo));
    Ok(())
}

#[tauri::command]
pub async fn get_log(app: AppHandle, key: String, step: Option<String>) -> Result<LogTail> {
    let (run, client) = lookup(&app, &key)?;
    client.log_tail(&run, step.as_deref().map_or(StepPick::Auto, StepPick::Id)).await
}

#[tauri::command]
pub fn open_log_window(app: AppHandle, key: String, step: Option<String>) -> Result<()> {
    let run = app.state::<AppState>().store.lock().unwrap().get_run(&key)?;
    let run = run.ok_or_else(|| Error::Other("That run is no longer in the list.".into()))?;
    let label = log_window_label(&key);
    if let Some(window) = app.get_webview_window(&label) {
        window.show()?;
        return Ok(window.set_focus()?);
    }
    // The page reads which run to show from this global before React starts.
    let init = format!(
        "window.__PIPESCOPE_LOG__ = {};",
        serde_json::json!({ "key": key, "step": step })
    );
    let window = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(format!("{} #{} · {}", run.title, run.number, run.repo))
        .inner_size(900.0, 640.0)
        .min_inner_size(480.0, 320.0)
        .center()
        .initialization_script(&init)
        .build()?;
    window.set_focus()?;
    Ok(())
}

// Labels allow only alphanumerics and -/:_, so escape everything else (and _ itself)
// reversibly; a lossy mapping would let two repos share one window.
fn log_window_label(key: &str) -> String {
    let mut label = String::from("log-");
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':') {
            label.push(c);
        } else {
            label.push_str(&format!("_{:x}_", c as u32));
        }
    }
    label
}

#[tauri::command]
pub fn set_repo_flag(app: AppHandle, repo_id: String, flag: String, on: bool) -> Result<()> {
    if flag != "pinned" && flag != "muted" {
        return Err(Error::Other(format!("unknown repo flag: {flag}")));
    }
    app.state::<AppState>().store.lock().unwrap().set_repo_flag(&flag, &repo_id, on)?;
    if flag == "muted" {
        tray::refresh(&app);
    }
    Ok(())
}

#[tauri::command]
pub fn set_onboarded(state: State<'_, AppState>) -> Result<()> {
    state.store.lock().unwrap().set_setting("onboarded", Some("1"))
}

#[cfg(test)]
mod tests {
    use super::{baseline_run, compare_base, log_window_label};
    use crate::model::{ProviderKind, Run, RunStatus};
    use chrono::{TimeZone, Utc};

    fn run(id: &str, minute: u32, branch: &str, status: RunStatus) -> Run {
        let t = Utc.with_ymd_and_hms(2026, 11, 9, 1, minute, 0).unwrap();
        Run {
            key: id.into(),
            provider: ProviderKind::Github,
            repo: "o/r".into(),
            id: id.into(),
            number: minute as u64,
            title: "CI".into(),
            subtitle: None,
            branch: Some(branch.into()),
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
    fn baseline_prefers_the_same_branch_then_any() {
        let failing = run("f", 30, "feature", RunStatus::Failed);
        let history = vec![
            failing.clone(),
            run("later", 40, "feature", RunStatus::Success),
            run("main", 20, "main", RunStatus::Success),
            run("feat", 10, "feature", RunStatus::Success),
        ];
        assert_eq!(baseline_run(&history, &failing).unwrap().id, "feat");
        let other = run("f2", 30, "new-branch", RunStatus::Failed);
        assert_eq!(baseline_run(&history, &other).unwrap().id, "main");
        assert!(baseline_run(&history[2..], &run("f3", 5, "main", RunStatus::Failed)).is_none());
    }

    #[test]
    fn compares_with_the_last_pass_unless_it_ran_the_same_commit() {
        let with = |mut r: Run, sha: Option<&str>| {
            r.commit = sha.map(str::to_string);
            r
        };
        let failing = with(run("f", 30, "feature", RunStatus::Failed), Some("head"));
        let pass = with(run("p", 10, "feature", RunStatus::Success), Some("old"));
        let (base, same) = compare_base(&[failing.clone(), pass], &failing);
        assert_eq!((base.unwrap().id.as_str(), same), ("p", false));

        let retry_pass = with(run("p", 10, "feature", RunStatus::Success), Some("head"));
        assert!(compare_base(&[failing.clone(), retry_pass], &failing).1);

        // A pass with no recorded commit can't be diffed against, so the run's own commit is used.
        let no_sha = with(run("p", 10, "feature", RunStatus::Success), None);
        assert_eq!(compare_base(&[failing.clone(), no_sha], &failing), (None, false));
    }

    #[test]
    fn window_labels_stay_distinct() {
        assert_eq!(log_window_label("github:acme/web:12"), "log-github:acme/web:12");
        assert_ne!(log_window_label("github:acme/my.app:1"), log_window_label("github:acme/my_app:1"));
        assert_eq!(log_window_label("bitbucket:ws/app:{u}"), "log-bitbucket:ws/app:_7b_u_7d_");
    }
}
