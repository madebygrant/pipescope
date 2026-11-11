use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::ProviderKind;

// Used when the setting is blank; folders that don't exist are skipped.
pub const DEFAULT_FOLDERS: &str = "~/Projects\n~/Developer\n~/Code\n~/src\n~/dev\n~/git";
// Deep enough for ~/Projects/client/group/repo.
const MAX_DEPTH: usize = 4;
const SKIP: [&str; 6] = ["node_modules", "vendor", "target", "dist", "build", "Library"];

pub fn folders(setting: &str, home: &Path) -> Vec<PathBuf> {
    let setting = if setting.trim().is_empty() { DEFAULT_FOLDERS } else { setting };
    setting
        .split([',', '\n'])
        .map(str::trim)
        .filter(|f| !f.is_empty())
        // A bundled app's working directory is /, so a relative folder is taken as being in home.
        .map(|f| match f.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if f == "~" => home.to_path_buf(),
            None => home.join(f),
        })
        .collect()
}

// Lowercased "provider:owner/repo" → checkout path. A clone whose origin is the repo
// wins over one that only has it as another remote, then the shorter path.
pub fn scan(roots: &[PathBuf]) -> HashMap<String, String> {
    let mut found: HashMap<String, (bool, String)> = HashMap::new();
    for root in roots {
        walk(root, 0, &mut |path, remotes| {
            for (name, url) in remotes {
                let Some((kind, repo)) = parse_remote(&url) else { continue };
                let candidate = (name == "origin", path.display().to_string());
                let better = |old: &(bool, String)| (candidate.0, std::cmp::Reverse(candidate.1.len())) > (old.0, std::cmp::Reverse(old.1.len()));
                let key = key(kind, &repo);
                if found.get(&key).is_none_or(better) {
                    found.insert(key, candidate);
                }
            }
        });
    }
    found.into_iter().map(|(k, (_, path))| (k, path)).collect()
}

fn key(kind: ProviderKind, repo: &str) -> String {
    format!("{}:{}", kind.as_str(), repo.to_lowercase())
}

fn walk(dir: &Path, depth: usize, found: &mut impl FnMut(&Path, Vec<(String, String)>)) {
    if let Some(config) = git_config(dir) {
        // A checkout's own folders are its code, not more checkouts.
        return found(dir, remotes(&config));
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
            continue;
        }
        // file_type doesn't follow symlinks, so a link loop can't recurse forever.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&entry.path(), depth + 1, found);
        }
    }
}

pub struct GitDirs {
    // HEAD lives here; for a worktree it's the worktree's own folder.
    pub own: PathBuf,
    // Config and refs live here.
    pub common: PathBuf,
}

// A worktree or submodule has a .git file pointing at the real git directory.
pub fn git_dirs(dir: &Path) -> Option<GitDirs> {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        return Some(GitDirs { own: dot_git.clone(), common: dot_git });
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let own = dir.join(pointer.strip_prefix("gitdir:")?.trim());
    let common = match std::fs::read_to_string(own.join("commondir")) {
        Ok(common) => own.join(common.trim()),
        Err(_) => own.clone(),
    };
    Some(GitDirs { own, common })
}

fn git_config(dir: &Path) -> Option<String> {
    std::fs::read_to_string(git_dirs(dir)?.common.join("config")).ok()
}

// The repo a clone belongs to, from its origin remote or else its first recognised one.
pub fn repo_of(config: &str) -> Option<(ProviderKind, String)> {
    let remotes = remotes(config);
    let origin = remotes.iter().filter(|(name, _)| name == "origin").find_map(|(_, url)| parse_remote(url));
    origin.or_else(|| remotes.iter().find_map(|(_, url)| parse_remote(url)))
}

fn remotes(config: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut remote: Option<String> = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            remote = line.strip_prefix("[remote \"").and_then(|r| r.strip_suffix("\"]")).map(str::to_string);
        } else if let Some(name) = &remote
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "url"
        {
            out.push((name.clone(), v.trim().to_string()));
        }
    }
    out
}

// Handles https, ssh:// and scp-style URLs, including ssh host aliases like github.com-work.
fn parse_remote(url: &str) -> Option<(ProviderKind, String)> {
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = host.rsplit('@').next()?.split(':').next()?.to_lowercase();
    // ssh config aliases look like "github.com-work" or "github-work"; a dotted host such as
    // github.mycompany.com is GitHub Enterprise, a different server.
    let on = |site: &str| {
        let name = site.split('.').next().unwrap_or(site);
        host == site || host.starts_with(&format!("{site}-")) || (!host.contains('.') && host.starts_with(name))
    };
    let kind = if on("github.com") {
        ProviderKind::Github
    } else if on("bitbucket.org") {
        ProviderKind::Bitbucket
    } else {
        return None;
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some((kind, format!("{owner}/{name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_urls() {
        let gh = |r: &str| Some((ProviderKind::Github, r.to_string()));
        assert_eq!(parse_remote("git@github.com:acme/web.git"), gh("acme/web"));
        assert_eq!(parse_remote("https://github.com/acme/web"), gh("acme/web"));
        assert_eq!(parse_remote("https://github.com/acme/web.git/"), gh("acme/web"));
        assert_eq!(parse_remote("ssh://git@github.com:22/acme/web.git"), gh("acme/web"));
        assert_eq!(parse_remote("git@github.com-work:acme/web.git"), gh("acme/web"));
        assert_eq!(
            parse_remote("https://grant@bitbucket.org/ws/app.git"),
            Some((ProviderKind::Bitbucket, "ws/app".into()))
        );
        assert_eq!(parse_remote("git@github-work:acme/web.git"), gh("acme/web"));
        assert_eq!(parse_remote("git@github.mycompany.com:acme/web.git"), None);
        assert_eq!(parse_remote("git@gitlab.com:acme/web.git"), None);
        assert_eq!(parse_remote("/local/path/repo.git"), None);
        assert_eq!(parse_remote("https://github.com/acme"), None);
    }

    #[test]
    fn reads_remotes_from_config() {
        let config = "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:me/web.git\n\tfetch = +refs/heads/*\n[remote \"upstream\"]\n\turl = https://github.com/acme/web\n[branch \"main\"]\n\tremote = origin\n";
        assert_eq!(
            remotes(config),
            vec![
                ("origin".into(), "git@github.com:me/web.git".into()),
                ("upstream".into(), "https://github.com/acme/web".into())
            ]
        );
    }

    #[test]
    fn expands_folders() {
        let home = Path::new("/Users/me");
        assert_eq!(
            folders("~/Projects, /opt/code\nwork\n", home),
            vec![home.join("Projects"), PathBuf::from("/opt/code"), home.join("work")]
        );
        assert_eq!(folders("  ", home).len(), DEFAULT_FOLDERS.lines().count());
    }

    #[test]
    fn scans_checkouts_preferring_origin() {
        let root = std::env::temp_dir().join(format!("pipescope-scan-{}", std::process::id()));
        let write = |dir: &str, config: &str| {
            let git = root.join(dir).join(".git");
            std::fs::create_dir_all(&git).unwrap();
            std::fs::write(git.join("config"), config).unwrap();
        };
        write("fork", "[remote \"origin\"]\n url = git@github.com:me/web.git\n[remote \"upstream\"]\n url = git@github.com:acme/web.git\n");
        write("client/longer-name/web", "[remote \"origin\"]\n url = git@github.com:Acme/Web.git\n");
        write("client/node_modules/dep", "[remote \"origin\"]\n url = git@github.com:x/dep.git\n");
        std::fs::create_dir_all(root.join("fork/nested/.git")).unwrap();

        let found = scan(std::slice::from_ref(&root));
        assert_eq!(found["github:acme/web"], root.join("client/longer-name/web").display().to_string());
        assert_eq!(found["github:me/web"], root.join("fork").display().to_string());
        assert!(!found.contains_key("github:x/dep"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
