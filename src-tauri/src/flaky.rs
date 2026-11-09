use std::collections::{HashMap, HashSet};

use crate::model::{Run, RunStatus};

fn same_workflow(a: &Run, b: &Run) -> bool {
    a.provider == b.provider && a.repo == b.repo && a.title == b.title
}

// An earlier failure of this workflow on the same commit, as Bitbucket re-runs start a new pipeline.
fn failed_earlier_on_commit(group: &[&Run], run: &Run) -> bool {
    run.commit.is_some()
        && group.iter().any(|r| {
            r.key != run.key && r.commit == run.commit && r.status == RunStatus::Failed && r.created_at < run.created_at
        })
}

// Passed after failing on the same commit: nothing changed, so the failure was noise.
fn passed_on_retry(group: &[&Run], run: &Run) -> bool {
    run.status == RunStatus::Success && (run.failed_before || failed_earlier_on_commit(group, run))
}

// Already a second attempt, so automatic re-runs never loop.
pub fn is_retry(history: &[Run], run: &Run) -> bool {
    let group: Vec<&Run> = history.iter().filter(|r| same_workflow(r, run)).collect();
    run.failed_before || failed_earlier_on_commit(&group, run)
}

// Fills in `flaky`, `flaky_workflow` and `watched`. Only runs read back from the store
// carry `failed_before`.
pub fn annotate(runs: &mut [Run], watched: &HashSet<String>) {
    let mut flaky = vec![false; runs.len()];
    {
        let mut groups: HashMap<(&str, &str, &str), Vec<&Run>> = HashMap::new();
        for r in runs.iter() {
            groups.entry((r.provider.as_str(), &r.repo, &r.title)).or_default().push(r);
        }
        for (i, r) in runs.iter().enumerate() {
            flaky[i] = passed_on_retry(&groups[&(r.provider.as_str(), r.repo.as_str(), r.title.as_str())], r);
        }
    }
    let flaky_workflows: HashSet<(String, String, String)> = runs
        .iter()
        .zip(&flaky)
        .filter(|(_, f)| **f)
        .map(|(r, _)| (r.provider.as_str().to_string(), r.repo.clone(), r.title.clone()))
        .collect();
    for (r, f) in runs.iter_mut().zip(flaky) {
        r.flaky = f;
        r.flaky_workflow = r.status == RunStatus::Failed
            && flaky_workflows.contains(&(r.provider.as_str().to_string(), r.repo.clone(), r.title.clone()));
        r.watched = watched.contains(&r.key);
    }
}

// A first failure of a workflow that has recently passed on retry.
pub fn should_auto_rerun(history: &[Run], run: &Run) -> bool {
    if run.status != RunStatus::Failed || is_retry(history, run) {
        return false;
    }
    let mut annotated: Vec<Run> = history.iter().filter(|r| same_workflow(r, run)).cloned().collect();
    annotate(&mut annotated, &HashSet::new());
    annotated.iter().any(|r| r.flaky)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderKind, run_key};
    use chrono::{TimeZone, Utc};

    fn run(id: &str, minute: u32, commit: &str, status: RunStatus) -> Run {
        let t = Utc.with_ymd_and_hms(2026, 11, 9, 1, minute, 0).unwrap();
        Run {
            key: run_key(ProviderKind::Bitbucket, "o/r", id),
            provider: ProviderKind::Bitbucket,
            repo: "o/r".into(),
            id: id.into(),
            number: minute as u64,
            title: "Pipeline".into(),
            subtitle: None,
            branch: Some("main".into()),
            commit: Some(commit.into()),
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
    fn pass_after_failure_on_same_commit_is_flaky() {
        let mut runs = vec![
            run("3", 30, "bbb", RunStatus::Failed),
            run("2", 20, "aaa", RunStatus::Success),
            run("1", 10, "aaa", RunStatus::Failed),
        ];
        annotate(&mut runs, &HashSet::new());
        assert!(runs[1].flaky);
        assert!(!runs[2].flaky);
        assert!(runs[0].flaky_workflow, "a new failure of a workflow known to be flaky");
    }

    #[test]
    fn pass_after_a_fix_is_not_flaky() {
        let mut runs = vec![run("2", 20, "bbb", RunStatus::Success), run("1", 10, "aaa", RunStatus::Failed)];
        let watched = HashSet::from([runs[0].key.clone()]);
        annotate(&mut runs, &watched);
        assert!(!runs[0].flaky);
        assert!(runs[0].watched);
    }

    #[test]
    fn github_rerun_of_the_same_run_counts() {
        let mut passed = run("1", 10, "aaa", RunStatus::Success);
        passed.failed_before = true;
        let mut runs = vec![passed];
        annotate(&mut runs, &HashSet::new());
        assert!(runs[0].flaky);
    }

    #[test]
    fn auto_rerun_only_first_failures_of_flaky_workflows() {
        let history = vec![
            run("4", 40, "ccc", RunStatus::Failed),
            run("2", 20, "aaa", RunStatus::Success),
            run("1", 10, "aaa", RunStatus::Failed),
        ];
        assert!(should_auto_rerun(&history, &history[0]));

        let mut retried = history.clone();
        retried.insert(0, run("5", 50, "ccc", RunStatus::Failed));
        assert!(!should_auto_rerun(&retried, &retried[0]), "already a retry of ccc");

        let steady = vec![run("4", 40, "ccc", RunStatus::Failed), run("1", 10, "aaa", RunStatus::Success)];
        assert!(!should_auto_rerun(&steady, &steady[0]));
    }
}
