use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::{AppState, tray};
use crate::model::{Run, RunStatus};

#[derive(Debug, PartialEq)]
pub enum Change {
    Failed(Run),
    Recovered(Run),
}

impl Change {
    pub fn run(&self) -> &Run {
        match self {
            Change::Failed(r) | Change::Recovered(r) => r,
        }
    }
}

pub fn detect_changes(
    baseline: bool,
    old: &HashMap<String, RunStatus>,
    new: &[Run],
    history: &[Run],
) -> Vec<Change> {
    if baseline {
        return Vec::new();
    }
    new.iter()
        .filter(|run| old.get(&run.key) != Some(&run.status))
        .filter_map(|run| match run.status {
            RunStatus::Failed => Some(Change::Failed(run.clone())),
            RunStatus::Success if previous_outcome(history, run) == Some(RunStatus::Failed) => {
                Some(Change::Recovered(run.clone()))
            }
            _ => None,
        })
        .collect()
}

// Outcome of the latest earlier run of the same workflow on the same branch.
fn previous_outcome(history: &[Run], run: &Run) -> Option<RunStatus> {
    history
        .iter()
        .filter(|r| {
            r.key != run.key
                && r.title == run.title
                && r.branch == run.branch
                && r.created_at < run.created_at
                && matches!(r.status, RunStatus::Success | RunStatus::Failed)
        })
        .max_by_key(|r| r.created_at)
        .map(|r| r.status)
}

#[derive(Debug, PartialEq)]
pub enum Target {
    Run(String),
    Failing,
    Popover,
}

#[derive(Default)]
pub struct Notifier {
    next: AtomicI32,
    targets: Mutex<HashMap<i32, Target>>,
    queue: Mutex<Vec<Change>>,
    flushing: AtomicBool,
}

// Repos are polled seconds apart, so a broken shared dependency would otherwise fire
// one notification per repo.
const BATCH_WINDOW: Duration = Duration::from_secs(3);
const MAX_TARGETS: i32 = 100;

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let handle = app.clone();
    let _ = app.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        let Some(id) = action.notification().map(|n| n.id()) else { return };
        let target = handle.state::<AppState>().notifier.targets.lock().unwrap().remove(&id);
        let Some(target) = target else { return };
        tray::show(&handle);
        let _ = match target {
            Target::Run(key) => handle.emit("focus-run", key),
            Target::Failing => handle.emit("show-failing", ()),
            Target::Popover => Ok(()),
        };
    });
    Ok(())
}

pub fn send_all(app: &AppHandle, changes: Vec<Change>) {
    let state = app.state::<AppState>();
    let (all, muted) = {
        let store = state.store.lock().unwrap();
        (store.notify_all(), store.repo_flags("muted"))
    };
    let wanted: Vec<Change> = changes
        .into_iter()
        .filter(|c| (all || c.run().mine) && !muted.contains(&c.run().repo_id()))
        .collect();
    if wanted.is_empty() {
        return;
    }
    state.notifier.queue.lock().unwrap().extend(wanted);
    if state.notifier.flushing.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(BATCH_WINDOW).await;
        let state = app.state::<AppState>();
        state.notifier.flushing.store(false, Ordering::SeqCst);
        let batch = std::mem::take(&mut *state.notifier.queue.lock().unwrap());
        let (failed, fixed): (Vec<_>, Vec<_>) = batch.into_iter().partition(|c| matches!(c, Change::Failed(_)));
        for (title, body, target) in [message(&failed), message(&fixed)].into_iter().flatten() {
            let id = state.notifier.next.fetch_add(1, Ordering::Relaxed);
            let mut targets = state.notifier.targets.lock().unwrap();
            targets.insert(id, target);
            // Entries only leave on click; old notifications are long gone from Notification Centre.
            targets.retain(|&k, _| k > id - MAX_TARGETS);
            drop(targets);
            if let Err(e) = app.notification().builder().id(id).title(title).body(body).show() {
                eprintln!("notification failed: {e}");
            }
        }
    });
}

// `group` holds changes of one kind.
fn message(group: &[Change]) -> Option<(String, String, Target)> {
    match group {
        [] => None,
        [one] => {
            let run = one.run();
            let (title, verb) = match one {
                Change::Failed(_) => (format!("Failed: {}", run.repo), "failed"),
                Change::Recovered(_) => (format!("Fixed: {}", run.repo), "passed again"),
            };
            let branch = run.branch.as_deref().map(|b| format!(" on {b}")).unwrap_or_default();
            let actor = run.actor.as_deref().map(|a| format!(" ({a})")).unwrap_or_default();
            let body = format!("{} #{}{branch} {verb}{actor}", run.title, run.number);
            Some((title, body, Target::Run(run.key.clone())))
        }
        many => {
            let failed = matches!(many[0], Change::Failed(_));
            let title = if failed {
                format!("{} runs failed", many.len())
            } else {
                format!("{} runs passed again", many.len())
            };
            let mut names: Vec<String> = many.iter().map(|c| format!("{} ({})", c.run().repo, c.run().title)).collect();
            names.dedup();
            let body = match names.len() {
                n if n > 3 => format!("{} and {} more", names[..3].join(", "), n - 3),
                _ => names.join(", "),
            };
            Some((title, body, if failed { Target::Failing } else { Target::Popover }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderKind, run_key};
    use chrono::{TimeZone, Utc};

    fn run(id: &str, minute: u32, branch: &str, status: RunStatus) -> Run {
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
            mine: true,
            created_at: t,
            started_at: None,
            updated_at: t,
            web_url: String::new(),
        }
    }

    #[test]
    fn baseline_never_notifies() {
        let new = [run("1", 0, "main", RunStatus::Failed)];
        assert!(detect_changes(true, &HashMap::new(), &new, &new).is_empty());
    }

    #[test]
    fn notifies_new_failures_once() {
        let failed = run("1", 0, "main", RunStatus::Failed);
        let new = [failed.clone()];
        assert_eq!(detect_changes(false, &HashMap::new(), &new, &new), [Change::Failed(failed.clone())]);
        let old = HashMap::from([(failed.key.clone(), RunStatus::Failed)]);
        assert!(detect_changes(false, &old, &new, &new).is_empty());
    }

    #[test]
    fn batches_several_failures_into_one_message() {
        let one = [Change::Failed(run("1", 0, "main", RunStatus::Failed))];
        let (title, _, target) = message(&one).unwrap();
        assert_eq!(title, "Failed: o/r");
        assert_eq!(target, Target::Run(one[0].run().key.clone()));

        let many: Vec<Change> = (0..5).map(|i| Change::Failed(run(&i.to_string(), i, "main", RunStatus::Failed))).collect();
        let (title, body, target) = message(&many).unwrap();
        assert_eq!(title, "5 runs failed");
        assert_eq!(body, "o/r (CI)");
        assert_eq!(target, Target::Failing);
        assert!(message(&[]).is_none());
    }

    #[test]
    fn recovery_needs_a_previous_failure_on_the_same_branch() {
        let fixed = run("2", 5, "main", RunStatus::Success);
        let history = [fixed.clone(), run("1", 0, "main", RunStatus::Failed), run("0", 1, "dev", RunStatus::Success)];
        let old = HashMap::from([(fixed.key.clone(), RunStatus::Running)]);
        assert_eq!(detect_changes(false, &old, std::slice::from_ref(&fixed), &history), [Change::Recovered(fixed.clone())]);

        let other_branch = [fixed.clone(), run("1", 0, "dev", RunStatus::Failed)];
        assert!(detect_changes(false, &old, std::slice::from_ref(&fixed), &other_branch).is_empty());
    }
}
