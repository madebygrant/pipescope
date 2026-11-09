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
