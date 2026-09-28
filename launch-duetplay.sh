#!/bin/bash
# DuetPlay Linux launcher - uses X11 backend for Wayland compatibility
# Source this file or run with: GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 ./launch-duetplay.sh
export GDK_BACKEND=x11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
exec /home/george/Development/Rust/hdmi-media-controller/src-tauri/target/debug/hdmi-media-controller "$@" > /dev/null 2>&1 &