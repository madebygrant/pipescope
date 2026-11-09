use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::Deserialize;
use url::Url;

use super::{split_repo, tail_log};
use crate::error::Result;
use crate::http::{Api, Auth, Fetched};
use crate::model::{LogStep, LogTail, ProviderKind, RateInfo, Repo, Run, RunStatus, run_key};

const API: &str = "https://api.github.com/";

#[derive(Clone)]
pub struct GitHub {
    api: Api,
    login: String,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
    pushed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    disabled: bool,
}

#[derive(Deserialize)]
struct RunsPage {
    workflow_runs: Vec<GhRun>,
}

#[derive(Deserialize)]
struct GhRun {
    id: u64,
    name: Option<String>,
    display_title: Option<String>,
    run_number: u64,
    head_branch: Option<String>,
    head_sha: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: String,
    created_at: DateTime<Utc>,
    run_started_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    actor: Option<User>,
    triggering_actor: Option<User>,
}

#[derive(Deserialize)]
struct JobsPage {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    name: String,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: Option<String>,
}

impl GitHub {
    pub async fn connect(token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("x-github-api-version", HeaderValue::from_static("2022-11-28"));
        let api = Api::new("GitHub", API, Auth::Bearer(token), 5000, headers)?;
        let (me, headers): (User, _) = api.get_with_headers(api.url(&["user"])).await?;
        let warnings = missing_scopes(headers.get("x-oauth-scopes").and_then(|v| v.to_str().ok()));
        Ok(Self { api, login: me.login, warnings })
    }

    pub fn account(&self) -> &str {
        &self.login
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }

    pub async fn rate(&self) -> RateInfo {
        self.api.rate().await
    }

    pub async fn list_repos(&self) -> Result<Vec<Repo>> {
        let mut url = self.api.url(&["user", "repos"]);
        url.query_pairs_mut()
            .append_pair("affiliation", "owner,collaborator,organization_member")
            .append_pair("sort", "pushed")
            .append_pair("per_page", "100");
        let mut repos = Vec::new();
        let mut next = Some(url);
        while let Some(url) = next.take() {
            let Fetched::Fresh { body, next: link, .. } = self.api.get_json::<Vec<GhRepo>>(url, None).await? else {
                break;
            };
            repos.extend(
                body.into_iter()
                    .filter(|r| !r.archived && !r.disabled)
                    .map(|r| Repo { full_name: r.full_name, last_activity: r.pushed_at }),
            );
            next = link.and_then(|l| Url::parse(&l).ok());
        }
        Ok(repos)
    }

    pub async fn list_runs(&self, repo: &str, etag: Option<&str>) -> Result<Fetched<Vec<Run>>> {
        let (owner, name) = split_repo(repo)?;
        let mut url = self.api.url(&["repos", owner, name, "actions", "runs"]);
        url.query_pairs_mut().append_pair("per_page", "20");
        match self.api.get_json::<RunsPage>(url, etag).await {
            Ok(fetched) => Ok(fetched.map(|page| page.workflow_runs.into_iter().map(|r| self.to_run(repo, r)).collect())),
            // Actions disabled, or an org that needs the token SSO-authorised.
            Err(e) if e.is_access_denied() => Ok(Fetched::Fresh { body: Vec::new(), etag: None, next: None }),
            Err(e) => Err(e),
        }
    }

    fn to_run(&self, repo: &str, r: GhRun) -> Run {
        let actor = r.triggering_actor.or(r.actor).map(|u| u.login);
        let id = r.id.to_string();
        Run {
            key: run_key(ProviderKind::Github, repo, &id),
            provider: ProviderKind::Github,
            repo: repo.to_string(),
            id,
            number: r.run_number,
            title: r.name.unwrap_or_else(|| "Workflow".into()),
            subtitle: r.display_title,
            branch: r.head_branch,
            commit: r.head_sha,
            status: status(r.status.as_deref(), r.conclusion.as_deref()),
            mine: actor.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&self.login)),
            actor,
            created_at: r.created_at,
            started_at: r.run_started_at,
            updated_at: r.updated_at,
            web_url: r.html_url,
        }
    }

    pub async fn rerun(&self, run: &Run, failed_only: bool) -> Result<()> {
        let (owner, name) = split_repo(&run.repo)?;
        let action = if failed_only { "rerun-failed-jobs" } else { "rerun" };
        self.api.post(self.api.url(&["repos", owner, name, "actions", "runs", &run.id, action]), None).await
    }

    pub async fn cancel(&self, run: &Run) -> Result<()> {
        let (owner, name) = split_repo(&run.repo)?;
        self.api.post(self.api.url(&["repos", owner, name, "actions", "runs", &run.id, "cancel"]), None).await
    }

    pub async fn log_tail(&self, run: &Run, step: Option<&str>) -> Result<LogTail> {
        let (owner, name) = split_repo(&run.repo)?;
        let mut url = self.api.url(&["repos", owner, name, "actions", "runs", &run.id, "jobs"]);
        url.query_pairs_mut().append_pair("filter", "latest").append_pair("per_page", "100");
        let page: JobsPage = self.api.get(url).await?;
        let steps = page
            .jobs
            .iter()
            .map(|j| LogStep {
                id: j.id.to_string(),
                name: j.name.clone(),
                status: status(j.status.as_deref(), j.conclusion.as_deref()),
            })
            .collect();
        let job = match step {
            Some(id) => page.jobs.iter().find(|j| j.id.to_string() == id),
            None => pick_job(&page.jobs),
        };
        let Some(job) = job else {
            return Ok(LogTail {
                step_id: String::new(),
                step: String::new(),
                steps,
                text: "This run has no jobs.".into(),
                skipped: 0,
                web_url: run.web_url.clone(),
            });
        };
        let web_url = job.html_url.clone().unwrap_or_else(|| run.web_url.clone());
        let job_id = job.id.to_string();
        let (text, skipped) = match self.api.get_text(self.api.url(&["repos", owner, name, "actions", "jobs", &job_id, "logs"])).await {
            Ok(raw) => tail_log(&raw),
            // GitHub only serves a job's log once the job has finished.
            Err(e) if e.is_access_denied() => ("The log is available once this job finishes.".into(), 0),
            Err(e) => return Err(e),
        };
        Ok(LogTail { step_id: job_id, step: job.name.clone(), steps, text, skipped, web_url })
    }
}

// Classic and OAuth tokens list their scopes in this header; fine-grained tokens
// don't send it, so there's nothing to check up front.
fn missing_scopes(header: Option<&str>) -> Vec<String> {
    let Some(header) = header else { return Vec::new() };
    if header.split(',').any(|s| s.trim() == "repo") {
        Vec::new()
    } else {
        vec!["The GitHub token lacks the repo scope, so private repos are hidden and re-run and cancel will fail.".into()]
    }
}

fn pick_job(jobs: &[Job]) -> Option<&Job> {
    jobs.iter()
        .find(|j| matches!(j.conclusion.as_deref(), Some("failure" | "timed_out")))
        .or_else(|| jobs.iter().find(|j| j.status.as_deref() == Some("in_progress")))
        .or(jobs.last())
}

fn status(status: Option<&str>, conclusion: Option<&str>) -> RunStatus {
    match status {
        Some("completed") => match conclusion {
            Some("success" | "neutral") => RunStatus::Success,
            Some("cancelled") => RunStatus::Cancelled,
            Some("skipped" | "stale") => RunStatus::Skipped,
            Some("action_required") => RunStatus::Waiting,
            _ => RunStatus::Failed,
        },
        Some("in_progress") => RunStatus::Running,
        Some("waiting" | "action_required") => RunStatus::Waiting,
        _ => RunStatus::Queued,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_statuses() {
        assert_eq!(status(Some("completed"), Some("success")), RunStatus::Success);
        assert_eq!(status(Some("completed"), Some("failure")), RunStatus::Failed);
        assert_eq!(status(Some("completed"), Some("timed_out")), RunStatus::Failed);
        assert_eq!(status(Some("completed"), Some("startup_failure")), RunStatus::Failed);
        assert_eq!(status(Some("completed"), Some("cancelled")), RunStatus::Cancelled);
        assert_eq!(status(Some("completed"), Some("skipped")), RunStatus::Skipped);
        assert_eq!(status(Some("in_progress"), None), RunStatus::Running);
        assert_eq!(status(Some("waiting"), None), RunStatus::Waiting);
        assert_eq!(status(Some("queued"), None), RunStatus::Queued);
    }

    #[test]
    fn parses_run_fixture() {
        let json = r#"{"total_count":1,"workflow_runs":[{"id":42,"name":"CI","display_title":"Fix login",
            "run_number":7,"head_branch":"main","head_sha":"abc","status":"completed","conclusion":"failure",
            "html_url":"https://github.com/o/r/actions/runs/42","created_at":"2026-11-09T01:00:00Z",
            "updated_at":"2026-11-09T01:05:00Z","actor":{"login":"bot"},"triggering_actor":{"login":"Grant"}}]}"#;
        let page: RunsPage = serde_json::from_str(json).unwrap();
        let gh = GitHub {
            api: Api::new("GitHub", API, Auth::Bearer(String::new()), 5000, HeaderMap::new()).unwrap(),
            login: "grant".into(),
            warnings: Vec::new(),
        };
        let run = gh.to_run("o/r", page.workflow_runs.into_iter().next().unwrap());
        assert_eq!(run.key, "github:o/r:42");
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.actor.as_deref(), Some("Grant"));
        assert!(run.mine);
        assert_eq!(run.subtitle.as_deref(), Some("Fix login"));
    }

    #[test]
    fn flags_a_missing_repo_scope() {
        assert!(missing_scopes(None).is_empty());
        assert!(missing_scopes(Some("gist, read:org, repo, workflow")).is_empty());
        assert_eq!(missing_scopes(Some("read:org, workflow")).len(), 1);
    }

    #[test]
    fn picks_failed_job_first() {
        let job = |id, status: &str, conclusion: Option<&str>| Job {
            id,
            name: format!("job{id}"),
            status: Some(status.into()),
            conclusion: conclusion.map(Into::into),
            html_url: None,
        };
        let jobs = vec![job(1, "completed", Some("success")), job(2, "completed", Some("failure")), job(3, "in_progress", None)];
        assert_eq!(pick_job(&jobs).unwrap().id, 2);
        assert_eq!(pick_job(&jobs[2..]).unwrap().id, 3);
        assert_eq!(pick_job(&jobs[..1]).unwrap().id, 1);
    }
}
