use chrono::{TimeDelta, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{Error, Result};
use crate::model::{LogTail, ProviderKind, ProviderStatus, Run};
use crate::providers::{Client, Creds};
use crate::scheduler::Control;
use crate::{AppState, auth, connect, tray};

const LIST_WINDOW: TimeDelta = TimeDelta::days(7);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    notify_all: bool,
    bitbucket_email: Option<String>,
    pinned: Vec<String>,
    muted: Vec<String>,
    onboarded: bool,
}

#[tauri::command]
pub fn get_runs(state: State<'_, AppState>) -> Result<Vec<Run>> {
    state.store.lock().unwrap().recent_runs(Utc::now() - LIST_WINDOW)
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Vec<ProviderStatus> {
    let status = state.status.lock().unwrap();
    ProviderKind::ALL.iter().filter_map(|k| status.get(k).cloned()).collect()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
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
    })
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
    app.state::<AppState>().scheduler.send(run.provider, Control::PollRepo(run.repo));
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
    client.log_tail(&run, step.as_deref()).await
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
