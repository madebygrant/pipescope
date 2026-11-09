use std::process::Command;

use keyring::Entry;

use crate::error::Result;

const SERVICE: &str = "au.com.hatchd.pipescope";

pub fn get_secret(account: &str) -> Result<Option<String>> {
    match Entry::new(SERVICE, account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set_secret(account: &str, secret: &str) -> Result<()> {
    Ok(Entry::new(SERVICE, account)?.set_password(secret)?)
}

pub fn delete_secret(account: &str) -> Result<()> {
    match Entry::new(SERVICE, account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// Apps launched from Finder get a minimal PATH, so try the Homebrew locations too.
pub fn gh_cli_token() -> Option<String> {
    ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"].into_iter().find_map(|bin| {
        let out = Command::new(bin).args(["auth", "token", "--hostname", "github.com"]).output().ok()?;
        let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !token.is_empty()).then_some(token)
    })
}

// Read the files rather than running git: on a Mac without the developer tools,
// /usr/bin/git pops up an install prompt.
pub fn git_emails() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return Vec::new() };
    [home.join(".gitconfig"), home.join(".config/git/config")]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|text| user_emails(&text))
        .collect()
}

fn user_emails(config: &str) -> Vec<String> {
    let mut in_user = false;
    let mut out = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_user = line.trim_start_matches('[').trim_end_matches(']').trim().eq_ignore_ascii_case("user");
        } else if in_user
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("email")
        {
            let value = value.split(['#', ';']).next().unwrap_or("").trim().trim_matches('"');
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::user_emails;

    #[test]
    fn reads_user_emails_from_git_config() {
        let config = "[core]\n\temail = not-me@x.com\n[user]\n\tname = Grant\n\temail = \"me@example.com\" # work\n[user \"x\"]\n";
        assert_eq!(user_emails(config), ["me@example.com"]);
    }
}
