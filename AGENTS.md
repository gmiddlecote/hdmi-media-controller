# AGENTS.md

Guidance for AI coding agents working on the HDMI Media Controller.

## Project

Windows desktop app (Rust + Tauri 2) that plays local images/videos
fullscreen on an external HDMI display while a control window stays on the
laptop screen. Static HTML/JS control UI — **no npm, no bundler**
(`withGlobalTauri: true` exposes `window.__TAURI__`).

Repo: https://github.com/gmiddlecote/hdmi-media-controller (private; remote
uses SSH `git@github.com:...`). Version `v0.1.0` tagged and released
(MSI/NSIS published via GitHub Actions).

## Build / verify (always run after changes)

```sh
cd src-tauri
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
node --check ../ui/app.js
```

`cargo build` also validates `tauri.conf.json` (parsed at compile time).
All checks must be clean before committing.

## Environment constraints

- Host: Arch Linux / Hyprland (Wayland). App only fully works on Windows.
- **No mingw-w64, no npm, very slow network** — do not add cargo deps or
  npm packages that aren't already in the offline cache.
- Windows correctness (display module) is cross-checked with a throwaway
  crate at `/tmp/display-check` (`cargo check/clippy --target
  x86_64-pc-windows-gnu`). Re-sync `src-tauri/src/display/*.rs` there when
  the display module changes.
- Visual smoke test: run `./target/debug/hdmi-media-controller` in bg,
  confirm window via `hyprctl clients`, log free of errors. Screenshots are
  not available (grim fails on this GPU), so UI is verified by code review
  + `node --check`.
- `gh` is authenticated (device flow) — use it for Actions/releases.

## Architecture

- `ui/` — `index.html` (control UI), `app.js` (IPC + events), `styles.css`,
  `render.html` (page for fullscreen output windows).
- `src-tauri/src/`:
  - `display/` — Win32 display list/connector (`QueryDisplayConfig`),
    resolutions (`EnumDisplaySettingsEx`), bounds, hotplug polling watcher
    (`displays-changed` event). `platform_other.rs` stubs non-Windows.
  - `media/` — `MediaItem`, kind/mime detection, `media://` registry
    (allow-list, dedupes by path, entries kept for the whole session),
    `serve()` protocol handler w/ byte-range support.
  - `playlist/` — ordered queue; `advance()` honors `RepeatMode`.
  - `scheduler/` — worker-thread kernel (50 ms tick) driving playback;
    `Command` channel, `PlayMode` = once/loop/timed, `Snapshot` events,
    `render-finished`/`output-control` events.
  - `renderer/` — borderless fullscreen windows; **must only be created off
    the main/command thread on Windows (deadlock)**.
  - `lib.rs` — all Tauri commands + managers + protocol registration.
- `.github/workflows/` — `ci.yml` (fmt/clippy/test + Windows MSI/NSIS build
  on push/PR), `release.yml` (tag `v*` → publish installers to a Release).

## Conventions

- Don't add code comments unless asked; keep existing module-doc style.
- Commands: `list_displays`, `list_directory` (`directory` → browsable
  entries), `get_display_modes`, `set_display_mode`, `scheduler_set_playlist`,
  `scheduler_play`, `scheduler_play_item` (`mode` = `once`/`loop`/`timed`,
  `seconds` for timed), `media_register`, `scheduler_pause`/`resume`/`next`/
  `prev`/`stop`/`set_dwell`/`set_overlay` (`text`)/`status`,
  `renderer_render_token`, `renderer_close`, `renderer_close_all`.
- JS invoke args are camelCase (`deviceName`, `paths`, `millis`, `directory`);
  listen to `displays-changed`, `scheduler-state`, `tauri://drag-drop`;
  backend-emitted to render windows: `output-control`, `overlay-text`;
  render windows emit `render-finished`, `media-progress`.
- Release: `git tag vX.Y.Z && git push origin vX.Y.Z`.