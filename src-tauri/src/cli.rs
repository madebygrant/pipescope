use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;

use crate::checkouts;
use crate::ipc::{self, Failure, LogReply, Outcome, Reply, Request, WatchReply};
use crate::model::Run;

const USAGE: &str = "Usage: pipescope <command> [options]

Commands:
  failures             Failing runs from the last 48 hours, yours unless --all
      --all            Include everyone's runs
      --repo <owner/name>   One repo, everyone's runs
      --here           The current folder's repo, everyone's runs
  log [run]            A run's log, the last 500 lines. Without a run, the newest
                       failure in the current folder's repo, on this branch if any
      --step <name>    A job (GitHub) or step (Bitbucket) other than the failing one
  watch                Wait for CI on what you just pushed, and exit 0 if it passes,
                       1 if it fails, 3 if it's waiting for approval
      --repo <owner/name>   Defaults to the current folder's repo
      --branch <name>  Runs on this branch that start from now, instead of HEAD's commit
      --commit <sha>   Defaults to HEAD
      --timeout <time> Like 90s, 20m or 1h (default 30m)

  --json               Print JSON instead of text (all commands)

A run is its key from `failures --json`, its web address, or owner/repo#number.
Pipescope must be installed; it's opened in the background if it isn't running.";

const EXIT_FAILED: i32 = 1;
const EXIT_ERROR: i32 = 2;
const EXIT_WAITING: i32 = 3;

// Some for a command-line call, None for a normal launch of the app.
pub fn run_if_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Older macOS passes -psn_<id> when Finder launches an app.
    match args.first() {
        // LaunchServices (Finder, `open`, login items) starts the app directly under launchd. Run from
        // a shell, script or coding agent, plain `pipescope` explains itself rather than start a second copy.
        None if !cfg!(debug_assertions) && std::os::unix::process::parent_id() != 1 => {
            println!("{USAGE}");
            Some(0)
        }
        None => None,
        Some(a) if a.starts_with("-psn_") => None,
        Some(_) => Some(main(&args)),
    }
}

fn main(args: &[String]) -> i32 {
    match dispatch(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("pipescope: {message}");
            EXIT_ERROR
        }
    }
}

struct Opts {
    positional: Vec<String>,
    json: bool,
    all: bool,
    here: bool,
    repo: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
    step: Option<String>,
    timeout: Option<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        positional: Vec::new(),
        json: false,
        all: false,
        here: false,
        repo: None,
        branch: None,
        commit: None,
        step: None,
        timeout: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value."));
        match arg.as_str() {
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--here" => o.here = true,
            "--repo" => o.repo = Some(value("--repo")?),
            "--branch" => o.branch = Some(value("--branch")?),
            "--commit" => o.commit = Some(value("--commit")?),
            "--step" => o.step = Some(value("--step")?),
            "--timeout" => o.timeout = Some(value("--timeout")?),
            flag if flag.starts_with('-') => return Err(format!("Unknown option {flag}.\n\n{USAGE}")),
            _ => o.positional.push(arg.clone()),
        }
    }
    Ok(o)
}

fn dispatch(args: &[String]) -> Result<i32, String> {
    let (command, rest) = args.split_first().expect("run_if_cli passes at least one argument");
    match command.as_str() {
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            return Ok(0);
        }
        "--version" | "-V" => {
            println!("pipescope {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        _ => {}
    }
    let o = parse(rest)?;
    let here = std::env::current_dir().ok().and_then(|d| Local::find(&d));
    match command.as_str() {
        "failures" => {
            let repo = match (&o.repo, o.here) {
                (Some(repo), _) => Some(repo.clone()),
                (None, true) => Some(here.as_ref().and_then(|h| h.repo.clone()).ok_or(NOT_A_CLONE)?),
                (None, false) => None,
            };
            let failures: Vec<Failure> = request(&Request::Failures { all: o.all, repo })?;
            print_failures(&failures, o.json)
        }
        "log" => {
            let repo = o.repo.clone().or_else(|| here.as_ref().and_then(|h| h.repo.clone()));
            let branch = o.branch.clone().or_else(|| here.as_ref().and_then(|h| h.branch.clone()));
            let req = Request::Log { run: o.positional.first().cloned(), repo, branch, step: o.step.clone() };
            let reply: LogReply = request(&req)?;
            print_log(&reply, o.json)
        }
        "watch" => {
            let repo = o.repo.clone().or_else(|| here.as_ref().and_then(|h| h.repo.clone())).ok_or(NOT_A_CLONE)?;
            // With neither given, HEAD's commit on HEAD's branch. An explicit branch means "the next runs
            // there", and an explicit commit is watched on whatever branch ran it.
            let (commit, branch) = match (&o.commit, &o.branch) {
                (None, None) => (here.as_ref().and_then(|h| h.commit.clone()), here.as_ref().and_then(|h| h.branch.clone())),
                (commit, branch) => (commit.clone(), branch.clone()),
            };
            if commit.is_none() && branch.is_none() {
                return Err("Give --commit or --branch, or run this inside a clone.".into());
            }
            let timeout_secs = o.timeout.as_deref().map(parse_duration).transpose()?.unwrap_or(30 * 60);
            let reply: WatchReply = request(&Request::Watch { repo, branch, commit, timeout_secs })?;
            print_watch(&reply, o.json)
        }
        other => Err(format!("Unknown command {other}.\n\n{USAGE}")),
    }
}

const NOT_A_CLONE: &str = "This folder isn't a clone of a GitHub or Bitbucket repo. Use --repo owner/name.";

fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len()));
    let n: u64 = number.parse().map_err(|_| format!("Couldn't read the time {text}."))?;
    match unit {
        "" | "s" => Ok(n),
        "m" => Ok(n * 60),
        "h" => Ok(n * 3600),
        _ => Err(format!("Couldn't read the time {text}. Use something like 90s, 20m or 1h.")),
    }
}

struct Local {
    repo: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
}

impl Local {
    fn find(start: &Path) -> Option<Self> {
        let dir = start.ancestors().find(|d| d.join(".git").exists())?;
        let dirs = checkouts::git_dirs(dir)?;
        let config = std::fs::read_to_string(dirs.common.join("config")).unwrap_or_default();
        let head = std::fs::read_to_string(dirs.own.join("HEAD")).unwrap_or_default();
        let head = head.trim();
        let (branch, commit) = match head.strip_prefix("ref: ") {
            Some(reference) => (
                reference.strip_prefix("refs/heads/").map(str::to_string),
                resolve_ref(&dirs.common, reference),
            ),
            // Detached: HEAD holds the commit itself.
            None => (None, Some(head.to_string()).filter(|h| !h.is_empty())),
        };
        Some(Local { repo: checkouts::repo_of(&config).map(|(_, repo)| repo), branch, commit })
    }
}

// Loose ref files win over packed-refs, as in git.
fn resolve_ref(common: &Path, reference: &str) -> Option<String> {
    if let Ok(sha) = std::fs::read_to_string(common.join(reference)) {
        return Some(sha.trim().to_string());
    }
    let packed = std::fs::read_to_string(common.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (sha, name) = line.split_once(' ')?;
        (name == reference).then(|| sha.to_string())
    })
}

fn socket() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME isn't set.")?;
    let path = ipc::socket_path(Path::new(&home));
    if !ipc::socket_path_fits(&path) {
        return Err(ipc::PATH_TOO_LONG.into());
    }
    Ok(path)
}

// Opens the app in the background if it isn't running, then waits for it to listen.
fn connect() -> Result<UnixStream, String> {
    let path = socket()?;
    if let Ok(stream) = UnixStream::connect(&path) {
        return Ok(stream);
    }
    let opened = std::process::Command::new("open").args(["-g", "-b", crate::IDENTIFIER]).status();
    if !opened.is_ok_and(|s| s.success()) {
        return Err("Pipescope isn't running and couldn't be opened. Is it installed?".into());
    }
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(250));
        if let Ok(stream) = UnixStream::connect(&path) {
            return Ok(stream);
        }
    }
    Err("Pipescope was opened but didn't answer. Is it an older version without the command line?".into())
}

fn request<T: serde::de::DeserializeOwned>(req: &Request) -> Result<T, String> {
    let mut stream = connect()?;
    let mut line = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| e.to_string())?;
    for line in BufReader::new(stream).lines() {
        let line = line.map_err(|e| e.to_string())?;
        match serde_json::from_str::<Reply>(&line).map_err(|e| format!("Unexpected reply from Pipescope: {e}"))? {
            Reply::Progress { message } => eprintln!("{message}"),
            Reply::Done { data } => return serde_json::from_value(data).map_err(|e| format!("Unexpected reply from Pipescope: {e}")),
            Reply::Error { message } => return Err(message),
        }
    }
    Err("Pipescope closed the connection without answering.".into())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    println!("{}", serde_json::to_string_pretty(value).map_err(|e| e.to_string())?);
    Ok(())
}

fn ago(run: &Run) -> String {
    let mins = (Utc::now() - run.created_at).num_minutes().max(0);
    match mins {
        0 => "just now".into(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

fn headline(run: &Run) -> String {
    let branch = run.branch.as_deref().map(|b| format!(" · {b}")).unwrap_or_default();
    format!("{} · {} #{}{branch}", run.repo, run.title, run.number)
}

fn print_failures(failures: &[Failure], json: bool) -> Result<i32, String> {
    if json {
        print_json(&failures)?;
        return Ok(0);
    }
    if failures.is_empty() {
        println!("Nothing is failing.");
    }
    for f in failures {
        let flaky = if f.run.flaky_workflow { " · often flaky" } else { "" };
        println!("✗ {} · failed {}{flaky}", headline(&f.run), ago(&f.run));
        println!("  {}", f.run.web_url);
        if let Some(path) = &f.checkout {
            println!("  {path}");
        }
        println!("  pipescope log {}", f.run.key);
    }
    Ok(0)
}

fn print_log(reply: &LogReply, json: bool) -> Result<i32, String> {
    if json {
        print_json(reply)?;
        return Ok(0);
    }
    // The header goes to stderr so the log itself can be piped.
    let step = if reply.log.step.is_empty() { String::new() } else { format!(" · {}", reply.log.step) };
    eprintln!("{} · {}{step}", headline(&reply.run), reply.run.status.as_str());
    if reply.log.skipped > 0 {
        eprintln!("Last {} lines. The full log is at {}", reply.log.text.lines().count(), reply.log.web_url);
    }
    println!("{}", reply.log.text);
    Ok(0)
}

fn print_watch(reply: &WatchReply, json: bool) -> Result<i32, String> {
    let code = match reply.outcome {
        Outcome::Passed => 0,
        Outcome::Failed => EXIT_FAILED,
        Outcome::Waiting => EXIT_WAITING,
    };
    if json {
        print_json(reply)?;
        return Ok(code);
    }
    let verdict = match reply.outcome {
        Outcome::Passed => "Passed",
        Outcome::Failed => "Failed",
        Outcome::Waiting => "Waiting for approval or a manual step",
    };
    println!("{verdict}");
    for run in &reply.runs {
        println!("  {} #{} {} · {}", run.title, run.number, run.status.as_str(), run.web_url);
        if run.status == crate::model::RunStatus::Failed {
            println!("  pipescope log {}", run.key);
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("90s"), Ok(90));
        assert_eq!(parse_duration("20m"), Ok(1200));
        assert_eq!(parse_duration("1h"), Ok(3600));
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("5d").is_err());
    }

    #[test]
    fn parses_options() {
        let args: Vec<String> = ["abc", "--json", "--step", "test"].map(String::from).to_vec();
        let o = parse(&args).unwrap();
        assert_eq!((o.positional, o.json, o.step.as_deref()), (vec!["abc".to_string()], true, Some("test")));
        assert!(parse(&["--step".to_string()]).is_err());
        assert!(parse(&["--nope".to_string()]).is_err());
    }

    #[test]
    fn reads_branch_and_commit_from_a_clone() {
        let root = std::env::temp_dir().join(format!("pipescope-local-{}", std::process::id()));
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("config"), "[remote \"origin\"]\n url = git@github.com:acme/web.git\n").unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        std::fs::write(git.join("packed-refs"), "# pack-refs\nabc123 refs/heads/feature/x\n").unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();

        let local = Local::find(&root.join("src/deep")).unwrap();
        assert_eq!(local.repo.as_deref(), Some("acme/web"));
        assert_eq!(local.branch.as_deref(), Some("feature/x"));
        assert_eq!(local.commit.as_deref(), Some("abc123"));

        // A loose ref is newer than the packed one.
        std::fs::create_dir_all(git.join("refs/heads/feature")).unwrap();
        std::fs::write(git.join("refs/heads/feature/x"), "def456\n").unwrap();
        assert_eq!(Local::find(&root).unwrap().commit.as_deref(), Some("def456"));

        std::fs::write(git.join("HEAD"), "0123abcd\n").unwrap();
        let detached = Local::find(&root).unwrap();
        assert_eq!((detached.branch, detached.commit.as_deref()), (None, Some("0123abcd")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
