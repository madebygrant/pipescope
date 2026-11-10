# Development

You need Rust, Node and pnpm. Install the dependencies with:

```sh
pnpm install
```

## Install from source

`scripts/install.sh` downloads the latest release's source, builds it and installs it to `/Applications`, replacing any running copy:

```sh
curl -fsSL https://raw.githubusercontent.com/madebygrant/pipescope/main/scripts/install.sh | bash
```

To read it before running it, download it with `curl -fsSLO` and open `install.sh`.

Set `PIPESCOPE_REF=main` (or any branch or tag) to build something other than the latest release. If the Xcode command line tools, Rust, Node 20.19 or later, or pnpm are missing, the script lists them and offers to install each one. It installs nothing you haven't said yes to, and with no terminal to ask in it stops after the list.

Run it again to update. Rust's build output stays in `~/Library/Caches/pipescope`, so later builds take a few minutes. Delete that folder to get the space back.

## Run

```sh
pnpm tauri dev
```

This runs the real app with hot reload for the UI. Rust changes rebuild and restart it.

To work on the UI alone, run `pnpm dev` and open http://localhost:1420. Without Tauri, `src/devMock.ts` supplies fake runs, statuses and a sample log. The mock is only included in dev builds.

## Test

```sh
pnpm test                         # UI logic (Vitest)
cd src-tauri && cargo test        # backend
cd src-tauri && cargo clippy --all-targets
```

## Build

```sh
pnpm tauri build --bundles app
```

The app lands in `src-tauri/target/release/bundle/macos/Pipescope.app`. A clean release build takes about 12 minutes.

The build isn't signed or notarised. That's fine on the machine that built it. If you copy it to another Mac, macOS will block it until you right-click it and choose Open.

## CI

GitHub Actions runs two workflows on a macOS runner.

- **Checks** (`.github/workflows/checks.yml`) runs on every push to `main` and every pull request. It builds the UI, runs Vitest, clippy with warnings as errors, and the Rust tests.
- **Build** (`.github/workflows/build.yml`) makes a universal `.dmg` that runs on Apple Silicon and Intel. Start it by hand from the Actions tab, and the `.dmg` appears as an artifact on the run. Pushing a tag like `v0.2.0` also attaches the build to a draft GitHub release.

To release, run this from a clean, up-to-date `main`:

```sh
pnpm release patch    # or minor, major, or an exact version like 0.3.0
```

It sets the version in `package.json`, `tauri.conf.json`, `Cargo.toml` and `Cargo.lock`, commits, tags `vX.Y.Z` and pushes. Build then makes the draft release, which you publish from GitHub. These builds aren't signed either.

## Icons

`assets/logo.svg` is the app icon and `assets/tray-mark.svg` the menu bar mark. To regenerate the app icons:

```sh
rsvg-convert -w 1024 -h 1024 assets/logo.svg -o assets/logo-1024.png
pnpm tauri icon assets/logo-1024.png
```

The menu bar PNGs in `src-tauri/icons/tray/` are rendered from `tray-mark.svg` at 36px high. If you move the lens in the artwork, update `LENS_CENTRE` in `src-tauri/src/tray.rs` so the status dot still lands inside it.

If Finder or the Dock shows an old icon after a rebuild, refresh macOS's icon cache:

```sh
touch /Applications/Pipescope.app
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Pipescope.app
killall Dock
```
