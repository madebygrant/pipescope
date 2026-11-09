# Pipescope

A macOS menu bar app for GitHub Actions and Bitbucket Pipelines. It shows which runs are failing or running across every repo you can access. It notifies you when your runs fail, and you can re-run, cancel or read logs without opening a browser.

## Get started

1. Build the app (see [Development](docs/development.md)) and move `Pipescope.app` to `/Applications`.
2. Open it. A welcome screen asks you to connect GitHub, Bitbucket or both.
3. Press `⌥⌘P` any time to open the popover.

GitHub picks up your `gh` CLI login if you have one. Bitbucket needs your Atlassian email and an API token. [Setup](docs/setup.md) lists the scopes each token needs.

## Docs

- [Setup](docs/setup.md): accounts, tokens, launch at login and where data is kept
- [Using Pipescope](docs/usage.md): the list, logs, notifications and keyboard shortcuts
- [How it works](docs/how-it-works.md): polling, rate limits and what each provider can do
- [Development](docs/development.md): running, testing, building and the app icons
- [Roadmap](docs/roadmap.md): what's planned, and the plan to connect with coding agents
