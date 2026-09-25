const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const video = document.getElementById("preview-video");
const title = document.getElementById("preview-title");
const subtitle = document.getElementById("preview-subtitle");
const empty = document.getElementById("preview-empty");
const status = document.getElementById("preview-status");
const chooseSpeaker = document.getElementById("choose-speaker");
const closePreview = document.getElementById("close-preview");
const sinkStorageKey = "hdmi-media-controller.preview-sink";
let sinkId = "";
let sinkReady = false;
let hasMedia = false;

try {
  sinkId = window.localStorage.getItem(sinkStorageKey) || "";
} catch (_) {
  sinkId = "";
}

function setStatus(message) {
  status.textContent = message;
}

function playWhenReady() {
  if (!sinkReady || !hasMedia) return;
  video.muted = false;
  video.play().catch(() => {
    setStatus("Click Play in the preview window to start it.");
  });
}

async function applyStoredSink() {
  if (!sinkId) {
    video.muted = true;
    setStatus("Choose the laptop speaker to enable preview audio.");
    return;
  }
  if (typeof video.setSinkId !== "function") {
    video.muted = true;
    setStatus("This WebView cannot select a specific audio output.");
    return;
  }
  try {
    await video.setSinkId(sinkId);
    sinkReady = true;
    video.muted = false;
    setStatus("Preview audio is routed to the selected laptop speaker.");
    playWhenReady();
  } catch (_) {
    sinkId = "";
    sinkReady = false;
    video.muted = true;
    try {
      window.localStorage.removeItem(sinkStorageKey);
    } catch (_) {}
    setStatus("The saved laptop speaker is unavailable. Choose it again.");
  }
}

async function chooseLaptopSpeaker() {
  if (!navigator.mediaDevices || typeof navigator.mediaDevices.selectAudioOutput !== "function") {
    setStatus("This WebView cannot open the audio-output picker.");
    return;
  }
  if (typeof video.setSinkId !== "function") {
    setStatus("This WebView cannot route video audio to a selected speaker.");
    return;
  }
  try {
    const device = await navigator.mediaDevices.selectAudioOutput();
    if (!device || !device.deviceId) {
      setStatus("No laptop speaker was selected.");
      return;
    }
    await video.setSinkId(device.deviceId);
    sinkId = device.deviceId;
    sinkReady = true;
    video.muted = false;
    try {
      window.localStorage.setItem(sinkStorageKey, sinkId);
    } catch (_) {}
    subtitle.textContent = device.label || "Selected laptop speaker";
    setStatus("Preview audio is routed to the selected laptop speaker.");
    playWhenReady();
  } catch (error) {
    setStatus(`Could not select the laptop speaker: ${String(error)}`);
  }
}

async function loadPayload(payload) {
  if (!payload || !payload.mediaUrl) return;
  hasMedia = true;
  title.textContent = payload.title || "Video preview";
  document.title = `Video Preview — ${payload.title || "Clip"}`;
  subtitle.textContent = sinkReady
    ? "Preview audio is routed to the selected laptop speaker."
    : "Choose the laptop speaker to enable preview audio.";
  empty.hidden = true;
  video.src = payload.mediaUrl;
  video.load();
  if (sinkReady) {
    playWhenReady();
  } else {
    video.muted = true;
    setStatus("Choose the laptop speaker to enable preview audio.");
  }
}

chooseSpeaker.addEventListener("click", chooseLaptopSpeaker);
closePreview.addEventListener("click", async () => {
  try {
    await invoke("renderer_close_preview");
  } catch (_) {
    window.close();
  }
});
video.addEventListener("loadedmetadata", playWhenReady);
video.addEventListener("error", () => setStatus("The selected video could not be decoded."));

(async function init() {
  await listen("preview-media", (event) => {
    if (event.payload) loadPayload(event.payload);
  });
  await applyStoredSink();
  try {
    const payload = await invoke("renderer_preview_token");
    await loadPayload(payload);
  } catch (error) {
    setStatus(String(error));
  }
})();
