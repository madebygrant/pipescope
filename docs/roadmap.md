# Roadmap

Pipescope's job is to be the fastest way to find out what broke. Later it will hand that same information to coding agents. No built-in AI for now. Each feature should produce structured data, not just a screen, so the agent integration in phase 3 is mostly a wrapper around phases 1 and 2.

## Phase 1: worth paying for without AI

1. **Compare with the last passing run.** Show only the log lines that are new compared with the last green run of the same workflow.
2. **Flaky test detection.** Mark tests that failed and then passed on the same commit. Offer a one-key re-run, and optionally re-run them automatically.
3. **Watch a run.** Pin any run, including someone else's, and get one notification when it finishes.
4. **Approvals and manual steps.** Approve GitHub deployments that are waiting on an environment, and start Bitbucket manual steps, from the menu bar.
5. **Notification rules.** Notify only for chosen branches, set quiet hours, and flag a run as "broken by my commit" even when a merge or bot triggered it.
6. **Client groups and build minutes.** Group repos by client across services with one health status per client. Show monthly build minutes per repo or client, for billing and spotting waste.

Items 1 and 2 matter most, because their output is what an agent needs to fix a failure.

## Phase 2: getting data out

7. **Link repos to local checkouts.** Find local clones by matching their git remotes, for example everything under `~/Projects`, so each run knows which folder it belongs to.
8. **"Copy failure for agent".** One key copies a Markdown failure summary: the error lines, the comparison with the last passing run, any flaky tests, the commit, the changed files and the local path. You can paste it into any agent tool, with no integration needed.
9. **A `pipescope` CLI.** Commands such as `pipescope failures --json`, `pipescope log <run>` and `pipescope watch --branch <name>`, run against the app that's already running. Scripts and agent hooks can call it, for example a hook that starts watching after every `git push`.

## Phase 3: connecting to agent tools

10. **An MCP server.** Claude Code, Codex CLI, Gemini CLI, Cursor, Copilot in VS Code and Windsurf all support MCP, so one server covers them all. Tools:
    - `list_failures`
    - `get_failure_context` (the same summary as item 8)
    - `get_log`
    - `wait_for_run`, which an agent calls after pushing. It blocks until CI finishes and returns the result.
    - `rerun` and `cancel`
11. **Closing the loop.** An agent pushes a fix, waits on `wait_for_run`, reads the failure if CI is still red, and tries again.
12. **Agent runs count as mine.** Copilot's coding agent, Codex cloud and Claude Code on the web open pull requests as bot accounts. Treat the bots linked to you as you, so their failures notify you and appear under "Mine".

**Safety.** Re-run, cancel and approve change real infrastructure. The MCP server is read-only by default. Write tools are turned on per tool, with a confirmation in the menu bar the first time an agent uses them.

## Why this stands out

GitHub publishes its own MCP server with Actions tools. It only covers GitHub, and it returns raw logs. Pipescope's version works across services, uses the logins you've already set up, and returns a ready-made summary: just the errors, the comparison with the last passing run, and flaky tests already marked. Agents spend fewer tokens and are less likely to try to fix a flaky test.

## Possible providers

- **Vercel.** About a day. Deployments map onto runs, and the build log arrives as a list of events.
- **GitLab CI.** About a day.
- **AWS CodePipeline and CodeBuild.** Two to four days. Sign-in needs AWS profiles and SSO, every region has to be polled separately, and "mine" can't be worked out reliably. Only worth doing if builds actually run there.

## Not planned

- **Full CI dashboards and analytics.** Datadog CI Visibility and GitHub Insights already do this, with server-side data a menu bar app can't get.
