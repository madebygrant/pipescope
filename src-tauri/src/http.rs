use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, ETAG, HeaderMap, HeaderValue, IF_NONE_MATCH, LINK, RETRY_AFTER, USER_AGENT};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;
use url::Url;

use crate::error::{Error, Result};
use crate::model::RateInfo;

const FAST_GAP: Duration = Duration::from_millis(150);
const HOUR: Duration = Duration::from_secs(3600);
// Kept back from the server-reported budget so re-run, cancel and logs still work
// after polling has spent the rest.
const RESERVE: u32 = 100;

// Spaces requests so polling stays inside the provider's hourly budget. Uses the
// server's rate-limit headers when present, else counts requests against `assumed_hourly`.
pub struct Pacer {
    assumed_hourly: u32,
    remaining: Option<u32>,
    limit: Option<u32>,
    reset_at: Option<Instant>,
    blocked_until: Option<Instant>,
    last: Option<Instant>,
    window: VecDeque<Instant>,
}

impl Pacer {
    pub fn new(assumed_hourly: u32) -> Self {
        Self {
            assumed_hourly,
            remaining: None,
            limit: None,
            reset_at: None,
            blocked_until: None,
            last: None,
            window: VecDeque::new(),
        }
    }

    pub fn delay(&mut self, now: Instant) -> Duration {
        while self.window.front().is_some_and(|t| now.duration_since(*t) >= HOUR) {
            self.window.pop_front();
        }
        let mut wait = self
            .last
            .map(|last| (last + self.gap(now)).saturating_duration_since(now))
            .unwrap_or_default();
        if let Some(until) = self.blocked_until {
            wait = wait.max(until.saturating_duration_since(now));
        }
        if self.remaining.is_none()
            && self.window.len() as u32 >= self.assumed_hourly * 9 / 10
            && let Some(oldest) = self.window.front()
        {
            wait = wait.max((*oldest + HOUR).saturating_duration_since(now));
        }
        wait
    }

    fn gap(&self, now: Instant) -> Duration {
        match (self.remaining, self.limit, self.reset_at) {
            (Some(remaining), Some(limit), Some(reset)) => {
                if remaining > limit / 2 {
                    FAST_GAP
                } else {
                    let spendable = remaining.saturating_sub(RESERVE).max(1);
                    (reset.saturating_duration_since(now) / spendable).max(FAST_GAP)
                }
            }
            _ if (self.window.len() as u32) < self.assumed_hourly / 2 => FAST_GAP,
            _ => HOUR / self.assumed_hourly,
        }
    }

    pub fn budget(&self, now: Instant) -> RateInfo {
        match (self.remaining, self.limit) {
            (Some(remaining), Some(limit)) => RateInfo {
                remaining,
                limit,
                reset_secs: self.reset_at.map(|r| r.saturating_duration_since(now).as_secs()),
            },
            _ => {
                let used = self.window.iter().filter(|t| now.duration_since(**t) < HOUR).count() as u32;
                RateInfo { remaining: self.assumed_hourly.saturating_sub(used), limit: self.assumed_hourly, reset_secs: None }
            }
        }
    }

    pub fn record(&mut self, now: Instant) {
        self.last = Some(now);
        self.window.push_back(now);
    }

    pub fn observe(&mut self, headers: &HeaderMap, now: Instant) {
        let num = |name: &str| {
            headers.get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok())
        };
        if let (Some(remaining), Some(limit)) = (num("x-ratelimit-remaining"), num("x-ratelimit-limit")) {
            self.remaining = Some(remaining as u32);
            self.limit = Some(limit as u32);
            if let Some(reset) = num("x-ratelimit-reset") {
                let unix_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                self.reset_at = Some(now + Duration::from_secs(reset.saturating_sub(unix_now)));
            }
        }
    }
}

#[derive(Clone)]
pub enum Auth {
    Bearer(String),
    Basic { user: String, secret: String },
}

pub enum Fetched<T> {
    NotModified,
    Fresh { body: T, etag: Option<String>, next: Option<String> },
}

impl<T> Fetched<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Fetched<U> {
        match self {
            Fetched::NotModified => Fetched::NotModified,
            Fetched::Fresh { body, etag, next } => Fetched::Fresh { body: f(body), etag, next },
        }
    }
}

#[derive(Clone)]
pub struct Api {
    name: &'static str,
    client: reqwest::Client,
    base: Url,
    auth: Auth,
    pacer: Arc<Mutex<Pacer>>,
}

impl Api {
    pub fn new(name: &'static str, base: &str, auth: Auth, assumed_hourly: u32, mut headers: HeaderMap) -> Result<Self> {
        headers.insert(USER_AGENT, HeaderValue::from_static("pipescope"));
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()?;
        let base = Url::parse(base).map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { name, client, base, auth, pacer: Arc::new(Mutex::new(Pacer::new(assumed_hourly))) })
    }

    // Segments are percent-encoded, which matters for Bitbucket's `{uuid}` ids.
    pub fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut().expect("API base URL has a path").pop_if_empty().extend(segments);
        url
    }

    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        {
            // Held across the sleep on purpose: requests queue in order instead of racing for the budget.
            let mut pacer = self.pacer.lock().await;
            let wait = pacer.delay(Instant::now());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            pacer.record(Instant::now());
        }
        let req = match &self.auth {
            Auth::Bearer(token) => req.bearer_auth(token),
            Auth::Basic { user, secret } => req.basic_auth(user, Some(secret)),
        };
        let resp = req.send().await?;
        let status = resp.status();
        {
            let now = Instant::now();
            let mut pacer = self.pacer.lock().await;
            pacer.observe(resp.headers(), now);
            // GitHub signals both primary and secondary limits with 403 rather than 429.
            let limited = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::FORBIDDEN
                    && (resp.headers().contains_key(RETRY_AFTER) || pacer.remaining == Some(0)));
            if limited {
                let wait = retry_after(resp.headers())
                    .or_else(|| pacer.reset_at.map(|r| r.saturating_duration_since(now)))
                    .unwrap_or(Duration::from_secs(60))
                    .max(Duration::from_secs(1));
                pacer.blocked_until = Some(now + wait);
                return Err(Error::RateLimited(wait));
            }
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized);
        }
        if status.is_success() || status == StatusCode::NOT_MODIFIED {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(Error::Status { api: self.name, code: status.as_u16(), message: error_message(&body) })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: Url, etag: Option<&str>) -> Result<Fetched<T>> {
        let mut req = self.client.get(url);
        if let Some(etag) = etag {
            req = req.header(IF_NONE_MATCH, etag);
        }
        let resp = self.send(req).await?;
        if resp.status() == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        let header = |name| resp.headers().get(name).and_then(|v: &HeaderValue| v.to_str().ok()).map(str::to_owned);
        let etag = header(ETAG);
        let next = header(LINK).as_deref().and_then(next_link);
        Ok(Fetched::Fresh { body: resp.json().await?, etag, next })
    }

    pub async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        match self.get_json(url, None).await? {
            Fetched::Fresh { body, .. } => Ok(body),
            Fetched::NotModified => Err(Error::Other("unexpected 304".into())),
        }
    }

    pub async fn get_with_headers<T: DeserializeOwned>(&self, url: Url) -> Result<(T, HeaderMap)> {
        let resp = self.send(self.client.get(url)).await?;
        let headers = resp.headers().clone();
        Ok((resp.json().await?, headers))
    }

    pub async fn rate(&self) -> RateInfo {
        self.pacer.lock().await.budget(Instant::now())
    }

    // Logs are plain text; Bitbucket answers 406 to the JSON Accept header the client sends by default.
    pub async fn get_text(&self, url: Url) -> Result<String> {
        Ok(self.send(self.client.get(url).header(ACCEPT, "*/*")).await?.text().await?)
    }

    pub async fn post(&self, url: Url, body: Option<&Value>) -> Result<()> {
        let mut req = self.client.post(url);
        if let Some(body) = body {
            req = req.json(body);
        }
        self.send(req).await.map(drop)
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs = headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

pub fn next_link(header: &str) -> Option<String> {
    let part = header.split(',').find(|p| p.contains("rel=\"next\""))?;
    let start = part.find('<')? + 1;
    let end = part.find('>')?;
    Some(part[start..end].to_string())
}

// GitHub errors are {"message"}, Bitbucket's are {"error":{"message"}}.
fn error_message(body: &str) -> String {
    let json: Option<Value> = serde_json::from_str(body).ok();
    let message = json.as_ref().and_then(|j| {
        j.get("message").or_else(|| j.get("error").and_then(|e| e.get("message"))).and_then(Value::as_str)
    });
    match message {
        Some(m) => m.to_string(),
        None => body.chars().take(200).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_next_link() {
        let h = r#"<https://api.github.com/user/repos?page=2>; rel="next", <https://api.github.com/user/repos?page=5>; rel="last""#;
        assert_eq!(next_link(h).as_deref(), Some("https://api.github.com/user/repos?page=2"));
        assert_eq!(next_link(r#"<https://x/?page=1>; rel="prev""#), None);
    }

    #[test]
    fn pacer_is_fast_with_plenty_of_budget() {
        let now = Instant::now();
        let mut p = Pacer::new(1000);
        p.record(now);
        assert_eq!(p.delay(now), FAST_GAP);
    }

    #[test]
    fn pacer_spreads_remaining_budget_until_reset() {
        let now = Instant::now();
        let mut p = Pacer::new(5000);
        p.remaining = Some(1100);
        p.limit = Some(5000);
        p.reset_at = Some(now + Duration::from_secs(1000));
        p.record(now);
        // 1000 spendable requests over 1000s.
        assert_eq!(p.delay(now), Duration::from_secs(1));
    }

    #[test]
    fn pacer_waits_for_window_when_assumed_budget_is_spent() {
        let start = Instant::now();
        let mut p = Pacer::new(100);
        for _ in 0..90 {
            p.record(start);
        }
        let later = start + Duration::from_secs(600);
        assert_eq!(p.delay(later), Duration::from_secs(3000));
    }

    #[test]
    fn pacer_honours_block() {
        let now = Instant::now();
        let mut p = Pacer::new(1000);
        p.blocked_until = Some(now + Duration::from_secs(30));
        assert_eq!(p.delay(now), Duration::from_secs(30));
    }

    #[test]
    fn budget_falls_back_to_counted_requests() {
        let now = Instant::now();
        let mut p = Pacer::new(1000);
        for _ in 0..250 {
            p.record(now);
        }
        assert_eq!(p.budget(now), RateInfo { remaining: 750, limit: 1000, reset_secs: None });
    }

    #[test]
    fn extracts_error_messages() {
        assert_eq!(error_message(r#"{"message":"Bad credentials"}"#), "Bad credentials");
        assert_eq!(error_message(r#"{"type":"error","error":{"message":"Not found"}}"#), "Not found");
        assert_eq!(error_message("oops"), "oops");
    }
}
