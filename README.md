# Pipescope

A macOS menu bar app that watches the GitHub Actions and Bitbucket Pipelines runs you have access to. It shows what's failing and running, notifies you when your runs fail or recover, and lets you re-run, cancel and read logs without opening a browser.

Built with Tauri 2 (Rust) and React.

## Setup

GitHub uses your `gh` CLI login automatically, or a classic personal access token with the `repo` scope. Bitbucket Cloud needs your Atlassian email and a scoped API token with `read:user`, `read:workspace`, `read:repository`, `read:pipeline` and `write:pipeline`. Tokens are stored in the macOS Keychain.

## Develop

```sh
pnpm install
pnpm tauri dev          # run the app
pnpm dev                # UI only in a browser, backed by src/devMock.ts
cd src-tauri && cargo test
```

`pnpm tauri build` produces the `.app` bundle.

## Icons

`assets/logo.svg` is the app icon and `assets/tray-mark.svg` the menu bar mark. To regenerate the app icons:

```sh
rsvg-convert -w 1024 -h 1024 assets/logo.svg -o assets/logo-1024.png
pnpm tauri icon assets/logo-1024.png
```

The tray PNGs in `src-tauri/icons/tray/` are rendered from `tray-mark.svg` at 36px high; `LENS_CENTRE` in `src-tauri/src/tray.rs` must match the lens position.
