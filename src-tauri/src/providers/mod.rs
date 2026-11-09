pub mod bitbucket;
pub mod github;

use crate::error::{Error, Result};
use crate::http::Fetched;
use crate::model::{LogTail, ProviderKind, RateInfo, Repo, Run};

use bitbucket::Bitbucket;
use github::GitHub;

const LOG_TAIL_LINES: usize = 500;

#[derive(Clone)]
pub enum Creds {
    Github(String),
    Bitbucket { email: String, token: String },
}

#[derive(Clone)]
pub enum Client {
    Github(GitHub),
    Bitbucket(Bitbucket),
}

impl Client {
    pub async fn connect(creds: Creds) -> Result<Self> {
        Ok(match creds {
            Creds::Github(token) => Client::Github(GitHub::connect(token).await?),
            Creds::Bitbucket { email, token } => Client::Bitbucket(Bitbucket::connect(email, token).await?),
        })
    }

    pub fn kind(&self) -> ProviderKind {
        match self {
            Client::Github(_) => ProviderKind::Github,
            Client::Bitbucket(_) => ProviderKind::Bitbucket,
        }
    }

    pub fn account(&self) -> &str {
        match self {
            Client::Github(c) => c.account(),
            Client::Bitbucket(c) => c.account(),
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        match self {
            Client::Github(c) => c.warnings(),
            Client::Bitbucket(_) => Vec::new(),
        }
    }

    pub async fn rate(&self) -> RateInfo {
        match self {
            Client::Github(c) => c.rate().await,
            Client::Bitbucket(c) => c.rate().await,
        }
    }

    pub async fn list_repos(&self) -> Result<Vec<Repo>> {
        match self {
            Client::Github(c) => c.list_repos().await,
            Client::Bitbucket(c) => c.list_repos().await,
        }
    }

    pub async fn list_runs(&self, repo: &str, etag: Option<&str>) -> Result<Fetched<Vec<Run>>> {
        match self {
            Client::Github(c) => c.list_runs(repo, etag).await,
            Client::Bitbucket(c) => c.list_runs(repo, etag).await,
        }
    }

    pub async fn rerun(&self, run: &Run, failed_only: bool) -> Result<()> {
        match self {
            Client::Github(c) => c.rerun(run, failed_only).await,
            Client::Bitbucket(c) => c.rerun(run).await,
        }
    }

    pub async fn cancel(&self, run: &Run) -> Result<()> {
        match self {
            Client::Github(c) => c.cancel(run).await,
            Client::Bitbucket(c) => c.cancel(run).await,
        }
    }

    pub async fn log_tail(&self, run: &Run, step: Option<&str>) -> Result<LogTail> {
        match self {
            Client::Github(c) => c.log_tail(run, step).await,
            Client::Bitbucket(c) => c.log_tail(run, step).await,
        }
    }
}

fn split_repo(full_name: &str) -> Result<(&str, &str)> {
    full_name.split_once('/').ok_or_else(|| Error::Other(format!("bad repo name: {full_name}")))
}

pub fn tail_log(raw: &str) -> String {
    let lines: Vec<String> = raw.lines().map(clean_line).collect();
    lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n")
}

fn clean_line(line: &str) -> String {
    let line = line.trim_start_matches('\u{feff}');
    // GitHub prefixes every line with an RFC 3339 timestamp.
    let line = match line.split_once(' ') {
        Some((ts, rest)) if ts.ends_with('Z') && chrono::DateTime::parse_from_rfc3339(ts).is_ok() => rest,
        _ => line,
    };
    strip_ansi(line)
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_github_log_lines() {
        let raw = "\u{feff}2026-11-09T01:02:03.4567890Z ##[group]Run tests\n2026-11-09T01:02:04.0000000Z \u{1b}[31merror\u{1b}[0m: boom\nplain line";
        assert_eq!(tail_log(raw), "##[group]Run tests\nerror: boom\nplain line");
    }

    #[test]
    fn keeps_only_the_tail() {
        let raw: String = (0..600).map(|i| format!("line {i}\n")).collect();
        let tail = tail_log(&raw);
        assert_eq!(tail.lines().count(), LOG_TAIL_LINES);
        assert!(tail.starts_with("line 100\n"));
    }
}
