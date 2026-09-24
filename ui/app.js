// Control UI for the HDMI Media Controller.
//
// Talks to the Rust backend over Tauri IPC. `window.__TAURI__` is provided
// by the `app.withGlobalTauri` setting in tauri.conf.json, so no bundler or
// npm packages are required.
//
// Events: listens for `displays-changed` (emitted by the backend hotplug
// watcher) and re-enumerates the display list automatically.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const refreshBtn = document.getElementById("refresh-btn");
const listEl = document.getElementById("display-list");
const statusEl = document.getElementById("status");

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function badge(label, on) {
  return el("span", `badge ${on ? "badge-on" : "badge-off"}`, label);
}

function formatMode(mode) {
  return `${mode.width}x${mode.height} @ ${mode.refreshRate} Hz${
    mode.bitDepth ? ` (${mode.bitDepth}-bit)` : ""
  }`;
}

function buildResolutionSection(card, display) {
  const section = el("div", "resolution");
  section.append(el("div", "resolution-label", "Resolution"));

  const currentLabel = el("div", "mode-current", "Loading\u2026");
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
    cardStatus.textContent = `Applying ${formatMode(mode)}\u2026`;
    try {
      await invoke("set_display_mode", {
        deviceName: display.deviceName,
        width: mode.width,
        height: mode.height,
        refreshRate: mode.refreshRate,
      });
      cardStatus.textContent = `Applied ${formatMode(mode)}.`;
      currentLabel.textContent = `Current: ${formatMode(mode)}`;
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

function renderDisplays(displays) {
  listEl.replaceChildren();

  if (displays.length === 0) {
    listEl.append(
      el("div", "empty", "No displays detected. Is a monitor connected?")
    );
    statusEl.textContent = "No displays found.";
    return;
  }

  for (const display of displays) {
    const card = el("article", "display-card");

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
  statusEl.textContent = "Enumerating displays\u2026";

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