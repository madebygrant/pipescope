# How it works

## Polling

Neither GitHub nor Bitbucket can push events to a desktop app without a public server, so Pipescope polls. How often depends on the repo:

| Repo | GitHub | Bitbucket |
| --- | --- | --- |
| Has a queued or running run | every 10s | every 20s |
| Active in the last 24 hours | every 60s | every 60s |
| Quiet | every 15 min | every 15 min |

The repo list itself refreshes every 30 minutes. A repo pushed to since the last check gets polled straight away.

The first poll of a repo only records its history, so you aren't flooded with notifications for old failures.

## Rate limits

GitHub allows 5,000 requests an hour. Pipescope sends ETags, and GitHub doesn't count the "not modified" replies against the limit. Bitbucket allows about 1,000 an hour.

Pipescope reads the rate limit headers on each response and spaces requests out so the budget lasts until it resets. When fewer than 10% of requests are left, a banner appears and polling slows down. If either service says to back off, Pipescope waits as long as it's told.

## Provider differences

| | GitHub | Bitbucket |
| --- | --- | --- |
| Re-run | Re-runs the same run. Can re-run only the failed jobs | Bitbucket has no re-run, so Pipescope starts a new pipeline on the same commit |
| Cancel | Cancels the run | Stops the pipeline |
| Logs | Per job, once the job finishes | Per step, while it runs |
| "Mine" | You triggered the run | You created the pipeline |

## Code layout

The Rust backend is in `src-tauri/src`:

- `providers/` talks to the GitHub and Bitbucket APIs.
- `http.rs` paces requests against the rate limits.
- `scheduler.rs` decides which repo to poll next.
- `store.rs` keeps runs and settings in SQLite.
- `notify.rs` spots failures and recoveries and batches the notifications.
- `tray.rs` draws the menu bar mark and its menu.
- `commands.rs` holds the calls the UI makes.

The React UI is in `src`. `App.tsx` is the list, `LogView.tsx` the log view and `SettingsView.tsx` the settings and welcome screen. `model.ts` holds the logic that doesn't depend on React, such as grouping runs and parsing logs.
