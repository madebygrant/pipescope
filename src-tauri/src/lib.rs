mod auth;
mod commands;
mod error;
mod flaky;
mod http;
mod model;
mod notify;
mod providers;
mod scheduler;
mod store;
mod tray;

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use model::{ConnState, ProviderKind, ProviderStatus};
use providers::{Client, Creds};
use scheduler::Scheduler;
use store::Store;

// Before 0.3.2 the bundle identifier, and so the data folder, was this.
const LEGACY_ID: &str = "au.com.hatchd.pipescope";

// Copies the old database across so history and settings survive the identifier change.
// A database that has never polled is replaced too: a dev build may have made it first.
// The old folder is left alone in case of a downgrade.
fn adopt_legacy_db(data_dir: &std::path::Path, dir: &std::path::Path) {
    let (old, new) = (data_dir.join(LEGACY_ID).join("pipescope.db"), dir.join("pipescope.db"));
    if old.exists()
        && !has_polled(&new)
        && let Err(e) = std::fs::copy(&old, &new)
    {
        eprintln!("copying the database from {} failed: {e}", old.display());
    }
}

fn has_polled(db: &std::path::Path) -> bool {
    db.exists()
        && rusqlite::Connection::open(db)
            .and_then(|c| c.query_row("SELECT EXISTS (SELECT 1 FROM polled_repos)", [], |r| r.get(0)))
            .unwrap_or(false)
}

fn toggle_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyP)
}

pub struct AppState {
    pub store: Mutex<Store>,
    pub scheduler: Scheduler,
    pub clients: Mutex<HashMap<ProviderKind, Client>>,
    pub status: Mutex<HashMap<ProviderKind, ProviderStatus>>,
    pub notifier: notify::Notifier,
    // Bumped by each connect(); an older attempt that sees a newer number gives up, so
    // a retry loop from before a token change can't reconnect the old account.
    pub connect_gen: Mutex<HashMap<ProviderKind, u64>>,
}

pub fn update_status(app: &AppHandle, kind: ProviderKind, f: impl FnOnce(&mut ProviderStatus)) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut map = state.status.lock().unwrap();
        let status = map.entry(kind).or_insert_with(|| ProviderStatus::new(kind, ConnState::Connecting));
        f(status);
        status.clone()
    };
    let _ = app.emit("status-changed", snapshot);
}

// For fields that change on every poll; the UI reads them when it refreshes status.
pub fn set_status_quiet(app: &AppHandle, kind: ProviderKind, f: impl FnOnce(&mut ProviderStatus)) {
    if let Some(status) = app.state::<AppState>().status.lock().unwrap().get_mut(&kind) {
        f(status);
    }
}

fn load_creds(app: &AppHandle, kind: ProviderKind) -> error::Result<Option<(Creds, &'static str)>> {
    Ok(match kind {
        ProviderKind::Github => match auth::get_secret("github")? {
            Some(token) => Some((Creds::Github(token), "Keychain")),
            None => auth::gh_cli_token().map(|token| (Creds::Github(token), "gh CLI")),
        },
        ProviderKind::Bitbucket => {
            let email = app.state::<AppState>().store.lock().unwrap().setting("bitbucket_email")?;
            match (email, auth::get_secret("bitbucket")?) {
                (Some(email), Some(token)) => Some((Creds::Bitbucket { email, token }, "Keychain")),
                _ => None,
            }
        }
    })
}

pub async fn connect(app: &AppHandle, kind: ProviderKind) {
    let generation = {
        let state = app.state::<AppState>();
        let mut gens = state.connect_gen.lock().unwrap();
        let g = gens.entry(kind).or_default();
        *g += 1;
        *g
    };
    let superseded = || app.state::<AppState>().connect_gen.lock().unwrap().get(&kind) != Some(&generation);
    let fresh = |state| move |s: &mut ProviderStatus| *s = ProviderStatus::new(kind, state);

    // Keychain reads and `gh auth token` block, so keep them off the async workers.
    let handle = app.clone();
    let loaded = tauri::async_runtime::spawn_blocking(move || load_creds(&handle, kind))
        .await
        .map_err(|e| error::Error::Other(e.to_string()))
        .and_then(|r| r);
    if superseded() {
        return;
    }
    let (creds, source) = match loaded {
        Ok(Some(found)) => found,
        Ok(None) => return update_status(app, kind, fresh(ConnState::NotConfigured)),
        Err(e) => {
            return update_status(app, kind, |s| {
                fresh(ConnState::Error)(s);
                s.message = Some(e.to_string());
            });
        }
    };
    update_status(app, kind, |s| {
        fresh(ConnState::Connecting)(s);
        s.source = Some(source.into());
    });
    loop {
        let result = Client::connect(creds.clone()).await;
        if superseded() {
            return;
        }
        match result {
            Ok(client) => {
                let account = client.account().to_string();
                let warnings = client.warnings();
                update_status(app, kind, |s| {
                    s.account = Some(account);
                    s.warnings = warnings;
                });
                let state = app.state::<AppState>();
                state.clients.lock().unwrap().insert(kind, client.clone());
                state.scheduler.start(app.clone(), client);
                return;
            }
            // Launching while offline (say, at login before Wi-Fi joins) shouldn't need a restart.
            Err(e) if e.is_network() => {
                update_status(app, kind, |s| {
                    s.state = ConnState::Offline;
                    s.message = Some("Offline. Retrying every minute.".into());
                });
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if superseded() {
                    return;
                }
            }
            Err(e) => {
                return update_status(app, kind, |s| {
                    s.state = ConnState::Error;
                    s.message = Some(e.to_string());
                });
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        tray::toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            adopt_legacy_db(&app.path().data_dir()?, &dir);
            app.manage(AppState {
                store: Mutex::new(Store::open(&dir.join("pipescope.db"))?),
                scheduler: Scheduler::default(),
                clients: Mutex::default(),
                status: Mutex::default(),
                notifier: notify::Notifier::default(),
                connect_gen: Mutex::default(),
            });
            notify::init(app.handle())?;
            tray::init(app.handle())?;
            // Another app may already own the shortcut; the tray icon still works without it.
            if let Err(e) = app.global_shortcut().register(toggle_shortcut()) {
                eprintln!("couldn't register the toggle shortcut: {e}");
            }
            for kind in ProviderKind::ALL {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move { connect(&handle, kind).await });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runs,
            commands::get_run,
            commands::get_status,
            commands::get_settings,
            commands::set_notify_all,
            commands::set_notify_rules,
            commands::set_ignored_repos,
            commands::set_auto_rerun,
            commands::watch_run,
            commands::set_repo_client,
            commands::get_build_time,
            commands::get_pending_deployments,
            commands::review_deployments,
            commands::manual_step_url,
            commands::get_baseline_log,
            commands::set_launch_at_login,
            commands::set_github_token,
            commands::set_bitbucket_auth,
            commands::refresh,
            commands::rerun,
            commands::cancel,
            commands::get_log,
            commands::open_log_window,
            commands::set_repo_flag,
            commands::set_onboarded,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{LEGACY_ID, adopt_legacy_db, has_polled};
    use crate::model::ProviderKind;
    use crate::store::Store;

    #[test]
    fn adopts_the_old_database_unless_the_new_one_has_polled() {
        let root = std::env::temp_dir().join(format!("pipescope-migrate-{}", std::process::id()));
        let (old_dir, new_dir) = (root.join(LEGACY_ID), root.join("new"));
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();
        let (old, new) = (old_dir.join("pipescope.db"), new_dir.join("pipescope.db"));
        Store::open(&old).unwrap().apply_poll(ProviderKind::Github, "o/r", &[]).unwrap();

        // A dev build got there first and made an empty database.
        drop(Store::open(&new).unwrap());
        assert!(!has_polled(&new));
        adopt_legacy_db(&root, &new_dir);
        assert!(has_polled(&new));

        // Once the new one has history of its own, it's left alone.
        std::fs::remove_file(&old).unwrap();
        Store::open(&old).unwrap();
        adopt_legacy_db(&root, &new_dir);
        assert!(has_polled(&new));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
