// Control UI for the HDMI Media Controller.
//
// Talks to the Rust backend over Tauri IPC. `window.__TAURI__` is provided
// by the `app.withGlobalTauri` setting in tauri.conf.json, so no bundler or
// npm packages are required.

const { invoke } = window.__TAURI__.core;

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

    card.append(title, meta, badges);
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
loadDisplays();