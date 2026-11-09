use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{TimeDelta, Utc};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Theme, WebviewWindow, Wry};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_positioner::{Position, WindowExt};

use crate::AppState;
use crate::model::{ProviderKind, Run, RunStatus};
use crate::scheduler::Control;

const TRAY_ID: &str = "main";
// Failures older than this stop turning the icon red, so a branch deleted after
// a failed run doesn't leave it red for days.
const HEALTH_WINDOW: TimeDelta = TimeDelta::hours(48);

static HIDDEN_AT_MS: AtomicU64 = AtomicU64::new(0);
static DARK_MODE: AtomicBool = AtomicBool::new(false);
// Keys of the failing runs the menu currently lists, so it's only rebuilt when they
// change. Only touched on the main thread.
static MENU_RUNS: Mutex<Option<Vec<String>>> = Mutex::new(None);
const MENU_MAX_RUNS: usize = 8;

// From assets/tray-mark.svg at 36px high; the white PNG is the black one RGB-inverted.
const MARK_BLACK: &[u8] = include_bytes!("../icons/tray/mark-black.png");
const MARK_WHITE: &[u8] = include_bytes!("../icons/tray/mark-white.png");
// Lens centre in PNG pixels; update if the SVG's viewBox or lens moves.
const LENS_CENTRE: (f32, f32) = (25.6, 8.2);
const LENS_DOT_RADIUS: f32 = 3.4;
// The lens dot is too small to read on its own.
const SIDE_GAP: u32 = 6;
const SIDE_DOT: u32 = 14;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Health {
    Unknown,
    Passing,
    Running,
    Failing,
}

// The latest run of each workflow on each branch that failed, newest first.
pub fn failing(runs: &[Run], only_mine: bool) -> Vec<&Run> {
    let mut failing: Vec<&Run> =
        latest_per_workflow(runs, only_mine).into_iter().filter(|r| r.status == RunStatus::Failed).collect();
    failing.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    failing
}

fn latest_per_workflow(runs: &[Run], only_mine: bool) -> Vec<&Run> {
    let mut latest: HashMap<(ProviderKind, &str, &str, Option<&str>), &Run> = HashMap::new();
    for run in runs.iter().filter(|r| !only_mine || r.mine) {
        latest
            .entry((run.provider, &run.repo, &run.title, run.branch.as_deref()))
            .and_modify(|cur| {
                if run.created_at > cur.created_at {
                    *cur = run;
                }
            })
            .or_insert(run);
    }
    latest.into_values().collect()
}

pub fn health(runs: &[Run], only_mine: bool) -> Health {
    let considered: Vec<&Run> = runs.iter().filter(|r| !only_mine || r.mine).collect();
    if considered.is_empty() {
        return Health::Unknown;
    }
    if !failing(runs, only_mine).is_empty() {
        Health::Failing
    } else if considered.iter().any(|r| r.status.is_active()) {
        Health::Running
    } else {
        Health::Passing
    }
}

fn icon_for(health: Health, dark: bool) -> (Image<'static>, &'static str) {
    let (rgb, tooltip) = match health {
        Health::Unknown => ([142, 142, 147], "Pipescope: no runs yet"),
        Health::Passing => ([52, 199, 89], "Pipescope: all passing"),
        Health::Running => ([255, 159, 10], "Pipescope: runs in progress"),
        Health::Failing => ([255, 59, 48], "Pipescope: something is failing"),
    };
    // A template image would let macOS pick black or white itself, but it can't
    // carry a coloured dot, so the mark follows the system theme instead.
    let mark = Image::from_bytes(if dark { MARK_WHITE } else { MARK_BLACK }).expect("bundled tray PNG decodes");
    (with_status(&mark, rgb), tooltip)
}

fn with_status(mark: &Image, rgb: [u8; 3]) -> Image<'static> {
    let (mark_w, h) = (mark.width(), mark.height());
    let w = mark_w + SIDE_GAP + SIDE_DOT;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let row = (mark_w * 4) as usize;
    for y in 0..h as usize {
        rgba[y * (w as usize) * 4..][..row].copy_from_slice(&mark.rgba()[y * row..][..row]);
    }
    paint_dot(&mut rgba, w, h, LENS_CENTRE, LENS_DOT_RADIUS, rgb);
    let side_r = SIDE_DOT as f32 / 2.0;
    paint_dot(&mut rgba, w, h, ((mark_w + SIDE_GAP) as f32 + side_r, h as f32 / 2.0), side_r, rgb);
    Image::new_owned(rgba, w, h)
}

fn paint_dot(rgba: &mut [u8], w: u32, h: u32, (cx, cy): (f32, f32), radius: f32, rgb: [u8; 3]) {
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let a = (radius + 0.5 - d).clamp(0.0, 1.0);
            if a == 0.0 {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            let under_a = rgba[i + 3] as f32 / 255.0;
            let out_a = a + under_a * (1.0 - a);
            for c in 0..3 {
                let blended = rgb[c] as f32 * a + rgba[i + c] as f32 * under_a * (1.0 - a);
                rgba[i + c] = (blended / out_a).round() as u8;
            }
            rgba[i + 3] = (out_a * 255.0).round() as u8;
        }
    }
}

fn build_menu(app: &AppHandle, failing: &[Run]) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    if !failing.is_empty() {
        menu.append(&MenuItem::with_id(app, "failing-header", "Failing", false, None::<&str>)?)?;
        for run in failing.iter().take(MENU_MAX_RUNS) {
            let branch = run.branch.as_deref().map(|b| format!(" · {b}")).unwrap_or_default();
            let label = format!("{} · {}{branch}", run.repo, run.title);
            menu.append(&MenuItem::with_id(app, format!("run:{}", run.key), label, true, None::<&str>)?)?;
        }
        if failing.len() > MENU_MAX_RUNS {
            let more = format!("{} more…", failing.len() - MENU_MAX_RUNS);
            menu.append(&MenuItem::with_id(app, "more", more, true, None::<&str>)?)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Pipescope", true, None::<&str>)?)?;
    Ok(menu)
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        DARK_MODE.store(matches!(window.theme(), Ok(Theme::Dark)), Ordering::Relaxed);
    }
    let (icon, tooltip) = icon_for(Health::Unknown, DARK_MODE.load(Ordering::Relaxed));
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(tooltip)
        .menu(&build_menu(app, &[])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "refresh" => {
                let state = app.state::<AppState>();
                for kind in ProviderKind::ALL {
                    state.scheduler.send(kind, Control::RefreshAll);
                }
            }
            "settings" => {
                show(app);
                let _ = app.emit("show-settings", ());
            }
            "more" => {
                show(app);
                let _ = app.emit("show-failing", ());
            }
            "quit" => app.exit(0),
            id => {
                let Some(key) = id.strip_prefix("run:") else { return };
                let run = app.state::<AppState>().store.lock().unwrap().get_run(key).ok().flatten();
                if let Some(run) = run {
                    let _ = app.opener().open_url(run.web_url, None::<&str>);
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        let w = window.clone();
        let handle = app.clone();
        window.on_window_event(move |event| match event {
            tauri::WindowEvent::Focused(false) => hide(&w),
            tauri::WindowEvent::ThemeChanged(theme) => {
                DARK_MODE.store(*theme == Theme::Dark, Ordering::Relaxed);
                refresh(&handle);
            }
            _ => {}
        });
    }
    refresh(app);
    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (runs, only_mine) = {
        let store = state.store.lock().unwrap();
        let muted = store.repo_flags("muted");
        let mut runs = store.recent_runs(Utc::now() - HEALTH_WINDOW).unwrap_or_default();
        runs.retain(|r| !muted.contains(&r.repo_id()));
        (runs, !store.notify_all())
    };
    let health = health(&runs, only_mine);
    let failing: Vec<Run> = failing(&runs, only_mine).into_iter().cloned().collect();
    // Off the main thread, tray calls block until the main thread runs them, and sync
    // commands on the main thread call this too; queueing the update avoids a deadlock.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || apply_tray(&handle, health, &failing));
}

fn apply_tray(app: &AppHandle, health: Health, failing: &[Run]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let (icon, tooltip) = icon_for(health, DARK_MODE.load(Ordering::Relaxed));
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_tooltip(Some(tooltip));
    let count = (!failing.is_empty()).then(|| failing.len().to_string());
    let _ = tray.set_title(count.as_deref());

    let keys: Vec<String> = failing.iter().map(|r| r.key.clone()).collect();
    let mut listed = MENU_RUNS.lock().unwrap();
    if listed.as_ref() != Some(&keys)
        && let Ok(menu) = build_menu(app, failing)
    {
        let _ = tray.set_menu(Some(menu));
        *listed = Some(keys);
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn hide(window: &WebviewWindow) {
    HIDDEN_AT_MS.store(now_ms(), Ordering::Relaxed);
    let _ = window.hide();
}

pub fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if window.is_visible().unwrap_or(false) {
        hide(&window);
    // Clicking the icon while the popover is open blurs it first; without this
    // guard the click would reopen what the blur just closed.
    } else if now_ms().saturating_sub(HIDDEN_AT_MS.load(Ordering::Relaxed)) > 300 {
        show(app);
    }
}

pub fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if window.move_window(Position::TrayBottomCenter).is_err() {
        let _ = window.move_window(Position::TopRight);
    }
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::run_key;
    use chrono::TimeZone;

    fn run(id: &str, minute: u32, branch: &str, status: RunStatus, mine: bool) -> Run {
        let t = Utc.with_ymd_and_hms(2026, 11, 9, 1, minute, 0).unwrap();
        Run {
            key: run_key(ProviderKind::Github, "o/r", id),
            provider: ProviderKind::Github,
            repo: "o/r".into(),
            id: id.into(),
            number: 1,
            title: "CI".into(),
            subtitle: None,
            branch: Some(branch.into()),
            commit: None,
            status,
            actor: None,
            mine,
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
    fn latest_run_per_branch_decides() {
        let fixed = [run("2", 5, "main", RunStatus::Success, true), run("1", 0, "main", RunStatus::Failed, true)];
        assert_eq!(health(&fixed, true), Health::Passing);
        let broken = [run("2", 5, "main", RunStatus::Failed, true), run("1", 0, "main", RunStatus::Success, true)];
        assert_eq!(health(&broken, true), Health::Failing);
    }

    #[test]
    fn running_is_amber_unless_something_failed() {
        let runs = [run("2", 5, "main", RunStatus::Running, true), run("1", 0, "dev", RunStatus::Success, true)];
        assert_eq!(health(&runs, true), Health::Running);
        let runs = [run("2", 5, "main", RunStatus::Running, true), run("1", 0, "dev", RunStatus::Failed, true)];
        assert_eq!(health(&runs, true), Health::Failing);
    }

    #[test]
    fn tray_icon_shows_status_in_the_lens_and_beside_it() {
        for dark in [false, true] {
            let mark = Image::from_bytes(if dark { MARK_WHITE } else { MARK_BLACK }).unwrap();
            let (icon, _) = icon_for(Health::Failing, dark);
            assert_eq!(icon.width(), mark.width() + SIDE_GAP + SIDE_DOT);
            assert_eq!(icon.height(), mark.height());
            let at = |x: u32, y: u32| {
                let i = ((y * icon.width() + x) * 4) as usize;
                icon.rgba()[i..i + 4].to_vec()
            };
            assert_eq!(at(LENS_CENTRE.0 as u32, LENS_CENTRE.1 as u32), [255, 59, 48, 255]);
            assert_eq!(at(icon.width() - SIDE_DOT / 2, icon.height() / 2), [255, 59, 48, 255]);
            let ink = if dark { 255 } else { 0 };
            let (w, h) = (icon.width(), icon.height());
            assert!((0..w).any(|x| at(x, h - 2) == [ink, ink, ink, 255]));
        }
    }

    #[test]
    fn failing_lists_only_the_latest_broken_workflows() {
        let runs = [
            run("3", 9, "dev", RunStatus::Failed, true),
            run("2", 5, "main", RunStatus::Success, true),
            run("1", 0, "main", RunStatus::Failed, true),
        ];
        let keys: Vec<&str> = failing(&runs, true).iter().map(|r| r.id.as_str()).collect();
        assert_eq!(keys, ["3"]);
    }

    #[test]
    fn mine_filter_ignores_others() {
        let runs = [run("1", 0, "main", RunStatus::Failed, false)];
        assert_eq!(health(&runs, true), Health::Unknown);
        assert_eq!(health(&runs, false), Health::Failing);
    }
}
