use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Github,
    Bitbucket,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 2] = [ProviderKind::Github, ProviderKind::Bitbucket];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Github => "github",
            ProviderKind::Bitbucket => "bitbucket",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Queued,
    Running,
    // Paused on a manual step or approval; not worth fast polling.
    Waiting,
    Success,
    Failed,
    Cancelled,
    Skipped,
}

impl RunStatus {
    pub fn is_active(self) -> bool {
        matches!(self, RunStatus::Queued | RunStatus::Running)
    }

    pub fn is_finished(self) -> bool {
        matches!(self, RunStatus::Success | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Skipped)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Waiting => "waiting",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => RunStatus::Queued,
            "running" => RunStatus::Running,
            "waiting" => RunStatus::Waiting,
            "success" => RunStatus::Success,
            "failed" => RunStatus::Failed,
            "cancelled" => RunStatus::Cancelled,
            "skipped" => RunStatus::Skipped,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Repo {
    pub full_name: String,
    pub last_activity: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub key: String,
    pub provider: ProviderKind,
    pub repo: String,
    pub id: String,
    pub number: u64,
    pub title: String,
    pub subtitle: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub status: RunStatus,
    pub actor: Option<String>,
    pub mine: bool,
    pub created_at: DateTime<Utc>,
    // Absent on runs saved before this field existed, and on Bitbucket.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub web_url: String,
    // Commit author's name, when the provider reports it.
    #[serde(default)]
    pub author: Option<String>,
    // Bitbucket's billed seconds. GitHub runs leave it empty; the scheduler sums their job times.
    #[serde(default)]
    pub build_secs: Option<u64>,
    // Carried across polls by the store: this run has been seen failing, so a pass
    // after a GitHub re-run of the same run counts as flaky.
    #[serde(default)]
    pub failed_before: bool,
    // The fields below are worked out when runs are read, never stored.
    #[serde(default)]
    pub flaky: bool,
    #[serde(default)]
    pub flaky_workflow: bool,
    #[serde(default)]
    pub watched: bool,
}

impl Run {
    // Identifies a repo across providers, for pinning and muting.
    pub fn repo_id(&self) -> String {
        format!("{}:{}", self.provider.as_str(), self.repo)
    }

    pub fn is_finished(&self) -> bool {
        self.status.is_finished()
    }
}

// How a log request chooses its job or step.
#[derive(Clone, Copy, Debug)]
pub enum StepPick<'a> {
    // The failing or running one.
    Auto,
    Id(&'a str),
    // Matching by name lets a log be compared with another run's.
    Name(&'a str),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDeployment {
    pub environment_id: u64,
    pub environment: String,
    pub can_approve: bool,
    pub reviewers: Vec<String>,
}

pub fn run_key(provider: ProviderKind, repo: &str, id: &str) -> String {
    format!("{}:{repo}:{id}", provider.as_str())
}

#[derive(Clone, Debug, Serialize)]
pub struct ChangedFile {
    pub path: String,
    // added, modified, removed or renamed, as each provider words it.
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStep {
    pub id: String,
    pub name: String,
    pub status: RunStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogTail {
    pub step_id: String,
    pub step: String,
    pub steps: Vec<LogStep>,
    pub text: String,
    // Lines dropped from the start of the log to keep it to the tail.
    pub skipped: usize,
    pub web_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateInfo {
    pub remaining: u32,
    pub limit: u32,
    pub reset_secs: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    NotConfigured,
    Connecting,
    Connected,
    Offline,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: ProviderKind,
    pub state: ConnState,
    pub account: Option<String>,
    pub message: Option<String>,
    pub repos: usize,
    pub source: Option<String>,
    pub last_poll: Option<DateTime<Utc>>,
    pub rate: Option<RateInfo>,
    pub warnings: Vec<String>,
}

impl ProviderStatus {
    pub fn new(provider: ProviderKind, state: ConnState) -> Self {
        Self {
            provider,
            state,
            account: None,
            message: None,
            repos: 0,
            source: None,
            last_poll: None,
            rate: None,
            warnings: Vec::new(),
        }
    }
}
