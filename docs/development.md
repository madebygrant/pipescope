# Development

You need Rust, Node and pnpm. Install the dependencies with:

```sh
pnpm install
```

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
