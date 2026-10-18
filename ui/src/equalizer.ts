import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu, Submenu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";

import { clamp, enableWindowDrag, handlePlaybackKey, type Status, type WindowsView } from "./common";
import { cropSprite, initSkin } from "./skin";

interface EqSettings {
  enabled: boolean;
  preamp_db: number;
  bands_db: number[];
  iso: boolean;
}

interface Preset {
  name: string;
  preamp_db: number;
  bands_db: number[];
}

interface EqView {
  settings: EqSettings;
  auto: boolean;
  builtin: Preset[];
  user: Preset[];
  /** Loaded by the "Auto" button when their file starts playing. */
  auto_presets: Preset[];
  current_file: string | null;
}

const MAX_DB = 12;
const TRAVEL = 51; // 62 px slider minus the 11 px thumb
const BAND_LEFT = [78, 96, 114, 132, 150, 168, 186, 204, 222, 240];

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const win = getCurrentWindow();
const root = $("eq-window");
const graph = $<HTMLCanvasElement>("eq-graph");

let view: EqView | null = null;
let settings: EqSettings = { enabled: false, preamp_db: 0, bands_db: Array(10).fill(0), iso: false };
let lineColors: string[] = [];
let graphBg: HTMLImageElement | null = null;
let preampLine: HTMLImageElement | null = null;

// ---------------------------------------------------------------- sliders

const bands: HTMLElement[] = BAND_LEFT.map((left, i) => {
  const el = document.createElement("div");
  el.className = "band ctl";
  el.dataset.band = String(i);
  el.style.left = `${left}px`;
  el.innerHTML = '<div class="thumb"></div>';
  root.appendChild(el);
  return el;
});
const preampEl = $("preamp");

function drawSlider(el: HTMLElement, db: number) {
  const frac = (clamp(db, -MAX_DB, MAX_DB) + MAX_DB) / (2 * MAX_DB); // 1 = +12 dB
  const n = Math.round(frac * 27);
  el.style.backgroundPosition = `-${13 + (n % 14) * 15}px -${164 + Math.floor(n / 14) * 65}px`;
  (el.firstElementChild as HTMLElement).style.top = `${Math.round((1 - frac) * TRAVEL)}px`;
}

function render() {
  bands.forEach((el, i) => drawSlider(el, settings.bands_db[i]));
  drawSlider(preampEl, settings.preamp_db);
  $("eq-on").classList.toggle("on", settings.enabled);
  $("eq-auto").classList.toggle("on", view?.auto ?? false);
  drawGraph();
}

function push(persist: boolean) {
  invoke("eq_set", { settings, persist });
  render();
}

function attachSlider(el: HTMLElement, get: () => number, set: (db: number) => void) {
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
    const top = el.getBoundingClientRect().top;
    const scale = el.getBoundingClientRect().height / el.offsetHeight || 1; // double size
    const thumb = el.firstElementChild as HTMLElement;
    const onThumb = e.target === thumb;
    const grab = onThumb ? (e.clientY - thumb.getBoundingClientRect().top) / scale : 5.5;
    const update = (ev: PointerEvent) => {
      const frac = 1 - clamp(((ev.clientY - top) / scale - grab) / TRAVEL, 0, 1);
      let db = Math.round((frac * 2 - 1) * MAX_DB * 10) / 10;
      if (Math.abs(db) < 0.6) db = 0; // snap to the center line
      set(db);
      push(false);
    };
    if (!onThumb) update(e);
    const up = () => {
      el.classList.remove("dragging");
      el.removeEventListener("pointermove", update);
      el.removeEventListener("pointerup", up);
      push(true);
    };
    el.addEventListener("pointermove", update);
    el.addEventListener("pointerup", up);
  });
  // Mouse wheel nudges like a fine adjustment.
  el.addEventListener("wheel", (e) => {
    e.preventDefault();
    set(clamp(get() - Math.sign(e.deltaY), -MAX_DB, MAX_DB));
    push(true);
  }, { passive: false });
}

bands.forEach((el, i) =>
  attachSlider(el, () => settings.bands_db[i], (db) => (settings.bands_db[i] = db)),
);
attachSlider(preampEl, () => settings.preamp_db, (db) => (settings.preamp_db = db));

// ---------------------------------------------------------------- graph

async function loadGraphSprites() {
  const img = (url: string) =>
    new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image();
      el.onload = () => resolve(el);
      el.onerror = reject;
      el.src = url;
    });
  graphBg = await img(await cropSprite("eqmain", 0, 294, 113, 19));
  preampLine = await img(await cropSprite("eqmain", 0, 314, 113, 1));
  // One color per graph row, taken from the skin's color column.
  const colors = await img(await cropSprite("eqmain", 115, 294, 1, 19));
  const c = document.createElement("canvas");
  c.width = 1;
  c.height = 19;
  const ctx = c.getContext("2d")!;
  ctx.drawImage(colors, 0, 0);
  const data = ctx.getImageData(0, 0, 1, 19).data;
  lineColors = Array.from({ length: 19 }, (_, y) => `rgb(${data[y * 4]},${data[y * 4 + 1]},${data[y * 4 + 2]})`);
}

/** Smooth curve through the 10 band values (Catmull-Rom). */
function curveAt(x: number): number {
  const pts = settings.bands_db;
  const step = 112 / 9;
  const i = clamp(Math.floor(x / step), 0, 8);
  const t = x / step - i;
  const p = (k: number) => pts[clamp(k, 0, 9)];
  const [p0, p1, p2, p3] = [p(i - 1), p(i), p(i + 1), p(i + 2)];
  return 0.5 * (2 * p1 + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t + (-p0 + 3 * p1 - 3 * p2 + p3) * t * t * t);
}

function drawGraph() {
  const ctx = graph.getContext("2d")!;
  ctx.clearRect(0, 0, 113, 19);
  if (graphBg) ctx.drawImage(graphBg, 0, 0);
  const yOf = (db: number) => clamp(Math.round(9 - (db / MAX_DB) * 9), 0, 18);
  if (preampLine) ctx.drawImage(preampLine, 0, yOf(settings.preamp_db));
  let prev = yOf(curveAt(0));
  for (let x = 0; x < 113; x++) {
    const y = yOf(curveAt(x));
    // Fill vertical gaps so steep slopes stay connected.
    for (let yy = Math.min(prev, y); yy <= Math.max(prev, y); yy++) {
      ctx.fillStyle = lineColors[yy] ?? "#0f0";
      ctx.fillRect(x, yy, 1, 1);
    }
    prev = y;
  }
}

// ---------------------------------------------------------------- buttons & presets

function setView(v: EqView) {
  view = v;
  settings = { ...v.settings, bands_db: [...v.settings.bands_db] };
  render();
}

$("eq-on").addEventListener("click", () => {
  settings.enabled = !settings.enabled;
  push(true);
});
$("eq-auto").addEventListener("click", async () => {
  setView(await invoke<EqView>("eq_set_auto", { on: !view?.auto }));
});
$("eq-close").addEventListener("click", () => invoke("set_window_visible", { label: "equalizer", visible: false }));

let shaded = false;
function showWindows(v: WindowsView) {
  shaded = v.eq_shade;
  document.documentElement.classList.toggle("shade", v.eq_shade);
  document.documentElement.classList.toggle("double", v.double);
}
const toggleShade = () => invoke("set_shade", { label: "equalizer", on: !shaded });
$("eq-shade").addEventListener("click", toggleShade);
$("eq-title").addEventListener("dblclick", toggleShade);
root.addEventListener("dblclick", (e) => {
  if (shaded && !(e.target as HTMLElement).closest(".ctl")) toggleShade();
});
listen<WindowsView>("windows", ({ payload }) => showWindows(payload));
invoke<WindowsView>("windows_view").then(showWindows);

async function presetsMenu() {
  if (!view) return;
  const apply = (p: Preset) => async () => setView(await invoke<EqView>("eq_apply_preset", { preset: p }));
  const items = (list: Preset[], action: (p: Preset) => () => unknown) =>
    list.map((p) => ({ text: p.name, action: () => void action(p)() }));
  const nextName = () => {
    const taken = new Set(view!.user.map((p) => p.name));
    let n = 1;
    while (taken.has(`Personalizado ${n}`)) n++;
    return `Personalizado ${n}`;
  };
  const menu = await Menu.new({
    items: [
      await Submenu.new({
        text: "Carregar",
        items: [
          ...items(view.builtin, apply),
          ...(view.user.length ? [{ item: "Separator" as const }, ...items(view.user, apply)] : []),
          { item: "Separator" as const },
          {
            text: "De arquivo (.eqf, .q1)...",
            action: async () => {
              const path = await open({ filters: [{ name: "Presets de EQ", extensions: ["eqf", "q1"] }] });
              if (typeof path === "string") setView(await invoke<EqView>("eq_import", { path }));
            },
          },
        ],
      }),
      await Submenu.new({
        text: "Salvar",
        items: [
          { text: "Como preset", action: async () => setView(await invoke<EqView>("eq_save_preset", { name: nextName() })) },
          {
            text: view.current_file ? `Automático para "${view.current_file}"` : "Automático (nada tocando)",
            enabled: !!view.current_file,
            action: async () => setView(await invoke<EqView>("eq_save_auto_preset")),
          },
          {
            text: "Para arquivo (.eqf)...",
            action: async () => {
              const path = await save({ defaultPath: "preset.eqf", filters: [{ name: "Preset de EQ", extensions: ["eqf"] }] });
              if (path) invoke("eq_export", { path });
            },
          },
        ],
      }),
      await Submenu.new({
        text: "Excluir",
        enabled: view.user.length + view.auto_presets.length > 0,
        items: [
          ...items(view.user, (p) => async () => setView(await invoke<EqView>("eq_delete_preset", { name: p.name }))),
          ...(view.user.length && view.auto_presets.length ? [{ item: "Separator" as const }] : []),
          ...items(view.auto_presets, (p) => async () =>
            setView(await invoke<EqView>("eq_delete_auto_preset", { name: p.name })),
          ).map((i) => ({ ...i, text: `Automático: ${i.text}` })),
        ],
      }),
      { item: "Separator" },
      {
        text: "Zerar",
        action: () => {
          settings.preamp_db = 0;
          settings.bands_db = Array(10).fill(0);
          push(true);
        },
      },
      {
        text: settings.iso ? "Bandas clássicas (70 Hz–16 kHz)" : "Bandas ISO (31 Hz–16 kHz)",
        action: () => {
          settings.iso = !settings.iso;
          push(true);
        },
      },
    ],
  });
  await menu.popup();
}
$("eq-presets").addEventListener("click", () => void presetsMenu());

// ---------------------------------------------------------------- window

enableWindowDrag(root, "equalizer");
win.onFocusChanged(({ payload }) => root.classList.toggle("focused", payload));
win.isFocused().then((f) => root.classList.toggle("focused", f));

let status: Status = { state: "stopped", position: 0, volume: 1, balance: 0, track: null };
window.addEventListener("keydown", async (e) => {
  status = await invoke<Status>("status");
  if (handlePlaybackKey(e, () => status)) e.preventDefault();
});
window.addEventListener("contextmenu", (e) => e.preventDefault());

initSkin(async () => {
  await loadGraphSprites();
  render();
});
listen<EqView>("eq", ({ payload }) => setView(payload));
invoke<EqView>("eq_view").then(setView);
