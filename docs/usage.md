# Using Pipescope

## Menu bar

The dot beside the menu bar mark is red when one of your runs is failing, amber while something is running, green when everything passes and grey before any runs have loaded. The number next to it counts the failing runs. Click to open the popover. Right-click for a menu with the failing runs, Refresh, Settings and Quit.

## The list

The popover shows the last 7 days, in three parts:

- **Needs attention.** The newest run of each workflow and branch, if it failed in the last 48 hours. Once a newer run passes, it drops off.
- **Running.** Queued and in-progress runs, with a live timer.
- **Repos.** Everything else, grouped by repo, four runs each. Pinned repos come first.

"Mine" shows only runs you triggered. "All" shows everyone's. The search box filters by repo, branch, workflow, author or commit message. Click the failing or running count to see only those runs.

Hover a repo heading to pin it or mute it. A muted repo is hidden, ignored by the menu bar count, and never notifies. You can unmute it in Settings.

## Logs

Press `⌘L` on a run to read its log. Pipescope picks the failing job or step and opens at the first error. Use the dropdown to switch steps.

- Setup blocks start collapsed, except ones that contain an error.
- Errors are red, warnings amber and commands blue.
- "Errors only" (`⌘E`) shows each error with two lines either side.
- `⌘F` searches the log, and Enter or `⇧Enter` moves between matches.
- The expand button (`⇧⌘E`) opens the log in its own window that you can resize. `⌘W` closes it.

Only the last 500 lines appear in Pipescope. The note at the top links to the full log on the web. While a run is going, the log refreshes on its own. GitHub only provides a job's log once the job has finished, so GitHub logs appear at the end.

## Notifications

By default you're notified when one of your runs fails, and again when that workflow passes on the next run. Settings has an option to include everyone's runs. Failures that arrive together come as one notification. Clicking a notification opens the popover on that run.

## Keyboard shortcuts

| Keys | Where | Does |
| --- | --- | --- |
| `⌥⌘P` | Anywhere | Open or close the popover |
| `↑` `↓` | List | Select a run |
| `↵` | List | Open the run in the browser |
| `⌘L` | List | Show the log |
| `⌘R` | List | Re-run (failed jobs only on GitHub) |
| `⇧⌘R` | List | Re-run all jobs |
| `⌘.` | List | Cancel, after you confirm |
| `⌘,` | List | Settings |
| `esc` | Anywhere | Go back, then close |
| `⌘G` `⇧⌘G` | Log | Next or previous error or match |
| `⌘F` | Log | Find |
| `⌘E` | Log | Errors only |
| `⇧⌘E` | Log | Open in a larger window |
