# Setup

## GitHub

If you're logged in to the `gh` CLI, Pipescope uses that token and there's nothing to do.

Otherwise, create a classic personal access token with the `repo` scope and paste it into Settings. Fine-grained tokens only cover one owner, so they miss repos in other orgs. If your org uses SSO, authorise the token for that org on GitHub.

A saved token takes priority over the `gh` login. "Use gh CLI instead" in Settings deletes the saved token.

## Bitbucket Cloud

Create a scoped API token at [id.atlassian.com](https://id.atlassian.com/manage-profile/security/api-tokens) with these scopes:

- `read:user`
- `read:workspace`
- `read:repository`
- `read:pipeline`
- `write:pipeline` (for re-run and cancel)

Enter your Atlassian account email and the token in Settings. Pipescope finds every workspace you belong to and watches their repos. If a workspace refuses access, Pipescope skips it and carries on with the rest.

## Launch at login

Tick "Open Pipescope when you log in" in Settings, or leave it ticked on the welcome screen. It adds a launch agent in `~/Library/LaunchAgents`, and Pipescope then shows up under System Settings › General › Login Items.

The setting stores the app's full path. If you move the app, untick and tick it again. It's greyed out in development builds.

## Where data is kept

- Tokens go in the macOS Keychain under the service `au.com.hatchd.pipescope`.
- Runs and settings go in `~/Library/Application Support/au.com.hatchd.pipescope/pipescope.db`. Deleting this file resets Pipescope. Your tokens stay in the Keychain.
