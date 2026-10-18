import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

import {
  APP_TITLE, SKIN_EXTENSIONS, clamp, ext, handlePlaybackKey, mmss, pickFiles, pickFolder, playPressed,
  type EngineEvent, type PlaylistView, type Status, type TrackInfo, type WindowsView,
  enableWindowDrag,
} from "./common";
import { showMainMenu } from "./mainmenu";
import { initSkin, loadSkin, renderText, setDigit, skin, textWidth, type SkinInfo } from "./skin";
import { Visualizer } from "./visualizer";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const win = getCurrentWindow();
const root = $("main-window");

let status: Status = { state: "stopped", position: 0, volume: 1, balance: 0, track: null };
let playlist: PlaylistView | null = null;
let currentTrack: TrackInfo | null = null;
let showRemaining = false;

// ---------------------------------------------------------------- marquee

const marquee = (() => {
  const box = $("marquee");
  const strip = $("marquee-text");
  const WIDTH = 154;
  let base = APP_TITLE;
  let override: string | null = null;
  let overrideTimer = 0;
  let offset = 0;
  let scrolling = false;

  function draw() {
    const text = override ?? base;
    // Long titles scroll with a separator, like the original.
    scrolling = override === null && textWidth(text) > WIDTH;
    renderText(strip, scrolling ? `${text}  ***  ${text}  ***  ` : text);
    offset = 0;
    strip.style.transform = "";
  }

  setInterval(() => {
    if (!scrolling) return;
    const loop = textWidth(`${base}  ***  `);
    offset = (offset + 5) % loop;
    strip.style.transform = `translateX(-${offset}px)`;
  }, 220);

  box.addEventListener("mousedown", (e) => e.stopPropagation());

  return {
    set(text: string) {
      base = text;
      draw();
    },
    /** Temporary message (slider feedback). `null` restores the title. */
    flash(text: string | null, ms = 1000) {
      clearTimeout(overrideTimer);
      override = text;
      draw();
      if (text !== null) {
        overrideTimer = window.setTimeout(() => {
          override = null;
          draw();
        }, ms);
      }
    },
  };
})();

// ---------------------------------------------------------------- sliders

interface SliderOptions {
  travel: number; // thumb travel in px
  thumb: number; // thumb width in px
  onInput(frac: number): void;
  onCommit?(frac: number): void;
}

function slider(el: HTMLElement, opts: SliderOptions) {
  const thumb = el.querySelector<HTMLElement>(".thumb")!;
  let dragging = false;
  let grab = opts.thumb / 2;

  // CSS px per screen px differ in double-size mode.
  const scale = () => el.getBoundingClientRect().width / el.offsetWidth || 1;
  const fracAt = (clientX: number) => {
    const x = (clientX - el.getBoundingClientRect().left) / scale() - grab;
    return clamp(x / opts.travel, 0, 1);
  };

  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || el.classList.contains("disabled")) return;
    e.stopPropagation();
    const thumbX = thumb.offsetLeft;
    const x = (e.clientX - el.getBoundingClientRect().left) / scale();
    // Grabbing the thumb keeps the grab point; clicking the track centers the thumb.
    grab = x >= thumbX && x < thumbX + opts.thumb ? x - thumbX : opts.thumb / 2;
    dragging = true;
    el.classList.add("dragging");
    el.setPointerCapture(e.pointerId);
    opts.onInput(fracAt(e.clientX));
  });
  el.addEventListener("pointermove", (e) => {
    if (dragging) opts.onInput(fracAt(e.clientX));
  });
  const end = (e: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    el.classList.remove("dragging");
    opts.onCommit?.(fracAt(e.clientX));
  };
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);

  return {
    get dragging() {
      return dragging;
    },
    set(frac: number) {
      thumb.style.left = `${Math.round(clamp(frac, 0, 1) * opts.travel)}px`;
    },
  };
}

const volumeEl = $("volume");
const balanceEl = $("balance");
const positionEl = $("position");

function drawVolume(v: number) {
  volumeSlider.set(v);
  volumeEl.style.backgroundPosition = `0 -${Math.round(v * 27) * 15}px`;
}

function drawBalance(b: number) {
  balanceSlider.set((b + 1) / 2);
  balanceEl.style.backgroundPosition = `-9px -${Math.round(Math.abs(b) * 27) * 15}px`;
}

const volumeSlider = slider(volumeEl, {
  travel: 51,
  thumb: 14,
  onInput(frac) {
    status.volume = frac;
    drawVolume(frac);
    invoke("set_volume", { volume: frac });
    marquee.flash(`Volume: ${Math.round(frac * 100)}%`);
  },
});

const balanceSlider = slider(balanceEl, {
  travel: 24,
  thumb: 14,
  onInput(frac) {
    let b = frac * 2 - 1;
    if (Math.abs(b) < 0.15) b = 0; // snap to center
    status.balance = b;
    drawBalance(b);
    invoke("set_balance", { balance: b });
    const pct = Math.round(Math.abs(b) * 100);
    marquee.flash(b === 0 ? "Balance: Center" : `Balance: ${pct}% ${b < 0 ? "Left" : "Right"}`);
  },
});

const shadePositionSlider = slider($("shade-position"), {
  travel: 14,
  thumb: 3,
  onInput(frac) {
    shadePositionSlider.set(frac);
  },
  onCommit(frac) {
    const dur = status.track?.duration;
    if (dur) invoke("seek", { secs: frac * dur });
  },
});

const positionSlider = slider(positionEl, {
  travel: 219,
  thumb: 29,
  onInput(frac) {
    const dur = status.track?.duration;
    if (!dur) return;
    positionSlider.set(frac);
    marquee.flash(`Seek to: ${mmss(frac * dur)}/${mmss(dur)} (${Math.round(frac * 100)}%)`, 60_000);
  },
  onCommit(frac) {
    const dur = status.track?.duration;
    marquee.flash(null);
    if (dur) invoke("seek", { secs: frac * dur });
  },
});

// ---------------------------------------------------------------- rendering

const digits = ["min-1", "min-2", "sec-1", "sec-2"].map((id) => $(id));
const timeEl = $("time");
const kbpsEl = $("kbps");
const khzEl = $("khz");
const visCanvas = $<HTMLCanvasElement>("visualizer");

function drawTime(st: Status) {
  const dur = st.track?.duration ?? null;
  const remaining = showRemaining && dur !== null;
  const secs = Math.floor(remaining ? dur! - st.position : st.position);
  const m = Math.min(99, Math.floor(secs / 60));
  const s = secs % 60;
  timeEl.classList.toggle("remaining", remaining);
  setDigit(digits[0], Math.floor(m / 10));
  setDigit(digits[1], m % 10);
  setDigit(digits[2], Math.floor(s / 10));
  setDigit(digits[3], s % 10);
}

function render(st: Status) {
  status = st;
  root.classList.remove("stopped", "playing", "paused");
  root.classList.add(st.state);
  drawTime(st);
  const dur = st.track?.duration;
  positionEl.classList.toggle("disabled", !dur);
  if (dur && !positionSlider.dragging) positionSlider.set(st.position / dur);
  if (dur && !shadePositionSlider.dragging) shadePositionSlider.set(st.position / dur);
  drawShadeTime(st);
}

/** Windowshade time: " m:ss" or "-m:ss" in the bitmap font. */
function drawShadeTime(st: Status) {
  if (!document.documentElement.classList.contains("shade")) return;
  const dur = st.track?.duration ?? null;
  let text = "";
  if (st.state !== "stopped") {
    const remaining = showRemaining && dur !== null;
    text = (remaining ? "-" : " ") + mmss(remaining ? dur! - st.position : st.position);
  }
  renderText($("shade-time"), text);
}

function showTrack(track: TrackInfo | null) {
  currentTrack = track;
  if (!track) {
    $("mono").classList.remove("on");
    $("stereo").classList.remove("on");
  } else {
    // The field holds 3 characters; longer values keep their leading digits.
    const kbps = track.bitrate ? String(track.bitrate).slice(0, 3) : "";
    renderText(kbpsEl, kbps.padStart(3, " "));
    renderText(khzEl, String(Math.round(track.sample_rate / 1000)).padStart(2, " ").slice(-2));
    $("mono").classList.toggle("on", track.channels === 1);
    $("stereo").classList.toggle("on", track.channels >= 2);
  }
  showTitle();
}

/** Marquee text: "N. Title (m:ss)" for the current playlist entry. */
function showTitle() {
  const list = playlist;
  const index = list?.entries.findIndex((e) => e.id === list.current) ?? -1;
  if (!list || index < 0) {
    marquee.set(currentTrack ? (currentTrack.path.split(/[\\/]/).pop() ?? "") : APP_TITLE);
    return;
  }
  const entry = list.entries[index];
  const dur = entry.duration ?? currentTrack?.duration;
  marquee.set(`${index + 1}. ${entry.title}${dur ? ` (${mmss(dur)})` : ""}`);
}

function showPlaylist(view: PlaylistView) {
  playlist = view;
  $("btn-shuffle").classList.toggle("on", view.shuffle);
  $("btn-repeat").classList.toggle("on", view.repeat);
  showTitle();
}

function showWindows(view: WindowsView) {
  windows = view;
  $("btn-eq").classList.toggle("on", view.equalizer);
  $("btn-pl").classList.toggle("on", view.playlist);
  document.documentElement.classList.toggle("shade", view.main_shade);
  document.documentElement.classList.toggle("double", view.double);
  drawShadeTime(status);
}

let windows: WindowsView | null = null;
const toggleShade = () => invoke("set_shade", { label: "main", on: !windows?.main_shade });
const toggleDouble = () => invoke("set_double", { on: !windows?.double });

const visualizer = new Visualizer(
  visCanvas,
  () => status.state,
  $<HTMLCanvasElement>("shade-vis"),
  () => document.documentElement.classList.contains("shade"),
);

function applySkinColors(info: SkinInfo) {
  visualizer.setColors(info.viscolors);
}

async function poll() {
  try {
    render(await invoke<Status>("status"));
  } catch (e) {
    console.error(e);
  }
}

// ---------------------------------------------------------------- actions

async function play() {
  if (!playlist?.entries.length) return openFile();
  return playPressed(status);
}

async function openFile() {
  const paths = await pickFiles();
  if (paths.length) invoke("pl_open", { paths });
}

async function openFolder() {
  const dir = await pickFolder();
  if (dir) invoke("pl_open", { paths: [dir] });
}

async function openSkin() {
  const picked = await open({
    multiple: false,
    filters: [{ name: "Skin clássica", extensions: SKIN_EXTENSIONS }],
  });
  if (typeof picked === "string") applySkin(picked);
}

async function applySkin(path: string | null) {
  try {
    await loadSkin(path);
  } catch (e) {
    marquee.flash(`Skin error: ${e}`, 4000);
  }
}

function seekBy(delta: number) {
  if (status.state !== "stopped") invoke("seek", { secs: Math.max(0, status.position + delta) });
}

function setVolume(v: number) {
  v = clamp(v, 0, 1);
  status.volume = v;
  drawVolume(v);
  invoke("set_volume", { volume: v });
  marquee.flash(`Volume: ${Math.round(v * 100)}%`);
}

const toggleRepeat = () => invoke("pl_set_repeat", { on: !playlist?.repeat });
const toggleShuffle = () => invoke("pl_set_shuffle", { on: !playlist?.shuffle });
const toggleWindow = (label: "equalizer" | "playlist", button: string) =>
  invoke<WindowsView>("set_window_visible", { label, visible: !$(button).classList.contains("on") });
const togglePlaylist = () => toggleWindow("playlist", "btn-pl");
const toggleEqualizer = () => toggleWindow("equalizer", "btn-eq");
const openUrl = () => void invoke("open_url_dialog").catch((e) => marquee.flash(`Error: ${e}`, 4000));
const fileInfo = () => void invoke("pl_file_info", { id: null }).catch((e) => marquee.flash(`${e}`, 3000));
const toggleAlwaysOnTop = () => invoke("set_always_on_top", { on: !windows?.always_on_top });
const about = () => marquee.flash(`${APP_TITLE} - skin: ${skin()?.name ?? "?"}`, 3000);

function setRemaining(on: boolean) {
  showRemaining = on;
  drawTime(status);
  drawShadeTime(status);
}

async function openMenu() {
  const globalHotkeys = await invoke<boolean>("global_hotkeys").catch(() => false);
  showMainMenu(
    {
      globalHotkeys,
      equalizer: !!windows?.equalizer,
      playlist: !!windows?.playlist,
      double: !!windows?.double,
      alwaysOnTop: !!windows?.always_on_top,
      showRemaining,
      repeat: !!playlist?.repeat,
      shuffle: !!playlist?.shuffle,
      hasSkin: skin()?.name !== "Kyara Default",
    },
    {
      about,
      openFile: () => void openFile(),
      openFolder: () => void openFolder(),
      openUrl,
      fileInfo,
      toggleEqualizer: () => void toggleEqualizer(),
      togglePlaylist: () => void togglePlaylist(),
      loadSkin: () => void openSkin(),
      defaultSkin: () => void applySkin(null),
      toggleDouble: () => void toggleDouble(),
      toggleAlwaysOnTop: () => void toggleAlwaysOnTop(),
      toggleGlobalHotkeys: () => void invoke("set_global_hotkeys", { on: !globalHotkeys }),
      setRemaining,
      toggleRepeat: () => void toggleRepeat(),
      toggleShuffle: () => void toggleShuffle(),
      prev: () => void invoke("pl_prev"),
      play: () => void play(),
      pause: () => void invoke("toggle_pause"),
      stop: () => void invoke("stop"),
      next: () => void invoke("pl_next"),
      seekBy,
      jump: (delta) => void invoke("pl_jump", { delta }),
      exit: () => void win.close(),
    },
  ).catch((e) => console.error("menu:", e));
}

// ---------------------------------------------------------------- wiring

const click = (id: string, fn: () => unknown) => $(id).addEventListener("click", () => void fn());

click("btn-play", play);
click("btn-pause", () => invoke("toggle_pause"));
click("btn-stop", () => invoke("stop"));
click("btn-prev", () => invoke("pl_prev"));
click("btn-next", () => invoke("pl_next"));
click("btn-eject", openFile);
click("btn-repeat", toggleRepeat);
click("btn-shuffle", toggleShuffle);
click("btn-eq", toggleEqualizer);
click("btn-pl", togglePlaylist);
click("btn-minimize", () => win.minimize());
click("btn-shade", toggleShade);
$("titlebar").addEventListener("dblclick", (e) => {
  if (!(e.target as HTMLElement).closest(".ctl")) toggleShade();
});
document.querySelectorAll<HTMLElement>("#shade-controls > div").forEach((el) => {
  const actions: Record<string, () => unknown> = {
    prev: () => invoke("pl_prev"),
    play,
    pause: () => invoke("toggle_pause"),
    stop: () => invoke("stop"),
    next: () => invoke("pl_next"),
    eject: openFile,
  };
  el.addEventListener("click", () => void actions[el.dataset.action!]());
});
click("btn-close", () => win.close());
click("btn-options", openMenu);
click("about", about);
click("time", () => setRemaining(!showRemaining));

// Any non-control area drags the window.
enableWindowDrag(root, "main");

win.onFocusChanged(({ payload }) => root.classList.toggle("focused", payload));
win.isFocused().then((f) => root.classList.toggle("focused", f));

listen<EngineEvent>("engine", ({ payload }) => {
  // Work indicator + message while a stream connects, like the original's buffering.
  const connecting = payload.type === "connecting";
  const wasConnecting = root.classList.contains("connecting");
  root.classList.toggle("connecting", connecting);
  if (connecting) marquee.flash("Conectando...", 30_000);
  else if (wasConnecting && payload.type !== "error") marquee.flash(null);
  if (payload.type === "trackStarted") showTrack(payload.track);
  if (payload.type === "error") marquee.flash(`Error: ${payload.message}`, 4000);
  poll();
});
// System-wide Ctrl+Alt hotkeys (registered by the backend).
listen<string>("hotkey", ({ payload }) => {
  const actions: Record<string, () => unknown> = {
    play,
    pause: () => invoke("toggle_pause"),
    stop: () => invoke("stop"),
    prev: () => invoke("pl_prev"),
    next: () => invoke("pl_next"),
    "volume-up": () => setVolume(status.volume + 0.02),
    "volume-down": () => setVolume(status.volume - 0.02),
    back: () => seekBy(-5),
    forward: () => seekBy(5),
  };
  void actions[payload]?.();
});
listen<PlaylistView>("playlist", ({ payload }) => showPlaylist(payload));
listen<WindowsView>("windows", ({ payload }) => showWindows(payload));

getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (payload.type !== "drop" || payload.paths.length === 0) return;
  const [first] = payload.paths;
  if (SKIN_EXTENSIONS.includes(ext(first))) applySkin(first);
  else invoke("pl_open", { paths: payload.paths });
});

window.addEventListener("keydown", (e) => {
  const k = e.key.toLowerCase();
  if (handlePlaybackKey(e, () => status)) {
    // handled
  } else if (e.altKey && k === "s") openSkin();
  else if (e.altKey && k === "e") togglePlaylist();
  else if (e.altKey && k === "g") toggleEqualizer();
  else if (e.ctrlKey && k === "t") $("time").click();
  else if (e.ctrlKey && k === "d") toggleDouble();
  else if (e.ctrlKey && k === "w") toggleShade();
  else if (e.ctrlKey && k === "a") toggleAlwaysOnTop();
  else if (e.ctrlKey && k === "l") openUrl();
  else if (e.altKey && e.code === "Digit3") fileInfo();
  else if (e.ctrlKey || e.altKey || e.metaKey) return;
  else if (k === "l" && e.shiftKey) openFolder();
  else if (k === "l") openFile();
  else if (k === "r") toggleRepeat();
  else if (k === "s") toggleShuffle();
  else if (k === "arrowleft") seekBy(-5);
  else if (k === "arrowright") seekBy(5);
  else if (k === "arrowup") setVolume(status.volume + 0.02);
  else if (k === "arrowdown") setVolume(status.volume - 0.02);
  else return;
  e.preventDefault();
});

window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void openMenu();
});

initSkin(applySkinColors);
visualizer.start();
invoke<PlaylistView>("pl_view").then(showPlaylist);
invoke<WindowsView>("windows_view").then(showWindows);
invoke<Status>("status").then((st) => {
  render(st);
  drawVolume(st.volume);
  drawBalance(st.balance);
  showTrack(st.track);
});
setInterval(poll, 100);
