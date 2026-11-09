use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use url::Url;

use super::{split_repo, tail_log};
use crate::error::{Error, Result};
use crate::http::{Api, Auth, Fetched};
use crate::model::{LogStep, LogTail, ProviderKind, RateInfo, Repo, Run, RunStatus, run_key};

const API: &str = "https://api.bitbucket.org/2.0/";
const WEB: &str = "https://bitbucket.org";
// Bitbucket doesn't report remaining budget in headers; 1,000/hour is its documented
// limit for repository data.
const HOURLY_BUDGET: u32 = 1000;

#[derive(Clone)]
pub struct Bitbucket {
    api: Api,
    uuid: String,
    name: String,
}

#[derive(Deserialize)]
struct User {
    uuid: String,
    display_name: String,
}

#[derive(Deserialize)]
struct Page<T> {
    values: Vec<T>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct BbRepo {
    full_name: String,
    updated_on: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct Pipeline {
    uuid: String,
    build_number: u64,
    created_on: DateTime<Utc>,
    completed_on: Option<DateTime<Utc>>,
    state: State,
    creator: Option<Creator>,
    #[serde(default)]
    target: Target,
}

#[derive(Deserialize)]
struct State {
    name: String,
    result: Option<Named>,
    stage: Option<Named>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Creator {
    uuid: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize, Default)]
struct Target {
    ref_name: Option<String>,
    commit: Option<Commit>,
    selector: Option<Selector>,
}

#[derive(Deserialize)]
struct Commit {
    hash: String,
}

#[derive(Deserialize)]
struct Selector {
    #[serde(rename = "type")]
    kind: String,
    pattern: Option<String>,
}

#[derive(Deserialize)]
struct Step {
    uuid: String,
    name: Option<String>,
    state: State,
}

impl Bitbucket {
    pub async fn connect(email: String, token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let api = Api::new("Bitbucket", API, Auth::Basic { user: email, secret: token }, HOURLY_BUDGET, headers)?;
        let me: User = api.get(api.url(&["user"])).await?;
        Ok(Self { api, uuid: me.uuid, name: me.display_name })
    }

    pub fn account(&self) -> &str {
        &self.name
    }

    pub async fn rate(&self) -> RateInfo {
        self.api.rate().await
    }

    async fn pages<T: DeserializeOwned>(&self, first: Url) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next.take() {
            let page: Page<T> = self.api.get(url).await?;
            out.extend(page.values);
            next = page.next.and_then(|l| Url::parse(&l).ok());
        }
        Ok(out)
    }

    // Bitbucket removed the cross-workspace `GET /repositories` listing, so walk each workspace.
    pub async fn list_repos(&self) -> Result<Vec<Repo>> {
        let mut url = self.api.url(&["user", "workspaces"]);
        url.query_pairs_mut().append_pair("pagelen", "100");
        let access: Vec<Value> = self.pages(url).await.map_err(|e| match e {
            Error::Status { code: 403, .. } => Error::Other("The Bitbucket token needs the read:workspace scope.".into()),
            e => e,
        })?;
        let mut repos = Vec::new();
        for slug in access.iter().filter_map(workspace_slug) {
            let mut url = self.api.url(&["repositories", slug]);
            url.query_pairs_mut()
                .append_pair("role", "member")
                .append_pair("pagelen", "100")
                .append_pair("sort", "-updated_on")
                .append_pair("fields", "next,values.full_name,values.updated_on");
            let page: Vec<BbRepo> = match self.pages(url).await {
                Ok(page) => page,
                Err(e) if e.is_access_denied() => {
                    eprintln!("skipping Bitbucket workspace {slug}: {e}");
                    continue;
                }
                Err(e) => return Err(e),
            };
            repos.extend(page.into_iter().map(|r| Repo { full_name: r.full_name, last_activity: r.updated_on }));
        }
        Ok(repos)
    }

    pub async fn list_runs(&self, repo: &str, etag: Option<&str>) -> Result<Fetched<Vec<Run>>> {
        let (ws, slug) = split_repo(repo)?;
        let mut url = self.api.url(&["repositories", ws, slug, "pipelines", ""]);
        url.query_pairs_mut().append_pair("sort", "-created_on").append_pair("pagelen", "20");
        match self.api.get_json::<Page<Pipeline>>(url, etag).await {
            Ok(fetched) => Ok(fetched.map(|page| page.values.into_iter().map(|p| self.to_run(repo, p)).collect())),
            // Pipelines not enabled for this repo.
            Err(e) if e.is_access_denied() => Ok(Fetched::Fresh { body: Vec::new(), etag: None, next: None }),
            Err(e) => Err(e),
        }
    }

    fn to_run(&self, repo: &str, p: Pipeline) -> Run {
        let title = match &p.target.selector {
            Some(Selector { kind, pattern: Some(pattern) }) if kind == "custom" => format!("Custom: {pattern}"),
            Some(s) if s.kind == "pull-requests" => "Pull request".into(),
            Some(s) if s.kind == "tags" => "Tag".into(),
            _ => "Pipeline".into(),
        };
        let creator = p.creator.unwrap_or(Creator { uuid: None, display_name: None });
        Run {
            key: run_key(ProviderKind::Bitbucket, repo, &p.uuid),
            provider: ProviderKind::Bitbucket,
            repo: repo.to_string(),
            number: p.build_number,
            title,
            subtitle: None,
            branch: p.target.ref_name,
            commit: p.target.commit.map(|c| c.hash),
            status: status(
                &p.state.name,
                p.state.result.as_ref().map(|r| r.name.as_str()),
                p.state.stage.as_ref().map(|s| s.name.as_str()),
            ),
            mine: creator.uuid.as_deref() == Some(self.uuid.as_str()),
            actor: creator.display_name,
            created_at: p.created_on,
            started_at: None,
            updated_at: p.completed_on.unwrap_or(p.created_on),
            web_url: format!("{WEB}/{repo}/pipelines/results/{}", p.build_number),
            id: p.uuid,
        }
    }

    // Bitbucket has no re-run endpoint, so this starts a new pipeline on the same target.
    pub async fn rerun(&self, run: &Run) -> Result<()> {
        let (ws, slug) = split_repo(&run.repo)?;
        let pipeline: Value = self.api.get(self.api.url(&["repositories", ws, slug, "pipelines", &run.id])).await?;
        let target = pipeline.get("target").ok_or_else(|| Error::Other("pipeline has no target".into()))?;
        let body = json!({ "target": rerun_target(target) });
        self.api.post(self.api.url(&["repositories", ws, slug, "pipelines", ""]), Some(&body)).await
    }

    pub async fn cancel(&self, run: &Run) -> Result<()> {
        let (ws, slug) = split_repo(&run.repo)?;
        self.api.post(self.api.url(&["repositories", ws, slug, "pipelines", &run.id, "stopPipeline"]), None).await
    }

    pub async fn log_tail(&self, run: &Run, step: Option<&str>) -> Result<LogTail> {
        let (ws, slug) = split_repo(&run.repo)?;
        let mut url = self.api.url(&["repositories", ws, slug, "pipelines", &run.id, "steps", ""]);
        url.query_pairs_mut().append_pair("pagelen", "100");
        let all: Vec<Step> = self.pages(url).await?;
        let steps = all
            .iter()
            .map(|s| LogStep { id: s.uuid.clone(), name: step_name(s), status: step_status(s) })
            .collect();
        let chosen = match step {
            Some(id) => all.iter().find(|s| s.uuid == id),
            None => pick_step(&all),
        };
        let Some(chosen) = chosen else {
            return Ok(LogTail {
                step_id: String::new(),
                step: String::new(),
                steps,
                text: "This pipeline has no steps.".into(),
                skipped: 0,
                web_url: run.web_url.clone(),
            });
        };
        let web_url = format!("{}/steps/{}", run.web_url, chosen.uuid);
        let log_url = self.api.url(&["repositories", ws, slug, "pipelines", &run.id, "steps", &chosen.uuid, "log"]);
        let (text, skipped) = match self.api.get_text(log_url).await {
            Ok(raw) => tail_log(&raw),
            Err(e) if e.is_access_denied() => ("This step has no log yet.".into(), 0),
            Err(e) => return Err(e),
        };
        Ok(LogTail { step_id: chosen.uuid.clone(), step: step_name(chosen), steps, text, skipped, web_url })
    }
}

fn step_name(s: &Step) -> String {
    s.name.clone().unwrap_or_else(|| "Step".into())
}

fn step_status(s: &Step) -> RunStatus {
    status(&s.state.name, s.state.result.as_ref().map(|r| r.name.as_str()), s.state.stage.as_ref().map(|r| r.name.as_str()))
}

// Entries are `workspace_access` objects wrapping the workspace; accept a bare workspace too.
fn workspace_slug(entry: &Value) -> Option<&str> {
    entry.get("workspace").unwrap_or(entry).get("slug").and_then(Value::as_str)
}

fn pick_step(steps: &[Step]) -> Option<&Step> {
    steps
        .iter()
        .find(|s| matches!(s.state.result.as_ref().map(|r| r.name.as_str()), Some("FAILED" | "ERROR")))
        .or_else(|| steps.iter().find(|s| s.state.name == "IN_PROGRESS"))
        .or(steps.last())
}

fn status(state: &str, result: Option<&str>, stage: Option<&str>) -> RunStatus {
    match state {
        "PENDING" => RunStatus::Queued,
        "IN_PROGRESS" if matches!(stage, Some("PAUSED" | "HALTED")) => RunStatus::Waiting,
        "IN_PROGRESS" => RunStatus::Running,
        "PAUSED" | "HALTED" => RunStatus::Waiting,
        _ => match result {
            Some("SUCCESSFUL") => RunStatus::Success,
            Some("FAILED" | "ERROR") => RunStatus::Failed,
            Some("STOPPED" | "EXPIRED") => RunStatus::Cancelled,
            _ => RunStatus::Skipped,
        },
    }
}

// The fetched target carries read-only fields (links, commit details) that the
// trigger endpoint doesn't accept, so copy only what identifies the target.
fn rerun_target(target: &Value) -> Value {
    let mut out = Map::new();
    for key in ["type", "ref_type", "ref_name", "selector", "source", "destination"] {
        if let Some(v) = target.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    for key in ["commit", "destination_commit"] {
        if let Some(hash) = target.get(key).and_then(|c| c.get("hash")) {
            out.insert(key.into(), json!({ "type": "commit", "hash": hash }));
        }
    }
    if let Some(id) = target.get("pullrequest").and_then(|p| p.get("id")) {
        out.insert("pullrequest".into(), json!({ "id": id }));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_statuses() {
        assert_eq!(status("PENDING", None, None), RunStatus::Queued);
        assert_eq!(status("IN_PROGRESS", None, Some("RUNNING")), RunStatus::Running);
        assert_eq!(status("IN_PROGRESS", None, Some("PAUSED")), RunStatus::Waiting);
        assert_eq!(status("COMPLETED", Some("SUCCESSFUL"), None), RunStatus::Success);
        assert_eq!(status("COMPLETED", Some("FAILED"), None), RunStatus::Failed);
        assert_eq!(status("COMPLETED", Some("ERROR"), None), RunStatus::Failed);
        assert_eq!(status("COMPLETED", Some("STOPPED"), None), RunStatus::Cancelled);
    }

    #[test]
    fn parses_pipeline_fixture() {
        let json = r#"{"values":[{"uuid":"{p-1}","build_number":12,"created_on":"2026-11-09T01:00:00.123456+00:00",
            "completed_on":null,"state":{"name":"IN_PROGRESS","type":"pipeline_state_in_progress","stage":{"name":"RUNNING"}},
            "creator":{"display_name":"Grant","uuid":"{me}"},
            "target":{"type":"pipeline_ref_target","ref_type":"branch","ref_name":"main",
            "commit":{"type":"commit","hash":"abc"},"selector":{"type":"branches","pattern":"main"}}}]}"#;
        let page: Page<Pipeline> = serde_json::from_str(json).unwrap();
        let bb = Bitbucket {
            api: Api::new("Bitbucket", API, Auth::Bearer(String::new()), HOURLY_BUDGET, HeaderMap::new()).unwrap(),
            uuid: "{me}".into(),
            name: "Grant".into(),
        };
        let run = bb.to_run("ws/repo", page.values.into_iter().next().unwrap());
        assert_eq!(run.key, "bitbucket:ws/repo:{p-1}");
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.branch.as_deref(), Some("main"));
        assert_eq!(run.web_url, "https://bitbucket.org/ws/repo/pipelines/results/12");
        assert!(run.mine);
    }

    #[test]
    fn reads_workspace_slugs() {
        assert_eq!(workspace_slug(&json!({"type": "workspace_access", "workspace": {"slug": "hatchd"}})), Some("hatchd"));
        assert_eq!(workspace_slug(&json!({"type": "workspace", "slug": "solo"})), Some("solo"));
        assert_eq!(workspace_slug(&json!({"type": "workspace_access"})), None);
    }

    #[test]
    fn strips_target_for_rerun() {
        let fetched = json!({
            "type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "main",
            "commit": {"type": "commit", "hash": "abc", "links": {"self": {"href": "x"}}},
            "selector": {"type": "custom", "pattern": "deploy"}
        });
        assert_eq!(
            rerun_target(&fetched),
            json!({
                "type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "main",
                "commit": {"type": "commit", "hash": "abc"},
                "selector": {"type": "custom", "pattern": "deploy"}
            })
        );
    }

    #[test]
    fn builds_encoded_step_urls() {
        let api = Api::new("Bitbucket", API, Auth::Bearer(String::new()), HOURLY_BUDGET, HeaderMap::new()).unwrap();
        let url = api.url(&["repositories", "ws", "r", "pipelines", "{p-1}", "steps", ""]);
        assert_eq!(url.as_str(), "https://api.bitbucket.org/2.0/repositories/ws/r/pipelines/%7Bp-1%7D/steps/");
    }
}
