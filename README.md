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
- Module layout for the features that will be built next.

## Architecture

```
.
├── ui/                         Static control UI (no npm/bundler needed)
│   ├── index.html
│   ├── styles.css
│   └── app.js                  Tauri IPC + displays-changed event handling
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
        │   ├── model.rs        DisplayInfo, DisplayMode, connector labels (unit tested)
        │   ├── hotplug.rs      Change-detection helpers (unit tested)
        │   ├── platform_windows.rs   Win32 GDI + display config implementation
        │   └── platform_other.rs     Non-Windows stub (returns an error)
        ├── media/              Media loading/playback (planned)
        ├── playlist/           Playlist handling (planned)
        ├── scheduler/          Scheduled playback (planned)
        └── renderer/           Fullscreen/borderless output (planned)
```

Each future feature is isolated in its own module so display management,
media playback, scheduling, and output rendering can be developed
independently.

### How the IPC works

`ui/app.js` talks to the Rust commands registered in `src-tauri/src/lib.rs`:

| Command              | Returns                       | Purpose                            |
| -------------------- | ----------------------------- | ---------------------------------- |
| `list_displays`      | `Vec<DisplayInfo>`            | Detected displays + connector type |
| `get_display_modes`  | `DisplayModes`                | Current mode + supported modes     |
| `set_display_mode`   | `()`                          | Apply a resolution to a display    |

Structures are serialized to camelCase JSON. The backend also runs a
hotplug watcher that emits the `displays-changed` event; the UI listens and
re-enumerates.

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
   `ENUM_CURRENT_SETTINGS` gives the mode in effect.
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
return a "display management is only supported on Windows" error.

## Checks and tests

```sh
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Roadmap

- [x] Detect HDMI connection type (`QueryDisplayConfig`).
- [x] Enumerate and select supported output resolutions (`EnumDisplaySettingsEx`
      + `ChangeDisplaySettingsEx`).
- [x] Resolution/connection surfaced through IPC with UI selection controls.
- [x] Display hotplug detection (re-enumerate on connect/disconnect via a
      polling watcher that emits `displays-changed`).
- [ ] Per-display fullscreen/borderless output windows (renderer).
- [ ] Load local image and video files.
- [ ] Playlists and scheduled playback.
- [ ] Windows installer (NSIS/MSI via Tauri bundler).