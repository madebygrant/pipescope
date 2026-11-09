use std::time::Duration;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("token rejected")]
    Unauthorized,
    #[error("rate limited, retrying in {}s", .0.as_secs())]
    RateLimited(Duration),
    #[error("{api} returned {code}: {message}")]
    Status { api: &'static str, code: u16, message: String },
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("bad JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("keychain error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("window error: {0}")]
    Window(#[from] tauri::Error),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn is_network(&self) -> bool {
        matches!(self, Error::Http(e) if e.is_connect() || e.is_timeout())
    }

    pub fn is_access_denied(&self) -> bool {
        matches!(self, Error::Status { code: 403 | 404 | 410, .. })
    }
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
