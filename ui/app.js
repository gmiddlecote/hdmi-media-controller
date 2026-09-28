// Control UI for DuetPlay.
//
// Talks to the Rust backend over Tauri IPC. `window.__TAURI__` is provided
// by the `app.withGlobalTauri` setting in tauri.conf.json, so no bundler or
// npm packages are required.
//
// Events: listens for `displays-changed` (emitted by the backend hotplug
// watcher) and re-enumerates the display list automatically.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Theme management
const THEME_KEY = "duetplay-theme";
const THEMES = ["system", "dark", "light"];

function getSystemTheme() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(theme) {
  const effective = theme === "system" ? getSystemTheme() : theme;
  document.documentElement.setAttribute("data-theme", effective);
  localStorage.setItem(THEME_KEY, theme);
  updateThemeIcon(effective);
}

function updateThemeIcon(effective) {
  const btn = document.getElementById("theme-toggle");
  if (!btn) return;
  const icons = { dark: "☀️", light: "🌙", system: "🖥️" };
  btn.textContent = icons[effective] || icons.dark;
  btn.title = `Theme: ${effective.charAt(0).toUpperCase() + effective.slice(1)} (click to cycle)`;
}

function cycleTheme() {
  const current = localStorage.getItem(THEME_KEY) || "system";
  const idx = THEMES.indexOf(current);
  const next = THEMES[(idx + 1) % THEMES.length];
  applyTheme(next);
  const select = document.getElementById("theme-select");
  if (select) select.value = next;
}

function initTheme() {
  const saved = localStorage.getItem(THEME_KEY) || "system";
  applyTheme(saved);
  const select = document.getElementById("theme-select");
  if (select) select.value = saved;
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    const current = localStorage.getItem(THEME_KEY) || "system";
    if (current === "system") applyTheme("system");
  });
}

// Keyboard shortcuts help dialog
function showShortcuts() {
  document.getElementById("shortcuts-dialog").classList.remove("hidden");
  document.getElementById("close-shortcuts").focus();
}

function hideShortcuts() {
  document.getElementById("shortcuts-dialog").classList.add("hidden");
}

// Tab management
function showTab(tabName) {
  console.log("[DEBUG] showTab called with:", tabName);
  const buttons = document.querySelectorAll(".tab-btn");
  const panels = document.querySelectorAll(".tab-panel");
  if (!buttons.length || !panels.length) {
    console.warn("[DEBUG] showTab: No buttons or panels found");
    return;
  }
  
  buttons.forEach(btn => {
    const isActive = btn.dataset.tab === tabName;
    btn.classList.toggle("active", isActive);
    btn.setAttribute("aria-selected", isActive);
  });
  panels.forEach(panel => {
    const isActive = panel.id === `${tabName}-panel`;
    panel.hidden = !isActive;
    if (isActive) panel.classList.add("active");
    else panel.classList.remove("active");
    console.log("[DEBUG] Panel", panel.id, "hidden:", panel.hidden);
  });
}

function initTabs() {
  const buttons = document.querySelectorAll(".tab-btn");
  const panels = document.querySelectorAll(".tab-panel");
  console.log("[DEBUG] initTabs called, buttons:", buttons.length, "panels:", panels.length);
  if (!buttons.length || !panels.length) {
    console.warn("[DEBUG] No tab buttons or panels found!");
    return;
  }
  
  buttons.forEach(btn => {
    btn.addEventListener("click", () => showTab(btn.dataset.tab));
    btn.addEventListener("keydown", (e) => {
      if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        const tabs = Array.from(document.querySelectorAll(".tab-btn"));
        const idx = tabs.indexOf(btn);
        const next = e.key === "ArrowRight" ? (idx + 1) % tabs.length : (idx - 1 + tabs.length) % tabs.length;
        tabs[next].focus();
        showTab(tabs[next].dataset.tab);
      }
    });
  });
  
  // Ensure initial state is correct
  showTab("displays");
}

// Transport shortcuts: Esc stop, Space pause/resume, ←/→ previous/next, ? help.
// Ignored while typing so they do not fight with the text fields.
document.addEventListener("keydown", (event) => {
  const target = event.target;
  const typing =
    target &&
    (target.tagName === "INPUT" ||
      target.tagName === "SELECT" ||
      target.tagName === "TEXTAREA" ||
      target.isContentEditable);

  // Global shortcuts that work even when typing in some cases
  if (event.key === "Escape") {
    if (event.repeat) return;
    // Close dialogs first
    if (!document.getElementById("update-dialog").classList.contains("hidden")) {
      document.getElementById("update-later-btn").click();
      return;
    }
    if (!document.getElementById("shortcuts-dialog").classList.contains("hidden")) {
      hideShortcuts();
      return;
    }
    event.preventDefault();
    invoke("scheduler_stop").catch(() => {});
    return;
  }

  if (event.key === "?" && !typing) {
    event.preventDefault();
    showShortcuts();
    return;
  }

  if (typing) return;

  if (event.key === " " || event.key === "Spacebar") {
    event.preventDefault();
    if (!lastSnapshot || !lastSnapshot.playing) return;
    const command = lastSnapshot.paused ? "scheduler_resume" : "scheduler_pause";
    invoke(command).catch(() => {});
    return;
  }

  if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    event.preventDefault();
    const command = event.key === "ArrowLeft" ? "scheduler_prev" : "scheduler_next";
    invoke(command).catch(() => {});
  }
});

const refreshBtn = document.getElementById("refresh-btn");
const listEl = document.getElementById("display-list");
const statusEl = document.getElementById("status");
const versionEl = document.getElementById("app-version");
const updateStatusEl = document.getElementById("update-status");
const checkUpdatesBtn = document.getElementById("check-updates-btn");
const exitBtn = document.getElementById("exit-btn");
const updateDialog = document.getElementById("update-dialog");
const updateMessage = document.getElementById("update-message");
const updateLaterBtn = document.getElementById("update-later-btn");
const updateInstallBtn = document.getElementById("update-install-btn");
const themeToggleBtn = document.getElementById("theme-toggle");
const helpBtn = document.getElementById("help-btn");
const themeSelect = document.getElementById("theme-select");
const showShortcutsBtn = document.getElementById("show-shortcuts");
const settingsVersionEl = document.getElementById("settings-version");
const shortcutsDialog = document.getElementById("shortcuts-dialog");
const closeShortcutsBtn = document.getElementById("close-shortcuts");
const closeShortcutsFooterBtn = document.getElementById("close-shortcuts-btn");

const updateApiUrl = "https://api.github.com/repos/gmiddlecote/hdmi-media-controller/releases/latest";
let currentVersion = "";
let pendingUpdate = null;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function badge(label, on) {
  return el("span", `badge ${on ? "badge-on" : "badge-off"}`, label);
}

/// Label for a display dropdown, using the OS device number (DISPLAY1 ->
/// "Display 1") since monitor make/model is often generic.
function displayLabel(display) {
  const device = display.deviceName || "";
  const tail = device.slice(device.lastIndexOf("\\") + 1);
  const match = /DISPLAY(\d+)/i.exec(tail);
  const name = match ? `Display ${match[1]}` : display.friendlyName;
  return `${name} (${display.connectionKind || "unknown connector"})`;
}

function formatMode(mode) {
  return `${mode.width}x${mode.height} @ ${mode.refreshRate} Hz${
    mode.bitDepth ? ` (${mode.bitDepth}-bit)` : ""
  }`;
}

function parseVersion(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/i.exec(String(value).trim());
  if (!match) return null;
  return match.slice(1, 4).map(Number);
}

function isNewerVersion(candidate, current) {
  const next = parseVersion(candidate);
  const present = parseVersion(current);
  if (!next || !present) return false;
  for (let index = 0; index < 3; index += 1) {
    if (next[index] !== present[index]) return next[index] > present[index];
  }
  return false;
}

function setUpdateStatus(message, error = false) {
  updateStatusEl.textContent = message;
  updateStatusEl.classList.toggle("error", error);
}

async function loadAppVersion() {
  try {
    currentVersion = await invoke("app_version");
    versionEl.textContent = `Version ${currentVersion}`;
    if (settingsVersionEl) settingsVersionEl.textContent = `Version ${currentVersion}`;
  } catch (_) {
    versionEl.textContent = "Version unavailable";
    if (settingsVersionEl) settingsVersionEl.textContent = "Version unavailable";
  }
}

function findWindowsInstaller(release) {
  const assets = Array.isArray(release.assets) ? release.assets : [];
  return (
    assets.find((asset) => /_x64-setup\.exe$/i.test(asset.name || "")) ||
    assets.find(
      (asset) =>
        /\.exe$/i.test(asset.name || "") &&
        !/blockmap|debug|symbols/i.test(asset.name || "")
    ) ||
    null
  );
}

function showUpdateDialog(update) {
  pendingUpdate = update;
  updateMessage.textContent = `DuetPlay ${update.version} is available. Download and install it now?`;
  updateDialog.classList.remove("hidden");
  updateInstallBtn.focus();
}

function dismissUpdateDialog() {
  updateDialog.classList.add("hidden");
}

async function checkForUpdates() {
  if (!currentVersion || checkUpdatesBtn.disabled) return;
  checkUpdatesBtn.disabled = true;
  setUpdateStatus("Checking for updates…");
  try {
    const response = await fetch(updateApiUrl, {
      cache: "no-store",
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
    const release = await response.json();
    const latestVersion = String(release.tag_name || release.name || "").replace(/^v/i, "");
    if (!isNewerVersion(latestVersion, currentVersion)) {
      setUpdateStatus("Up to date");
      return;
    }
    const asset = findWindowsInstaller(release);
    const digest = String(asset && asset.digest ? asset.digest : "")
      .replace(/^sha256:/i, "")
      .toLowerCase();
    const size = Number(asset && asset.size);
    if (!asset || !asset.browser_download_url || !/^[0-9a-f]{64}$/.test(digest) || !Number.isSafeInteger(size) || size <= 0) {
      setUpdateStatus(`Update ${latestVersion} has no verified Windows installer`, true);
      return;
    }
    setUpdateStatus(`Update ${latestVersion} available`);
    showUpdateDialog({
      version: latestVersion,
      url: asset.browser_download_url,
      sha256: digest,
      size,
    });
  } catch (_) {
    setUpdateStatus("Update check unavailable", true);
  } finally {
    checkUpdatesBtn.disabled = false;
  }
}

async function installPendingUpdate() {
  if (!pendingUpdate) return;
  updateInstallBtn.disabled = true;
  updateLaterBtn.disabled = true;
  setUpdateStatus("Downloading and installing…");
  try {
    await invoke("download_and_install_update", {
      version: pendingUpdate.version,
      url: pendingUpdate.url,
      sha256: pendingUpdate.sha256,
      size: pendingUpdate.size,
    });
  } catch (error) {
    updateInstallBtn.disabled = false;
    updateLaterBtn.disabled = false;
    updateMessage.textContent = `Update failed: ${String(error)}`;
    setUpdateStatus("Update failed", true);
  }
}

checkUpdatesBtn.addEventListener("click", checkForUpdates);
exitBtn.addEventListener("click", () => invoke("app_exit").catch(() => {}));
updateLaterBtn.addEventListener("click", () => {
  dismissUpdateDialog();
  pendingUpdate = null;
  setUpdateStatus("Update available for the next launch");
});
updateInstallBtn.addEventListener("click", installPendingUpdate);
themeToggleBtn?.addEventListener("click", cycleTheme);
helpBtn?.addEventListener("click", showShortcuts);
themeSelect?.addEventListener("change", (e) => applyTheme(e.target.value));
showShortcutsBtn?.addEventListener("click", showShortcuts);
closeShortcutsBtn?.addEventListener("click", hideShortcuts);
closeShortcutsFooterBtn?.addEventListener("click", hideShortcuts);
shortcutsDialog?.addEventListener("click", (e) => {
  if (e.target === shortcutsDialog) hideShortcuts();
});

function buildResolutionSection(card, display) {
  const section = el("div", "resolution");
  section.append(el("div", "resolution-label", "Resolution"));

  const currentLabel = el("div", "mode-current", "Loading…");
  const select = el("select", "mode-select");
  select.disabled = true;
  const applyBtn = el("button", "apply-btn", "Apply");
  applyBtn.disabled = true;

  const row = el("div", "resolution-row");
  row.append(select, applyBtn);

  const cardStatus = el("p", "card-status");
  section.append(currentLabel, row, cardStatus);

  let modes = [];

  select.addEventListener("change", () => {
    applyBtn.disabled = select.selectedIndex < 0;
  });

  applyBtn.addEventListener("click", async () => {
    const mode = modes[select.selectedIndex];
    if (!mode) return;
    applyBtn.disabled = true;
    cardStatus.textContent = `Applying ${formatMode(mode)}…`;
    try {
      await invoke("set_display_mode", {
        deviceName: display.deviceName,
        width: mode.width,
        height: mode.height,
        refreshRate: mode.refreshRate,
      });
      cardStatus.textContent = `Applied ${formatMode(mode)}.`;
      currentLabel.textContent = `Current: ${formatMode(mode)}`;
      // Update the current mode badge if present
      updateCurrentModeBadge(card, mode);
    } catch (error) {
      cardStatus.textContent = `Failed: ${String(error)}`;
    } finally {
      applyBtn.disabled = select.selectedIndex < 0;
    }
  });

  invoke("get_display_modes", { deviceName: display.deviceName })
    .then((result) => {
      modes = result.modes;
      currentLabel.textContent = `Current: ${formatMode(result.current)}`;

      // Add current mode badge
      updateCurrentModeBadge(card, result.current);

      let currentMatch = null;
      select.replaceChildren();
      modes.forEach((mode, index) => {
        const opt = el("option", "", formatMode(mode));
        opt.value = String(index);
        if (
          mode.width === result.current.width &&
          mode.height === result.current.height &&
          mode.refreshRate === result.current.refreshRate
        ) {
          currentMatch = index;
        }
        select.append(opt);
      });
      if (currentMatch !== null) select.selectedIndex = currentMatch;
      select.disabled = modes.length === 0;
      applyBtn.disabled = modes.length === 0;
      if (modes.length === 0) {
        currentLabel.textContent = "Current: " + formatMode(result.current);
        cardStatus.textContent = "No additional modes reported by the display.";
      }
    })
    .catch((error) => {
      currentLabel.textContent = "Resolutions unavailable";
      cardStatus.textContent = String(error);
    });

  card.append(section);
}

function updateCurrentModeBadge(card, mode) {
  let badge = card.querySelector(".current-mode-badge");
  if (!badge) {
    badge = el("div", "current-mode-badge");
    const meta = card.querySelector(".display-meta");
    if (meta) meta.insertAdjacentElement("afterend", badge);
  }
  badge.textContent = `Current: ${formatMode(mode)}`;
}

function renderDisplays(list) {
  displays = list;
  listEl.replaceChildren();
  syncOutputSelect(list);
  renderQueue();

  if (displays.length === 0) {
    const emptyState = el("div", "empty-state");
    emptyState.innerHTML = `
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
        <path d="M8 21h8M12 17v4"/>
      </svg>
      <h3>No displays detected</h3>
      <p>Connect an HDMI display and click Refresh</p>
    `;
    const refreshBtn = el("button", "subtle-btn", "Refresh");
    refreshBtn.addEventListener("click", loadDisplays);
    emptyState.appendChild(refreshBtn);
    listEl.append(emptyState);
    statusEl.textContent = "No displays found.";
    return;
  }

  for (const display of displays) {
    const card = el("article", "display-card");
    if (display.isActive) card.classList.add("current-mode");

    const title = el("div", "display-title", display.friendlyName);
    const meta = el(
      "div",
      "display-meta",
      `${display.deviceName}\n${display.id}`
    );

    const badges = el("div", "badges");
    badges.append(
      badge("primary", display.isPrimary),
      badge("active", display.isActive),
      badge("attached", display.isAttached)
    );
    if (display.connectionKind) {
      badges.append(badge(display.connectionKind, true));
    }

    card.append(title, meta, badges);
    buildResolutionSection(card, display);
    listEl.append(card);
  }

  statusEl.textContent = `${displays.length} display(s) detected.`;
}

async function loadDisplays() {
  refreshBtn.disabled = true;
  statusEl.textContent = "Enumerating displays…";

  try {
    const displays = await invoke("list_displays");
    renderDisplays(displays);
  } catch (error) {
    listEl.replaceChildren();
    listEl.append(el("div", "error", String(error)));
    statusEl.textContent = "Display enumeration failed.";
  } finally {
    refreshBtn.disabled = false;
  }
}

refreshBtn.addEventListener("click", loadDisplays);
listen("displays-changed", () => loadDisplays());
loadDisplays();

// ---------------------------------------------------------------------------
// Playback: queue, output display, transport controls.
// ---------------------------------------------------------------------------

const dropzone = document.getElementById("dropzone");
const dropzoneHint = document.getElementById("dropzone-hint");
const pathInput = document.getElementById("path-input");
const addPathBtn = document.getElementById("add-path-btn");
const queueEl = document.getElementById("queue");
const outputDisplay = document.getElementById("output-display");
const dwellInput = document.getElementById("dwell-input");
const volumeInput = document.getElementById("volume-input");
const playBtn = document.getElementById("play-btn");
const pauseBtn = document.getElementById("pause-btn");
const resumeBtn = document.getElementById("resume-btn");
const prevBtn = document.getElementById("prev-btn");
const nextBtn = document.getElementById("next-btn");
const stopBtn = document.getElementById("stop-btn");
const previewBtn = document.getElementById("preview-btn");
const playbackStatus = document.getElementById("playback-status");

let queuedPaths = [];
let displays = [];
let pendingDisplay = "";
let audioDevices = [];
let dragSourceIndex = null;

function loadAudioDevices() {
  invoke("list_audio_devices")
    .then((devices) => {
      audioDevices = devices;
      renderQueue();
    })
    .catch(() => {
      audioDevices = [];
      renderQueue();
    });
}

loadAudioDevices();

function queueEntry(path) {
  return {
    path,
    fit: "cover",
    display: outputDisplay.value || "",
    audioDevice: "",
    overlay: "",
    overlay_v: "bottom",
    overlay_h: "left",
    seconds: 5,
  };
}

function syncOutputSelect(displays) {
  const previous = pendingDisplay || outputDisplay.value;
  pendingDisplay = "";
  outputDisplay.replaceChildren();
  if (displays.length === 0) {
    const option = el("option", "", "No displays");
    option.disabled = true;
    outputDisplay.append(option);
    outputDisplay.disabled = true;
  } else {
    displays.forEach((display) => {
      const option = el("option", "", displayLabel(display));
      option.value = display.deviceName;
      outputDisplay.append(option);
    });
    const keep = displays.some((d) => d.deviceName === previous)
      ? previous
      : displays[0].deviceName;
    outputDisplay.value = keep;
    outputDisplay.disabled = false;
  }
}

const VIDEO_RE = /\.(mp4|webm|mov|mkv|avi|ogv|m4v)$/i;
const AUDIO_RE = /\.(mp3|m4a|aac|wav|flac|ogg|oga|opus|wma)$/i;

function mediaKind(path) {
  if (VIDEO_RE.test(path)) return "video";
  if (AUDIO_RE.test(path)) return "audio";
  return "image";
}

/// Starts one specific queued item with the chosen mode on its monitor (or
/// the master monitor when the item inherits it). `mode` is `once`, `loop`,
/// or `timed` (backend enum).
async function playItem(path, mode, seconds) {
  const entry = queuedPaths.find((queued) => queued.path === path);
  const device = (entry && entry.display) || outputDisplay.value || "";
  if (!device) {
    playbackStatus.textContent = "Choose an output display first.";
    return;
  }
  const fit = entry ? entry.fit : "cover";
  const audioDevice = entry ? entry.audioDevice : "";
  try {
    await invoke("scheduler_play_item", {
      path,
      deviceName: device,
      mode,
      seconds: seconds > 0 ? seconds : 5,
      fit,
      audioDevice,
    });
  } catch (error) {
    playbackStatus.textContent = `Could not play: ${String(error)}`;
  }
}

async function openPreview(path) {
  try {
    await invoke("renderer_open_preview", { path });
    playbackStatus.textContent = "Video preview opened.";
  } catch (error) {
    playbackStatus.textContent = `Could not open video preview: ${String(error)}`;
  }
}

let lastSnapshot = null;

function setItemFit(path, fit) {
  if (lastSnapshot && lastSnapshot.playing && lastSnapshot.path === path) {
    invoke("scheduler_set_fit", { path, fit }).catch(() => {});
  }
}

function moveQueuedItem(fromIndex, toIndex) {
  if (toIndex < 0 || toIndex >= queuedPaths.length || fromIndex === toIndex) return;
  const [item] = queuedPaths.splice(fromIndex, 1);
  queuedPaths.splice(toIndex, 0, item);
  if (activeIndex === fromIndex) {
    activeIndex = toIndex;
  } else if (activeIndex === toIndex) {
    activeIndex = fromIndex > toIndex ? activeIndex + 1 : activeIndex - 1;
  } else if (fromIndex < activeIndex && toIndex >= activeIndex) {
    activeIndex--;
  } else if (fromIndex > activeIndex && toIndex <= activeIndex) {
    activeIndex++;
  }
  renderQueue();
  pushPlaylist();
}

function renderQueue() {
  queueEl.replaceChildren();
  queuedPaths.forEach((queued, index) => {
    const path = queued.path;
    const item = el("li", "queued-item");
    item.draggable = true;
    item.dataset.index = index;

    // Drag and drop handlers
    item.addEventListener("dragstart", (e) => {
      dragSourceIndex = index;
      item.classList.add("dragging");
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", index);
    });
    item.addEventListener("dragend", () => {
      item.classList.remove("dragging");
      dragSourceIndex = null;
      document.querySelectorAll(".queued-item.drag-over").forEach(el => el.classList.remove("drag-over"));
    });
    item.addEventListener("dragover", (e) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      if (dragSourceIndex !== null && dragSourceIndex !== index) {
        item.classList.add("drag-over");
      }
    });
    item.addEventListener("dragleave", () => {
      item.classList.remove("drag-over");
    });
    item.addEventListener("drop", (e) => {
      e.preventDefault();
      item.classList.remove("drag-over");
      if (dragSourceIndex !== null && dragSourceIndex !== index) {
        moveQueuedItem(dragSourceIndex, index);
      }
    });

    // Thumbnail area — click = play once. A live frame comes from the
    // media:// url (img for images, video poster for videos); the kind
    // badge stays as a fallback when decoding fails.
    const media = el("div", "queued-media");
    media.title = "Play once (hover video to preview)";
    const kindBadge = el("span", "queued-kind", mediaKind(path).toUpperCase());
    media.append(kindBadge);
    media.addEventListener("click", () => playItem(path, "once", 0));

    // Hover preview for videos
    let hoverVideo = null;
    media.addEventListener("mouseenter", () => {
      const kind = mediaKind(path);
      if (kind !== "video" || hoverVideo) return;
      invoke("media_register", { path })
        .then((url) => {
          hoverVideo = el("video", "thumb");
          hoverVideo.muted = true;
          hoverVideo.playsInline = true;
          hoverVideo.preload = "auto";
          hoverVideo.loop = true;
          hoverVideo.src = url;
          media.append(hoverVideo);
          hoverVideo.play().catch(() => {});
          kindBadge.style.opacity = "0";
        })
        .catch(() => {});
    });
    media.addEventListener("mouseleave", () => {
      if (hoverVideo) {
        hoverVideo.remove();
        hoverVideo = null;
        kindBadge.style.opacity = "";
      }
    });

    const loadThumb = (url) => {
      const showBadge = () => kindBadge.remove();
      const kind = mediaKind(path);
      if (kind === "video") {
        const video = el("video", "thumb");
        video.muted = true;
        video.playsInline = true;
        video.preload = "metadata";
        video.addEventListener("loadedmetadata", showBadge);
        video.addEventListener("error", () => video.remove());
        video.src = url;
        media.append(video);
      } else if (kind === "image") {
        const img = el("img", "thumb");
        img.alt = "";
        img.addEventListener("load", showBadge);
        img.addEventListener("error", () => img.remove());
        img.src = url;
        media.append(img);
      }
    };
    invoke("media_register", { path })
      .then(loadThumb)
      .catch(() => {});

    const name = path.split(/[\\/]/).pop() || path;

    // File name below thumbnail
    const nameEl = el("div", "queued-name", name);
    nameEl.title = path + "\nClick to play once";
    nameEl.addEventListener("click", () => playItem(path, "once", 0));

    // Right side: action groups (2-3 lines)
    const actions = el("div", "queued-actions");

    // Group 1: Playback mode (Once / Loop / Secs)
    const playbackGroup = el("div", "action-group");
    const playbackLabel = el("div", "action-group-label", "Playback");
    const playbackRow = el("div", "action-row");
    const once = el("button", "action", "Once");
    once.title = "Play this item once, then stop";
    once.addEventListener("click", () => playItem(path, "once", 0));
    const loop = el("button", "action", "Loop");
    loop.title = "Play this item continuously until stopped";
    loop.addEventListener("click", () => playItem(path, "loop", 0));
    const seconds = el("input", "seconds");
    seconds.type = "number";
    seconds.min = "1";
    seconds.step = "1";
    seconds.value = String(queued.seconds || 5);
    seconds.title = "Seconds to show this item";
    seconds.addEventListener("click", (event) => event.stopPropagation());
    const timed = el("button", "action", "Secs");
    timed.title = "Play this item for the set number of seconds, then stop";
    timed.addEventListener("click", () => {
      playItem(path, "timed", Number(seconds.value) || 5);
    });
    playbackRow.append(once, loop, seconds, timed);
    playbackGroup.append(playbackLabel, playbackRow);

    // Group 2: Fit mode (Cover / Contain)
    const fitGroup = el("div", "action-group");
    const fitLabel = el("div", "action-group-label", "Fit");
    const fitRow = el("div", "action-row");
    const fit = el("select", "fit-select");
    fit.title = "How the media fills the screen";
    fit.addEventListener("click", (event) => event.stopPropagation());
    const cover = el("option", "", "Cover");
    cover.value = "cover";
    const contain = el("option", "", "Contain");
    contain.value = "contain";
    fit.append(cover, contain);
    fit.value = queued.fit || "cover";
    fit.addEventListener("change", () => {
      queued.fit = fit.value;
      pushPlaylist();
      setItemFit(path, fit.value);
    });
    fitRow.append(fit);
    fitGroup.append(fitLabel, fitRow);

    // Group 3: Display (Master / specific display)
    const monitorGroup = el("div", "action-group");
    const monitorLabel = el("div", "action-group-label", "Display");
    const monitorRow = el("div", "action-row");
    const monitor = el("select", "monitor-select");
    monitor.title = "Monitor this item plays on (Master follows the master dropdown)";
    monitor.addEventListener("click", (event) => event.stopPropagation());
    const inherit = el("option", "", "Master");
    inherit.value = "";
    monitor.append(inherit);
    displays.forEach((display) => {
      const opt = el("option", "", displayLabel(display));
      opt.value = display.deviceName;
      monitor.append(opt);
    });
    monitor.value = queued.display || "";
    monitor.addEventListener("change", () => {
      queued.display = monitor.value;
      pushPlaylist();
    });
    monitorRow.append(monitor);
    monitorGroup.append(monitorLabel, monitorRow);

    // Group 4: Overlay (per-item)
    const overlayGroup = el("div", "overlay-group");
    const overlayLabel = el("div", "overlay-group-label", "Overlay");
    const overlayInputs = el("div", "overlay-inputs");
    const overlayText = el("input", "overlay-text");
    overlayText.type = "text";
    overlayText.placeholder = "Caption (empty = none)";
    overlayText.value = queued.overlay || "";
    overlayText.addEventListener("change", () => {
      queued.overlay = overlayText.value;
      pushPlaylist();
    });
    const overlayV = el("select", "overlay-v");
    overlayV.title = "Vertical position";
    const overlayVBottom = el("option", "", "Bottom");
    overlayVBottom.value = "bottom";
    const overlayVTop = el("option", "", "Top");
    overlayVTop.value = "top";
    overlayV.append(overlayVBottom, overlayVTop);
    overlayV.value = queued.overlay_v || "bottom";
    overlayV.addEventListener("change", () => {
      queued.overlay_v = overlayV.value;
      pushPlaylist();
    });
    const overlayH = el("select", "overlay-h");
    overlayH.title = "Horizontal position";
    const overlayHLeft = el("option", "", "Left");
    overlayHLeft.value = "left";
    const overlayHCenter = el("option", "", "Center");
    overlayHCenter.value = "center";
    const overlayHRight = el("option", "", "Right");
    overlayHRight.value = "right";
    overlayH.append(overlayHLeft, overlayHCenter, overlayHRight);
    overlayH.value = queued.overlay_h || "left";
    overlayH.addEventListener("change", () => {
      queued.overlay_h = overlayH.value;
      pushPlaylist();
    });
    overlayInputs.append(overlayText, overlayV, overlayH);
    overlayGroup.append(overlayLabel, overlayInputs);

    // Remove button (always at bottom right)
    const remove = el("button", "queued-remove", "\u2715");
    remove.title = "Remove from playlist";
    remove.disabled = index === activeIndex;
    remove.addEventListener("click", () => {
      queuedPaths.splice(index, 1);
      renderQueue();
      pushPlaylist();
    });

    // Up/Down buttons (for non-drag reorder)
    const up = el("button", "action", "Up");
    up.title = "Move this item up (or drag to reorder)";
    up.disabled = index === 0;
    up.addEventListener("click", () => moveQueuedItem(index, index - 1));
    const down = el("button", "action", "Down");
    down.title = "Move this item down (or drag to reorder)";
    down.disabled = index === queuedPaths.length - 1;
    down.addEventListener("click", () => moveQueuedItem(index, index + 1));

    // Preview button (for videos)
    const preview = mediaKind(path) === "video" ? el("button", "action", "Preview") : null;
    let previewActive = false;
    if (preview) {
      preview.title = "Preview (click to stop)";
      preview.addEventListener("click", () => {
        previewActive = !previewActive;
        preview.textContent = previewActive ? "Stop" : "Preview";
        if (previewActive) {
          invoke("scheduler_play_item", { path, mode: "loop", seconds: 0 }).catch(() => {});
        } else {
          invoke("scheduler_stop", {}).catch(() => {});
        }
      });
    }

    // Audio output selector (for audio files)
    let audioOutput = null;
    if (mediaKind(path) === "audio") {
      if (
        queued.audioDevice &&
        !audioDevices.some((device) => device.id === queued.audioDevice)
      ) {
        queued.audioDevice = "";
      }
      audioOutput = el("select", "audio-item-select");
      audioOutput.title = "Audio output for this item";
      audioOutput.setAttribute("aria-label", `Audio output for ${name}`);
      audioOutput.addEventListener("click", (event) => event.stopPropagation());
      const systemDefault = el("option", "", "Current system default");
      systemDefault.value = "";
      audioOutput.append(systemDefault);
      audioDevices.forEach((device) => {
        const option = el("option", "", device.name);
        option.value = device.id;
        audioOutput.append(option);
      });
      audioOutput.value = queued.audioDevice || "";
      audioOutput.disabled = audioDevices.length === 0;
      audioOutput.addEventListener("change", async () => {
        const previous = queued.audioDevice;
        queued.audioDevice = audioOutput.value;
        try {
          await invoke("scheduler_set_audio_device", {
            path,
            audioDevice: queued.audioDevice,
          });
          const selected = audioDevices.find(
            (device) => device.id === queued.audioDevice
          );
          playbackStatus.textContent = selected
            ? `Audio output saved for this item: ${selected.name}.`
            : "This item will use the current system audio output.";
          scheduleSessionSave();
        } catch (error) {
          queued.audioDevice = previous;
          audioOutput.value = previous;
          playbackStatus.textContent = `Could not set audio output: ${String(error)}`;
        }
      });
    }

    // Build right side: stack groups vertically (2-3 lines)
    actions.append(playbackGroup, fitGroup, monitorGroup, overlayGroup);
    if (audioOutput) {
      const audioGroup = el("div", "action-group");
      const audioLabel = el("div", "action-group-label", "Audio");
      audioGroup.append(audioLabel, audioOutput);
      actions.append(audioGroup);
    }
    if (preview) actions.append(preview);
    actions.append(up, down, remove);

    // Build item: media (with name below) on left, actions on right
    const mediaWrapper = el("div", "queued-media-wrapper");
    mediaWrapper.append(media, nameEl);
    item.append(mediaWrapper, actions);
    queueEl.append(item);
  });
  dropzoneHint.textContent =
    queuedPaths.length === 0
      ? "Drop image, video, or audio files here to build the playlist"
      : `${queuedPaths.length} file(s) queued. Drop more to add them.`;
}

async function addPaths(paths) {
  const accepted = paths.filter(
    (path) => !queuedPaths.some((queued) => queued.path === path)
  );
  if (accepted.length === 0) return;
  queuedPaths.push(...accepted.map(queueEntry));
  renderQueue();
  await pushPlaylist();
  playbackStatus.textContent = `${accepted.length} file(s) added to the playlist.`;
}

async function pushPlaylist() {
  try {
    const accepted = await invoke("scheduler_set_playlist", {
      entries: queuedPaths.map((queued) => ({
        path: queued.path,
        fit: queued.fit,
        display: queued.display,
        audioDevice: queued.audioDevice,
        overlay: queued.overlay,
        overlay_v: queued.overlay_v,
        overlay_h: queued.overlay_h,
      })),
    });
    if (accepted < queuedPaths.length) {
      playbackStatus.textContent = `${queuedPaths.length - accepted} file(s) skipped (not found on disk).`;
    }
  } catch (error) {
    playbackStatus.textContent = `Could not update playlist: ${String(error)}`;
  }
  scheduleSessionSave();
}

// ---------------------------------------------------------------------------
// Session restore: keep the queue and settings between runs.
// ---------------------------------------------------------------------------

// Saves stay disabled until the saved session has been read, so an early
// render cannot overwrite session.json with an empty queue.
let sessionReady = false;
let sessionSaveTimer = null;

function scheduleSessionSave() {
  if (!sessionReady) return;
  clearTimeout(sessionSaveTimer);
  sessionSaveTimer = setTimeout(saveSession, 500);
}

async function saveSession() {
  try {
    await invoke("session_save", {
      session: {
        entries: queuedPaths.map((queued) => ({
          path: queued.path,
          fit: queued.fit,
          display: queued.display,
          audioDevice: queued.audioDevice,
          overlay: queued.overlay,
          overlay_v: queued.overlay_v,
          overlay_h: queued.overlay_h,
          seconds: queued.seconds,
        })),
        dwellMillis: Math.max(1, Number(dwellInput.value) || 5) * 1000,
        overlay: overlayInput.value,
        display: outputDisplay.value || "",
        theme: localStorage.getItem(THEME_KEY) || "system",
      },
    });
  } catch (error) {
    playbackStatus.textContent = `Could not save session: ${String(error)}`;
  }
}

async function restoreSession() {
  let session = null;
  try {
    session = await invoke("session_load");
  } catch (error) {
    playbackStatus.textContent = `Could not load session: ${String(error)}`;
  }
  sessionReady = true;
  if (!session) return;

  if (Array.isArray(session.entries)) {
    queuedPaths = session.entries
      .filter((entry) => entry && typeof entry.path === "string")
      .map((entry) => ({
        path: entry.path,
        fit: entry.fit === "contain" ? "contain" : "cover",
        display: entry.display || "",
        audioDevice: entry.audioDevice || "",
        overlay: entry.overlay || "",
        overlay_v: entry.overlay_v || "bottom",
        overlay_h: entry.overlay_h || "left",
        seconds: entry.seconds || 5,
      }));
  }
  if (session.dwellMillis > 0) {
    dwellInput.value = Math.max(1, Math.round(session.dwellMillis / 1000));
  }
  if (typeof session.overlay === "string") {
    overlayInput.value = session.overlay;
    if (session.overlay) {
      invoke("scheduler_set_overlay", { text: session.overlay }).catch(() => {});
    }
  }
  if (session.display) {
    if (displays.some((display) => display.deviceName === session.display)) {
      outputDisplay.value = session.display;
    } else {
      pendingDisplay = session.display;
    }
  }
  if (session.theme) {
    applyTheme(session.theme);
    if (themeSelect) themeSelect.value = session.theme;
  }
  renderQueue();
  if (queuedPaths.length > 0) {
    playbackStatus.textContent = `${queuedPaths.length} file(s) restored from the last session.`;
  }
}

let activeIndex = -1;

function updateSnapshot(snapshot) {
  lastSnapshot = snapshot;
  const kind = snapshot.playing ? (snapshot.paused ? "Paused" : "Playing") : "Idle";
  const where = snapshot.display ? `on ${snapshot.display}` : "";
  const title = snapshot.title ? `\u201c${snapshot.title}\u201d` : "(empty playlist)";
  const item = snapshot.total ? `item ${Math.min(snapshot.index + 1, snapshot.total)} of ${snapshot.total}` : "";
  playbackStatus.textContent = `${kind}: ${title} ${where} ${item}.`.trim();
  if (snapshot.lastError) {
    playbackStatus.textContent += ` ${snapshot.lastError}`;
  }

  activeIndex = snapshot.playing ? snapshot.index : -1;
  [...queueEl.children].forEach((child, i) => {
    child.classList.toggle("active", i === activeIndex);
    const remove = child.querySelector(".queued-remove");
    if (remove) remove.disabled = i === activeIndex;
  });

  playBtn.disabled = snapshot.playing || !snapshot.total;
  stopBtn.disabled = !snapshot.playing;
  previewBtn.disabled =
    !snapshot.playing || !snapshot.path || mediaKind(snapshot.path) !== "video";
  pauseBtn.disabled = !snapshot.playing || snapshot.paused;
  resumeBtn.disabled = !snapshot.playing || !snapshot.paused;

  updateNowPlaying(snapshot);
}

// ---------------------------------------------------------------------------
// Now playing: current item thumbnail, dwell countdown / video progress.
// ---------------------------------------------------------------------------

const nowPlaying = document.getElementById("now-playing");
const npThumb = document.getElementById("np-thumb");
const npTitle = document.getElementById("np-title");
const npFill = document.getElementById("np-fill");
const npTime = document.getElementById("np-time");

let nowPlayingPath = null;

function fmtTime(seconds) {
  if (!isFinite(seconds) || seconds < 0) return "\u2013";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60)
    .toString()
    .padStart(2, "0");
  return `${m}:${s}`;
}

function updateNowPlaying(snapshot) {
  if (!snapshot.playing || !snapshot.path) {
    nowPlaying.classList.add("hidden");
    nowPlayingPath = null;
    npThumb.replaceChildren();
    return;
  }
  nowPlaying.classList.remove("hidden");
  npTitle.textContent = snapshot.title || snapshot.path;

  if (snapshot.path !== nowPlayingPath) {
    nowPlayingPath = snapshot.path;
    npThumb.replaceChildren();
    invoke("media_register", { path: snapshot.path })
      .then((url) => {
        if (nowPlayingPath !== snapshot.path) return;
        const kind = mediaKind(snapshot.path);
        if (kind === "audio") {
          npThumb.replaceChildren();
          return;
        }
        const thumb =
          kind === "video"
            ? Object.assign(document.createElement("video"), {
                muted: true,
                playsInline: true,
                preload: "metadata",
              })
            : document.createElement("img");
        thumb.alt = "";
        thumb.addEventListener("error", () => thumb.remove());
        thumb.src = url;
        npThumb.replaceChildren(thumb);
      })
      .catch(() => {});
  }

  if (snapshot.progress && snapshot.progress.duration > 0) {
    const { current, duration } = snapshot.progress;
    npFill.style.width = `${Math.min(100, (current / duration) * 100)}%`;
    npTime.textContent = `${fmtTime(current)} / ${fmtTime(duration)}`;
  } else if (snapshot.dwellMs > 0) {
    npFill.style.width = `${Math.min(100, (snapshot.elapsedMs / snapshot.dwellMs) * 100)}%`;
    const left = Math.max(0, snapshot.dwellMs - snapshot.elapsedMs);
    npTime.textContent = `${(left / 1000).toFixed(1)}s left \u00b7 dwell ${(snapshot.dwellMs / 1000).toFixed(1)}s`;
  } else {
    npFill.style.width = "0%";
    npTime.textContent = "";
  }
}

// ---------------------------------------------------------------------------
// Output overlay caption.
// ---------------------------------------------------------------------------

const overlayInput = document.getElementById("overlay-input");
const overlayBtn = document.getElementById("overlay-btn");

overlayBtn.addEventListener("click", async () => {
  try {
    await invoke("scheduler_set_overlay", { text: overlayInput.value, v_pos: document.getElementById("overlay-v").value, h_pos: document.getElementById("overlay-h").value });
    playbackStatus.textContent = "Overlay updated.";
    scheduleSessionSave();
  } catch (error) {
    playbackStatus.textContent = `Could not set overlay: ${String(error)}`;
  }
});
overlayInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") overlayBtn.click();
});
dwellInput.addEventListener("change", scheduleSessionSave);

volumeInput.addEventListener("input", () => {
  const level = Number(volumeInput.value) / 100;
  invoke("renderer_set_volume", { level }).catch(() => {});
});

(async function initPlayback() {
  await restoreSession();
  await pushPlaylist();

  invoke("renderer_volume")
    .then((level) => {
      volumeInput.value = String(Math.round(level * 100));
    })
    .catch(() => {});

  dropzone.addEventListener("dragover", (event) => {
    event.preventDefault();
    dropzone.classList.add("dragging");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragging"));
  dropzone.addEventListener("drop", (event) => {
    event.preventDefault();
    dropzone.classList.remove("dragging");
  });

  // Native drop paths arrive as the tauri://drag-drop event on Windows.
  listen("tauri://drag-drop", (event) => {
    const paths = event.payload && event.payload.paths;
    if (Array.isArray(paths)) addPaths(paths);
  });
  // Block the webview from navigating when files hit empty page space.
  window.addEventListener("dragover", (event) => event.preventDefault());
  window.addEventListener("drop", (event) => event.preventDefault());

  addPathBtn.addEventListener("click", () => {
    const path = pathInput.value.trim();
    if (!path) return;
    addPaths([path]);
    pathInput.value = "";
  });
  pathInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") addPathBtn.click();
  });

  outputDisplay.addEventListener("change", () => {
    const value = outputDisplay.value;
    queuedPaths.forEach((queued) => {
      queued.display = value;
    });
    renderQueue();
    pushPlaylist();
    scheduleSessionSave();
  });

  playBtn.addEventListener("click", async () => {
    const device = outputDisplay.value;
    if (!device || queuedPaths.length === 0) return;
    try {
      await invoke("scheduler_set_dwell", { millis: Number(dwellInput.value || 5) * 1000 });
      await invoke("scheduler_play", { deviceName: device });
    } catch (error) {
      playbackStatus.textContent = `Could not start playback: ${String(error)}`;
    }
  });

  previewBtn.addEventListener("click", () => {
    if (lastSnapshot && lastSnapshot.path) openPreview(lastSnapshot.path);
  });
  pauseBtn.addEventListener("click", () => invoke("scheduler_pause").catch(() => {}));
  resumeBtn.addEventListener("click", () => invoke("scheduler_resume").catch(() => {}));
  prevBtn.addEventListener("click", () => invoke("scheduler_prev").catch(() => {}));
  nextBtn.addEventListener("click", () => invoke("scheduler_next").catch(() => {}));
  stopBtn.addEventListener("click", () => invoke("scheduler_stop").catch(() => {}));

  listen("scheduler-state", (event) => {
    if (event.payload) updateSnapshot(event.payload);
  });
  try {
    const snapshot = await invoke("scheduler_status");
    updateSnapshot(snapshot);
  } catch (error) {
    playbackStatus.textContent = `Could not read playback state: ${String(error)}`;
  }
  await loadAppVersion();
  await invoke("app_ready").catch(() => {});
  checkForUpdates();
  initTabs();
  initTheme();
})();