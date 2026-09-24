# HDMI Media Controller

A Windows desktop application (Rust + [Tauri 2](https://tauri.app)) for
controlling media playback on external HDMI-connected displays: pick an
output display, pick its resolution, and show local images/videos
fullscreen on it while the control UI stays on the laptop display.

## Current status

Initial implementation. The following is working today:

- Tauri 2 project skeleton (Rust backend, static HTML/CSS/JS control UI).
- Native Windows display enumeration via the Win32 `EnumDisplayDevices`
  API (raw FFI, no extra crates) — displays are listed in the UI.
- Module layout for the features that will be built next.

## Architecture

```
.
├── ui/                         Static control UI (no npm/bundler needed)
│   ├── index.html
│   ├── styles.css
│   └── app.js                  Calls `list_displays` over Tauri IPC
└── src-tauri/                  Rust backend
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── build.rs
    └── src/
        ├── main.rs             Binary entry point
        ├── lib.rs              Tauri builder + IPC commands
        ├── display/            Display management
        │   ├── mod.rs          Public API (`list_displays`, `DisplayError`)
        │   ├── model.rs        `DisplayInfo`, `StateFlags` (unit tested)
        │   ├── platform_windows.rs   Win32 EnumDisplayDevices FFI
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

`ui/app.js` invokes the Rust command `list_displays` (registered in
`src-tauri/src/lib.rs`). The command returns `Vec<DisplayInfo>` serialized
to camelCase JSON, which the UI renders as cards (friendly name, device
name, stable id, primary/active/attached badges).

### Windows display enumeration

`src-tauri/src/display/platform_windows.rs` performs a two-level Win32
enumeration:

1. Display adapters (which own the `\\.\DISPLAYN` device names).
2. The monitor(s) driven by each adapter, using
   `EDD_GET_DEVICE_INTERFACE_NAME` to obtain a stable device interface id.

Mirroring-driver devices and monitors that are not attached to the desktop
are filtered out. Connection-type detection (HDMI vs DisplayPort, etc.)
will be added later using `QueryDisplayConfig`.

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

On non-Windows hosts the app still builds and runs; the display list will
show a "display enumeration is only supported on Windows" error.

## Checks and tests

```sh
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Roadmap

- [ ] Detect HDMI connection type (`QueryDisplayConfig`).
- [ ] Enumerate and select supported output resolutions (`EnumDisplaySettingsEx`).
- [ ] Load local image and video files.
- [ ] Fullscreen/borderless output window on the selected external display,
      control UI stays on the primary display.
- [ ] Playlists and scheduled playback.
- [ ] Windows installer (NSIS/MSI via Tauri bundler).