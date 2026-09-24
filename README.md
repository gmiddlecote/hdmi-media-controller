# HDMI Media Controller

A Windows desktop application (Rust + [Tauri 2](https://tauri.app)) for
controlling media playback on external HDMI-connected displays: pick an
output display, pick its resolution, and show local images/videos
fullscreen on it while the control UI stays on the laptop display.

## Current status

- Tauri 2 project skeleton (Rust backend, static HTML/CSS/JS control UI).
- Native Windows display enumeration via Win32 `EnumDisplayDevices`.
- Connector-type detection (HDMI, DisplayPort, DVI, VGA, ...) via
  `QueryDisplayConfig`.
- Resolution enumeration (current + all supported modes) via
  `EnumDisplaySettingsEx`, and mode selection via `ChangeDisplaySettingsEx`
  (test first, then apply and persist).
- Display hotplug detection: a polling watcher emits a `displays-changed`
  Tauri event when the connected set changes, and the UI re-enumerates
  automatically.
- Fullscreen, borderless **renderer windows** positioned on the chosen
  display (`renderer` module).
- **Local media loading**: image/video files added by drag-and-drop or by
  path, served to renderers over a custom `media://` scheme with byte-range
  support (`media` module).
- **Playlists and scheduled playback**: a background scheduler kernel
  advances the playlist, dwells on images, pauses/resumes on demand, and
  drives render windows (`playlist` + `scheduler` modules).

## Architecture

```
.
├── ui/                         Static control UI (no npm/bundler needed)
│   ├── index.html             Control + playback UI
│   ├── render.html           Fullscreen output page used by renderer windows
│   ├── styles.css
│   └── app.js                  Tauri IPC + event handling
└── src-tauri/                  Rust backend
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── build.rs
    └── src/
        ├── main.rs             Binary entry point
        ├── lib.rs              Tauri builder + IPC commands + hotplug watcher
        ├── display/            Display management
        │   ├── mod.rs          Public API (list/modes/set, DisplayError)
        │   ├── model.rs        DisplayInfo, DisplayBounds, DisplayMode, connector labels
        │   ├── hotplug.rs      Change-detection helpers (unit tested)
        │   ├── platform_windows.rs   Win32 GDI + display config implementation
        │   └── platform_other.rs     Non-Windows stub (returns an error)
        ├── media/              MediaItem registry + media:// protocol handler
        ├── playlist/           RepeatMode + ordered Playlist (unit tested)
        ├── scheduler/          Background scheduler kernel + control State
        └── renderer/           Fullscreen/borderless output windows
```

Each feature is isolated in its own module so display management, media
playback, scheduling, and output rendering can be developed independently.

### How the IPC works

`ui/app.js` talks to the Rust commands registered in `src-tauri/src/lib.rs`:

| Command                 | Returns                | Purpose                                       |
| ----------------------- | ---------------------- | --------------------------------------------- |
| `list_displays`         | `Vec<DisplayInfo>`     | Detected displays + connector type + bounds   |
| `get_display_modes`     | `DisplayModes`         | Current mode + supported modes                |
| `set_display_mode`      | `()`                   | Apply a resolution to a display               |
| `scheduler_set_playlist`| `usize`                | Replace the playlist (`paths`); returns count |
| `scheduler_play`        | `()`                   | Start playback on a display (`deviceName`)    |
| `scheduler_pause`       | `()`                   | Pause the currently playing video             |
| `scheduler_resume`      | `()`                   | Resume the paused video                       |
| `scheduler_next`        | `()`                   | Advance to the next playlist entry            |
| `scheduler_prev`        | `()`                   | Go back to the previous entry                 |
| `scheduler_stop`        | `()`                   | Stop playback and close render windows        |
| `scheduler_set_dwell`   | `()`                   | Image display duration in ms (`millis`, min 100) |
| `scheduler_status`      | `Snapshot`             | Current playback state (polled)               |
| `renderer_render_token` | `RenderPayload`        | `media://` URL + kind + title for render.html |
| `renderer_show`         | `()`                   | Preview a single file on a display            |
| `renderer_close_all`    | `usize`                | Close all renderer windows                    |

Structures are serialized to camelCase JSON. The backend also runs a
hotplug watcher that emits the `displays-changed` event; the UI listens and
re-enumerates (which refreshes the output-display selector too).

### Async events

- `displays-changed` — emitted by the hotplug watcher on connect/disconnect.
- `scheduler-state` — `Snapshot` emitted from the scheduler kernel whenever
  playback state changes (playing/paused, current item, last error).
- `output-control` — sent by the scheduler to a render window: `true` =
  pause, `false` = resume the video.
- `render-finished` — emitted by `render.html` when a video ends; the
  scheduler advances to the next item.
- `tauri://drag-drop` — built-in Tauri event carrying `paths` when files are
  dropped on a window; the UI adds them to the playlist.

### Media loading

There is no file-picker plugin and no process spawning. Files are added two
ways:

1. **Drag-and-drop** — the window receives the built-in `tauri://drag-drop`
   event with absolute file paths.
2. **Manual path** — paste a path into the text field and press Add.

On set, the backend resolves each path to canonical form and records a
`MediaItem` (`path`, basename, `image`/`video` kind). The scheduler opens a
renderer window on the target display, and `render.html` asks for a
`media://<id>` URL via `renderer_render_token`. The `media://` protocol
handler streams the file bytes with `Accept-Ranges`/`Range` support so long
videos seek correctly. CSP grants the renderer `img-src`/`media-src` access
to the `media:` scheme.

### Playback scheduler

A kernel thread polls a command channel every 50 ms and also listens for
`render-finished` events:

- Images dwell for the configured duration (default 5 s, settable in the
  UI) then advance automatically.
- Videos play to the end and advance on the `render-finished` event.
- Pause/resume applies to the active video window via `output-control`.
- Playlist repeat is forced to repeat-all so shows run until stopped.
- The snapshot (`scheduler_status` / `scheduler-state`) keeps the UI's
  play/pause/next buttons and the active queue item in sync.
- Errors are surfaced through the snapshot's `lastError` field.

Renderer windows must be created from asynchronous contexts on Windows
(calling `WebviewWindowBuilder::build` inside a synchronous command
deadlocks the main thread), so `scheduler::spawn` runs on a dedicated
thread.

### Windows display management

`src-tauri/src/display/platform_windows.rs` uses the lightweight
`windows-sys` bindings (declarations only — no runtime, correct `DEVMODE`
and `DISPLAYCONFIG_*` layouts without hand-maintaining them):

1. **Display list**: two-level `EnumDisplayDevices` enumeration (adapters
   own the `\\.\DISPLAYN` names; each adapter drives one or more monitors).
   Mirroring drivers and devices not attached to the desktop are filtered
   out. `EDD_GET_DEVICE_INTERFACE_NAME` yields a stable device id.
2. **Connector type**: `QueryDisplayConfig` path targets report a
   `DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY`; the resulting labels are merged
   into the display list by matching the monitor device-interface path.
3. **Resolutions**: `EnumDisplaySettingsEx` walks all modes for a display;
   `ENUM_CURRENT_SETTINGS` gives the mode in effect and the current
   position/size (used to place renderer windows).
4. **Mode selection**: `ChangeDisplaySettingsEx` is called with `CDS_TEST`
   first, then applied with `CDS_UPDATEREGISTRY` (persisted across boots).

If the display config can't be queried the app degrades gracefully: the
display list still shows, just without a `connectionKind`.

**Hotplug detection** is polling-based for now: every 2 seconds the backend
compares the sorted set of connected monitor ids and emits
`displays-changed` on any change. A future iteration can switch to
`RegisterDeviceNotification` / `WM_DISPLAYCHANGE` for push-based events.

## Prerequisites

- Rust stable (1.77.2+).
- **Windows** (target platform): WebView2 (preinstalled on Windows 11).
- Optional, for `tauri` CLI workflows: `cargo install tauri-cli` or
  `npm i -D @tauri-apps/cli`.
- **Linux** (development/host checks only): `webkit2gtk-4.1`, `gtk3`,
  `libsoup-3.0`, `javascriptcoregtk-4.1` development packages.

## Build and run

```sh
# From the repository root
cd src-tauri

cargo run          # plain build/run (serve ui/ statically)
cargo test         # unit tests
cargo fmt          # format
cargo clippy       # lints
cargo build --release
```

With the Tauri CLI (from the repository root):

```sh
cargo tauri dev
cargo tauri build --target x86_64-pc-windows-msvc
```

On non-Windows hosts the app still builds and runs; display operations
return a "display management is only supported on Windows" error. Playback
and the `media://` scheme work on any platform with a WebView backend.

## Windows installer

Installers are produced by CI (`.github/workflows/`): the
`build-and-test.yml` workflow runs fmt/clippy/tests on the push branch, and
`release.yml` builds `x86_64-pc-windows-msvc` with `tauri-action` on tagged
releases, producing signed-package MSI and NSIS installers as release
artifacts. Building installers requires Windows (the Tauri/NSIS toolchain),
so it happens in the workflow rather than on a dev machine. On Windows,
`cargo tauri build` locally produces both `.msi` and `.exe` (NSIS).

## Checks and tests

```sh
cd src-tauri
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Roadmap

- [x] Detect HDMI connection type (`QueryDisplayConfig`).
- [x] Enumerate and select supported output resolutions (`EnumDisplaySettingsEx`
      + `ChangeDisplaySettingsEx`).
- [x] Resolution/connection surfaced through IPC with UI selection controls.
- [x] Display hotplug detection (re-enumerate on connect/disconnect via a
      polling watcher that emits `displays-changed`).
- [x] Per-display fullscreen/borderless output windows (renderer).
- [x] Load local image and video files (drag-and-drop + manual path + `media://`).
- [x] Playlists and scheduled playback (images dwell, videos auto-advance, pause/resume).
- [x] Windows installer pipeline (MSI/NSIS via GitHub Actions).