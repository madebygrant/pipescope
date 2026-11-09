use chrono::{TimeDelta, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

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
    // None in dev builds, where the login item would point at the debug binary.
    launch_at_login: Option<bool>,
}

#[tauri::command]
pub fn get_runs(state: State<'_, AppState>) -> Result<Vec<Run>> {
    state.store.lock().unwrap().recent_runs(Utc::now() - LIST_WINDOW)
}

#[tauri::command]
pub fn get_run(state: State<'_, AppState>, key: String) -> Result<Option<Run>> {
    state.store.lock().unwrap().get_run(&key)
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
    use super::log_window_label;

    #[test]
    fn window_labels_stay_distinct() {
        assert_eq!(log_window_label("github:acme/web:12"), "log-github:acme/web:12");
        assert_ne!(log_window_label("github:acme/my.app:1"), log_window_label("github:acme/my_app:1"));
        assert_eq!(log_window_label("bitbucket:ws/app:{u}"), "log-bitbucket:ws/app:_7b_u_7d_");
    }
}
