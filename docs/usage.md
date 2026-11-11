# Using Pipescope

## Menu bar

The dot beside the menu bar mark is red when one of your runs is failing, amber while something is running, green when everything passes and grey before any runs have loaded. The number next to it counts the failing runs. Click to open the popover. Right-click for a menu with the failing runs, Refresh, Settings and Quit.

## The list

The popover shows the last 7 days, in three parts:

- **Needs attention.** The newest run of each workflow and branch, if it failed in the last 48 hours. Once a newer run passes, it drops off.
- **Running.** Queued and in-progress runs. A running run shows a progress bar and a time like "2:04 / ~3:00", estimated from the median of that workflow's last 10 passes. Without past passes it shows the elapsed time only. A queued run shows how long it has waited.
- **Repos.** Everything else, grouped by repo, four runs each. Pinned repos come first.

"Mine" shows only runs you triggered. "All" shows everyone's. The filter box matches repo, branch, workflow, author or commit message. Click the failing or running count to see only those runs, and click it again to see everything. The refresh button beside the "updated" time polls now.

### Run actions

Each run has buttons on the right for its main action and its log:

- A finished run can be re-run. On GitHub, a failed run re-runs only its failed jobs.
- A running run can be watched, so you're notified when it finishes.
- A GitHub deployment waiting on you can be approved or rejected. A Bitbucket manual step opens in the browser.

Right-click a run for everything else: Open in Browser, Show Log, Re-run All Jobs, Notify When Finished, Cancel Run, Copy Failure for Agent, Copy Link, Show Checkout in Finder and the actions above. Cancel, approve and reject ask you to confirm on the run's status line first.

Clicking a run opens it in the browser.

### Repos

Hover a repo heading to set its client, pin it or mute it. A muted repo is hidden, ignored by the menu bar count, and never notifies. You can unmute it in Settings.

To stop following a repo altogether, add it to Settings › Ignored repos. Put one pattern per line or separate them with commas. `acme/legacy-*` matches by name, and an owner on its own, like `acme`, matches all of that owner's repos. Case doesn't matter. Ignored repos are never fetched, so they use no API calls, and their stored runs are removed.

### Clients

The grid button in the header opens Clients. It groups repos by client and totals each client's build time for this month or last month. Click a repo's name to set or change its client. The box suggests existing client names as you type: `↑` `↓` pick one, `↵` or Tab sets it, and esc closes the list. Leave it blank to clear the client.

## Logs

Press `⌘L` on a run, or click its log button, to read its log. Pipescope picks the failing job or step and opens at the first error. Use the Step dropdown to switch steps.

The top of the log shows the result, the number of errors and how many lines are new since the last passing run on that workflow. For a failed run Pipescope fetches that passing run's log on its own.

- Click the error count's arrows, or press `⌘G` and `⇧⌘G`, to move between errors.
- Click the new lines count (`⌘J`) to show only lines that weren't in the last passing run.
- For a failing Jest or Vitest test, a panel shows the expected and received values and the test's file and line. Click the location to jump to it in the log.
- Setup blocks start collapsed, except ones that contain an error.
- Errors are red, warnings amber and commands blue.
- "Errors" (`⌘E`) shows only each error with two lines either side.
- `⌘F` searches the log, and Enter or `⇧Enter` moves between matches.
- The wrap button wraps long lines, and the copy button copies the log, or only what's shown while filtered.
- The expand button (`⇧⌘E`) opens the log in its own window that you can resize. `⌘W` closes it.

Only the last 500 lines appear in Pipescope. The note at the top links to the full log on the web. While a run is going, the log refreshes on its own. GitHub only provides a job's log once the job has finished, so GitHub logs appear at the end.

## Copying a failure for an agent

On a failed run, press `⇧⌘C`, choose Copy Failure for Agent from the right-click menu, or click Copy for agent in its log. Pipescope copies a Markdown summary to paste into Claude Code, Codex, Cursor or any other coding agent. It holds:

- The run's link, branch, commit, author and failing step, and any other steps that failed.
- The local checkout's path, if Pipescope found one.
- A warning when the workflow is known to be flaky, or when the last pass ran the same commit, so the code hasn't changed.
- For a Jest or Vitest failure, the test, its file and line, and the expected and received values.
- Each error line with two lines either side, up to 120 lines.
- The lines that weren't in the last passing run, up to 80.
- The files changed since the last pass, or in the run's commit when there's no earlier pass, up to 100.

From the log, the summary covers the step you're viewing. If part of it can't be fetched, the summary says why and carries on. Listing changed files costs one API call.

## Local checkouts

Pipescope looks for git clones in `~/Projects`, `~/Developer`, `~/Code`, `~/src`, `~/dev` and `~/git`, up to four folders deep, and matches each clone to a repo by its remotes. Change the folders in Settings › Local checkouts, one per line or separated by commas. A folder without `~` or `/` counts as being in your home folder. Settings shows how many repos were found.

When a repo has more than one clone, the one whose `origin` points at it wins, then the one with the shorter path. Folders like `node_modules` and anything inside a clone are skipped. New clones show up within 30 seconds.

A copied failure includes the checkout's path, and right-clicking a run offers Show Checkout in Finder.

## Notifications

By default you're notified when one of your runs fails, and again when that workflow passes on the next run. Settings has an option to include everyone's runs. Failures that arrive together come as one notification. Clicking a notification opens the popover on that run.

To hear when a particular run finishes, watch it with its eye button, `⌘D` or Notify When Finished in the right-click menu.

## Appearance

Settings › Appearance follows macOS by default, or sets light or dark. The log window follows the same setting.

## Keyboard shortcuts

| Keys | Where | Does |
| --- | --- | --- |
| `⌥⌘P` | Anywhere | Open or close the popover |
| `↑` `↓` | List | Select a run |
| `↵` | List | Open the run in the browser |
| `⌘L` | List | Show the log |
| `⌘R` | List | Re-run (failed jobs only on GitHub) |
| `⇧⌘R` | List | Re-run all jobs |
| `⌘D` | List | Watch a running run, or stop watching |
| `⌘.` | List | Cancel, after you confirm |
| `⇧⌘C` | List, Log | Copy a failed run for an agent |
| `⌘,` | List | Settings |
| `esc` | Anywhere | Go back, then close |
| `⌘G` `⇧⌘G` | Log | Next or previous error or match |
| `⌘F` | Log | Find |
| `⌘E` | Log | Errors only |
| `⌘J` | Log | New lines only |
| `⇧⌘E` | Log | Open in a larger window |
