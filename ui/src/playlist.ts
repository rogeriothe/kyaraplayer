import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";

import {
  PLAYLIST_EXTENSIONS, SKIN_EXTENSIONS, clamp, ext, handlePlaybackKey, hmmss, mmss, pickFiles, type WindowsView,
  pickFolder, playPressed, type PlaylistView, type Status,
  enableWindowDrag,
} from "./common";
import { cropSprite, initSkin, renderText, type SkinInfo } from "./skin";

const ROW_H = 13;
const LIST_PAD = 3;

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const win = getCurrentWindow();
const root = $("playlist-window");
const listEl = $("pl-list");
const scrollEl = $("pl-scroll");
const handleEl = $("pl-handle");

let view: PlaylistView = { entries: [], current: null, shuffle: false, repeat: false, total: 0, total_complete: true };
let status: Status = { state: "stopped", position: 0, volume: 1, balance: 0, track: null };
const selected = new Set<number>();
let anchor = 0; // index for shift-selection and keyboard focus
let scroll = 0; // first visible row

// ---------------------------------------------------------------- skin

async function applySkin(info: SkinInfo) {
  const s = document.documentElement.style;
  const tile = async (name: string, x: number, y: number, w: number, h: number) =>
    s.setProperty(name, `url("${await cropSprite("pledit", x, y, w, h)}")`);
  await Promise.all([
    tile("--pl-top-tile", 127, 21, 25, 20),
    tile("--pl-top-tile-focused", 127, 0, 25, 20),
    tile("--pl-left-tile", 0, 42, 12, 29),
    tile("--pl-right-tile", 31, 42, 20, 29),
    tile("--pl-bottom-tile", 179, 0, 25, 38),
    tile("--pl-shade-tile", 72, 57, 25, 14),
  ]);
  const p = info.playlist;
  s.setProperty("--pl-normal", p.normal);
  s.setProperty("--pl-current", p.current);
  s.setProperty("--pl-normal-bg", p.normal_bg);
  s.setProperty("--pl-selected-bg", p.selected_bg);
  s.setProperty("--pl-font", JSON.stringify(p.font));
}

// ---------------------------------------------------------------- list rendering

const visibleRows = () => Math.max(1, Math.floor((listEl.clientHeight - LIST_PAD * 2) / ROW_H));
const maxScroll = () => Math.max(0, view.entries.length - visibleRows());

function setScroll(row: number) {
  scroll = clamp(Math.round(row), 0, maxScroll());
  renderList();
}

/** Scroll the minimum needed to show `index`. */
function reveal(index: number) {
  if (index < scroll) setScroll(index);
  else if (index >= scroll + visibleRows()) setScroll(index - visibleRows() + 1);
}

function renderList() {
  scroll = clamp(scroll, 0, maxScroll());
  const rows = visibleRows();
  const frag = document.createDocumentFragment();
  const end = Math.min(view.entries.length, scroll + rows + 1);
  for (let i = scroll; i < end; i++) {
    const e = view.entries[i];
    const row = document.createElement("div");
    row.className = "row";
    row.dataset.index = String(i);
    if (selected.has(e.id)) row.classList.add("selected");
    if (e.id === view.current) row.classList.add("current");
    const title = document.createElement("span");
    title.className = "title";
    title.textContent = `${i + 1}. ${e.title}`;
    const dur = document.createElement("span");
    dur.className = "dur";
    dur.textContent = e.duration != null ? mmss(e.duration) : "";
    row.append(title, dur);
    frag.appendChild(row);
  }
  listEl.replaceChildren(frag);

  // Scroll handle.
  const track = scrollEl.clientHeight - handleEl.offsetHeight;
  const max = maxScroll();
  handleEl.style.top = `${max ? Math.round((scroll / max) * track) : 0}px`;

  renderRunningTime();
}

function renderRunningTime() {
  const sel = view.entries.filter((e) => selected.has(e.id));
  const selTotal = sel.reduce((t, e) => t + (e.duration ?? 0), 0);
  const plus = view.total_complete ? "" : "+";
  renderText($("pl-running"), `${hmmss(selTotal)}/${hmmss(view.total)}${plus}`);
}

function renderMiniTime() {
  const text = status.state === "stopped" ? "" : mmss(status.position);
  renderText($("pl-minitime"), text.padStart(5, " "));
  if (shaded) {
    const index = view.entries.findIndex((e) => e.id === view.current);
    const entry = view.entries[index];
    renderText($("pl-shade-title"), entry ? `${index + 1}. ${entry.title}` : "");
    renderText($("pl-shade-time"), entry?.duration != null ? mmss(entry.duration) : "");
  }
}

let shaded = false;
function showWindows(v: WindowsView) {
  shaded = v.pl_shade;
  document.documentElement.classList.toggle("shade", v.pl_shade);
  renderMiniTime();
}
const toggleShade = () => invoke("set_shade", { label: "playlist", on: !shaded });

function setView(v: PlaylistView) {
  view = v;
  const ids = new Set(v.entries.map((e) => e.id));
  for (const id of [...selected]) if (!ids.has(id)) selected.delete(id);
  anchor = clamp(anchor, 0, Math.max(0, v.entries.length - 1));
  renderList();
}

// ---------------------------------------------------------------- selection & mouse

const rowAt = (clientY: number) => {
  const y = clientY - listEl.getBoundingClientRect().top - LIST_PAD;
  return scroll + Math.floor(y / ROW_H);
};

function selectRange(a: number, b: number, add: boolean) {
  if (!add) selected.clear();
  for (let i = Math.min(a, b); i <= Math.max(a, b); i++) {
    const e = view.entries[i];
    if (e) selected.add(e.id);
  }
}

let dragRows: { startRow: number; moved: boolean } | null = null;

listEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  listEl.focus();
  const index = rowAt(e.clientY);
  const entry = view.entries[index];
  if (!entry) {
    if (!e.ctrlKey && !e.shiftKey) selected.clear();
    renderList();
    return;
  }
  if (e.shiftKey) {
    selectRange(anchor, index, e.ctrlKey);
  } else if (e.ctrlKey) {
    if (selected.has(entry.id)) selected.delete(entry.id);
    else selected.add(entry.id);
    anchor = index;
  } else {
    if (!selected.has(entry.id)) {
      selected.clear();
      selected.add(entry.id);
    }
    anchor = index;
    // Dragging a selection moves it.
    dragRows = { startRow: index, moved: false };
    listEl.setPointerCapture(e.pointerId);
  }
  renderList();
});

listEl.addEventListener("pointermove", (e) => {
  if (!dragRows) return;
  const row = clamp(rowAt(e.clientY), 0, view.entries.length - 1);
  const delta = row - dragRows.startRow;
  if (delta === 0) return;
  // Move the selection live, like the original.
  const indices = view.entries.flatMap((x, i) => (selected.has(x.id) ? [i] : []));
  if (indices[0] + delta < 0 || indices[indices.length - 1] + delta >= view.entries.length) return;
  const ids = indices.map((i) => view.entries[i].id);
  const target = delta < 0 ? indices[0] + delta : indices[indices.length - 1] + delta + 1;
  dragRows = { startRow: row, moved: true };
  anchor = clamp(anchor + delta, 0, view.entries.length - 1);
  invoke("pl_move", { ids, to: target });
  if (row < scroll) setScroll(scroll - 1);
  else if (row >= scroll + visibleRows()) setScroll(scroll + 1);
});

const endDrag = (e: PointerEvent) => {
  if (!dragRows) return;
  // A plain click on an already-selected row selects just that row.
  if (!dragRows.moved) {
    const entry = view.entries[rowAt(e.clientY)];
    if (entry) {
      selected.clear();
      selected.add(entry.id);
      renderList();
    }
  }
  dragRows = null;
};
listEl.addEventListener("pointerup", endDrag);
listEl.addEventListener("pointercancel", endDrag);

listEl.addEventListener("dblclick", (e) => {
  const entry = view.entries[rowAt(e.clientY)];
  if (entry) invoke("pl_play", { id: entry.id });
});

listEl.addEventListener("wheel", (e) => {
  e.preventDefault();
  setScroll(scroll + Math.sign(e.deltaY) * 3);
}, { passive: false });

// Scrollbar dragging.
scrollEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  scrollEl.classList.add("dragging");
  scrollEl.setPointerCapture(e.pointerId);
  const track = scrollEl.clientHeight - handleEl.offsetHeight;
  const top = scrollEl.getBoundingClientRect().top;
  const onTarget = e.target === handleEl;
  const grab = onTarget ? e.clientY - handleEl.getBoundingClientRect().top : handleEl.offsetHeight / 2;
  const move = (ev: PointerEvent) => {
    const frac = track > 0 ? clamp((ev.clientY - top - grab) / track, 0, 1) : 0;
    setScroll(frac * maxScroll());
  };
  move(e);
  const up = () => {
    scrollEl.classList.remove("dragging");
    scrollEl.removeEventListener("pointermove", move);
    scrollEl.removeEventListener("pointerup", up);
  };
  scrollEl.addEventListener("pointermove", move);
  scrollEl.addEventListener("pointerup", up);
});

// ---------------------------------------------------------------- actions

const selectedIds = () => view.entries.filter((e) => selected.has(e.id)).map((e) => e.id);

/** File info for the first selected entry, else the current one. */
function fileInfo() {
  const id = view.entries.find((e) => selected.has(e.id))?.id ?? null;
  void invoke("pl_file_info", { id });
}

async function addFiles() {
  const paths = await pickFiles();
  if (paths.length) invoke("pl_add", { paths });
}

async function addFolder() {
  const dir = await pickFolder();
  if (dir) invoke("pl_add", { paths: [dir] });
}

async function loadList() {
  const picked = await open({ multiple: false, filters: [{ name: "Playlist", extensions: PLAYLIST_EXTENSIONS }] });
  if (typeof picked === "string") invoke("pl_load", { path: picked });
}

async function saveList() {
  const path = await save({
    defaultPath: "playlist.m3u8",
    filters: [
      { name: "M3U8", extensions: ["m3u8"] },
      { name: "M3U", extensions: ["m3u"] },
      { name: "PLS", extensions: ["pls"] },
    ],
  });
  if (path) invoke("pl_save", { path });
}

async function popup(items: [string, () => unknown][]) {
  const menu = await Menu.new({ items: items.map(([text, action]) => ({ text, action: () => void action() })) });
  await menu.popup();
}

function selectAll(on: boolean) {
  selected.clear();
  if (on) view.entries.forEach((e) => selected.add(e.id));
  renderList();
}

function invertSelection() {
  for (const e of view.entries) {
    if (selected.has(e.id)) selected.delete(e.id);
    else selected.add(e.id);
  }
  renderList();
}

interface MenuItem {
  x: number;
  y: number;
  action: () => unknown;
}

/** Sprite menus: items listed top to bottom; the bottom item sits on the button. */
const MENUS: Record<string, { bar: [number, number, number]; items: MenuItem[] }> = {
  add: {
    bar: [48, 111, 54],
    items: [
      { x: 0, y: 111, action: () => invoke("open_url_dialog") },
      { x: 0, y: 130, action: addFolder },
      { x: 0, y: 149, action: addFiles },
    ],
  },
  rem: {
    bar: [100, 111, 72],
    items: [
      {
        x: 54, y: 168,
        action: () => popup([
          ["Remover arquivos inexistentes", () => invoke("pl_remove_misc", { kind: "dead" })],
          ["Remover duplicados", () => invoke("pl_remove_misc", { kind: "duplicates" })],
        ]),
      },
      { x: 54, y: 111, action: () => invoke("pl_clear") },
      { x: 54, y: 130, action: () => invoke("pl_crop", { ids: selectedIds() }) },
      { x: 54, y: 149, action: () => invoke("pl_remove", { ids: selectedIds() }) },
    ],
  },
  sel: {
    bar: [150, 111, 54],
    items: [
      { x: 104, y: 111, action: invertSelection },
      { x: 104, y: 130, action: () => selectAll(false) },
      { x: 104, y: 149, action: () => selectAll(true) },
    ],
  },
  misc: {
    bar: [200, 111, 54],
    items: [
      {
        x: 154, y: 111,
        action: () => popup([
          ["Ordenar por título", () => invoke("pl_sort", { kind: "title" })],
          ["Ordenar por nome de arquivo", () => invoke("pl_sort", { kind: "filename" })],
          ["Ordenar por caminho", () => invoke("pl_sort", { kind: "path" })],
          ["Inverter lista", () => invoke("pl_sort", { kind: "reverse" })],
          ["Embaralhar lista", () => invoke("pl_sort", { kind: "random" })],
        ]),
      },
      { x: 154, y: 130, action: () => fileInfo() },
      { x: 154, y: 149, action: () => undefined }, // misc options: later milestone
    ],
  },
  list: {
    bar: [250, 111, 54],
    items: [
      { x: 204, y: 111, action: () => invoke("pl_clear") },
      { x: 204, y: 130, action: saveList },
      { x: 204, y: 149, action: loadList },
    ],
  },
};

const menuEl = $("pl-menu");
let openMenu: { name: string; hover: number } | null = null;

function drawMenu() {
  if (!openMenu) {
    menuEl.classList.remove("open");
    return;
  }
  const def = MENUS[openMenu.name];
  const btn = document.querySelector<HTMLElement>(`[data-menu="${openMenu.name}"]`)!;
  const rect = btn.getBoundingClientRect();
  menuEl.style.left = `${rect.left}px`;
  menuEl.style.top = `${rect.bottom - def.items.length * 18}px`;
  const [bx, by, bh] = def.bar;
  const bar = `<div class="bar" style="height:${bh}px;background-position:-${bx}px -${by}px"></div>`;
  menuEl.innerHTML = bar + def.items
    .map((it, i) => {
      const x = i === openMenu!.hover ? it.x + 23 : it.x;
      return `<div class="item" data-i="${i}" style="background-position:-${x}px -${it.y}px"></div>`;
    })
    .join("");
  menuEl.classList.add("open");
}

function menuItemAt(x: number, y: number): number {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  return el?.classList.contains("item") ? Number(el.dataset.i) : -1;
}

document.querySelectorAll<HTMLElement>(".pl-menu-btn").forEach((btn) => {
  btn.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    const name = btn.dataset.menu!;
    openMenu = { name, hover: MENUS[name].items.length - 1 };
    drawMenu();
    // Press, slide to an item, release: like the original.
    const move = (ev: PointerEvent) => {
      if (!openMenu) return;
      openMenu.hover = menuItemAt(ev.clientX, ev.clientY);
      drawMenu();
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const i = menuItemAt(ev.clientX, ev.clientY);
      const menu = openMenu;
      openMenu = null;
      drawMenu();
      if (menu && i >= 0) void MENUS[menu.name].items[i].action();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
});

// Mini transport buttons.
const ACTIONS: Record<string, () => unknown> = {
  prev: () => invoke("pl_prev"),
  play: () => playPressed(status),
  pause: () => invoke("toggle_pause"),
  stop: () => invoke("stop"),
  next: () => invoke("pl_next"),
  eject: async () => {
    const paths = await pickFiles();
    if (paths.length) invoke("pl_open", { paths });
  },
};
document.querySelectorAll<HTMLElement>("#pl-actions > div").forEach((el) => {
  el.addEventListener("click", () => void ACTIONS[el.dataset.action!]());
});
$("pl-scroll-up").addEventListener("click", () => setScroll(scroll - 1));
$("pl-scroll-down").addEventListener("click", () => setScroll(scroll + 1));

// ---------------------------------------------------------------- window

$("pl-close").addEventListener("click", () => invoke("set_window_visible", { label: "playlist", visible: false }));
$("pl-shade-close").addEventListener("click", () => invoke("set_window_visible", { label: "playlist", visible: false }));
$("pl-shade").addEventListener("click", toggleShade);
$("pl-unshade").addEventListener("click", toggleShade);
root.addEventListener("dblclick", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest(".ctl") && (shaded || t.closest(".pl-top"))) toggleShade();
});
listen<WindowsView>("windows", ({ payload }) => showWindows(payload));
invoke<WindowsView>("windows_view").then(showWindows);

enableWindowDrag(root, "playlist");

// Resize in 25x29 steps from the bottom-right grip.
$("pl-resize").addEventListener("pointerdown", async (e) => {
  e.stopPropagation();
  const el = e.currentTarget as HTMLElement;
  el.setPointerCapture(e.pointerId);
  const startW = window.innerWidth;
  const startH = window.innerHeight;
  const sx = e.screenX;
  const sy = e.screenY;
  let size = [startW, startH];
  const move = (ev: PointerEvent) => {
    const w = 275 + Math.max(0, Math.round((startW + ev.screenX - sx - 275) / 25)) * 25;
    const h = 116 + Math.max(0, Math.round((startH + ev.screenY - sy - 116) / 29)) * 29;
    if (w === size[0] && h === size[1]) return;
    size = [w, h];
    win.setSize(new LogicalSize(w, h));
  };
  const up = () => {
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", up);
    invoke("save_playlist_size", { width: size[0], height: size[1] });
  };
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", up);
});

new ResizeObserver(() => {
  root.classList.toggle("wide", window.innerWidth >= 350);
  renderList();
}).observe(listEl);

win.onFocusChanged(({ payload }) => root.classList.toggle("focused", payload));

// Files dropped on the list are inserted where they land.
getCurrentWebview().onDragDropEvent(async ({ payload }) => {
  if (payload.type === "over" || payload.type === "enter") listEl.classList.add("drop-target");
  if (payload.type === "leave") listEl.classList.remove("drop-target");
  if (payload.type !== "drop") return;
  listEl.classList.remove("drop-target");
  const paths = payload.paths.filter((p) => !SKIN_EXTENSIONS.includes(ext(p)));
  if (!paths.length) return;
  const y = payload.position.y / (await win.scaleFactor());
  const at = clamp(rowAt(y), 0, view.entries.length);
  invoke("pl_add", { paths, at });
});

// ---------------------------------------------------------------- keyboard

listEl.addEventListener("keydown", (e) => {
  const n = view.entries.length;
  const move = (to: number) => {
    to = clamp(to, 0, n - 1);
    if (e.shiftKey) selectRange(anchor, to, false);
    else {
      selected.clear();
      if (view.entries[to]) selected.add(view.entries[to].id);
    }
    if (!e.shiftKey) anchor = to;
    reveal(to);
    renderList();
  };
  const k = e.key;
  if (k === "Delete") invoke("pl_remove", { ids: selectedIds() });
  else if (k === "Enter") { const id = selectedIds()[0]; if (id !== undefined) invoke("pl_play", { id }); }
  else if (k === "ArrowDown") move(anchor + 1);
  else if (k === "ArrowUp") move(anchor - 1);
  else if (k === "PageDown") move(anchor + visibleRows());
  else if (k === "PageUp") move(anchor - visibleRows());
  else if (k === "Home") move(0);
  else if (k === "End") move(n - 1);
  else if (e.ctrlKey && k.toLowerCase() === "a") selectAll(true);
  else if (e.ctrlKey && k.toLowerCase() === "i") invertSelection();
  else if (e.altKey && e.code === "Digit3") fileInfo();
  else if (handlePlaybackKey(e, () => status)) { /* handled */ }
  else return;
  e.preventDefault();
});

window.addEventListener("contextmenu", (e) => e.preventDefault());

// ---------------------------------------------------------------- startup

initSkin(applySkin);
listen<PlaylistView>("playlist", ({ payload }) => setView(payload));
invoke<PlaylistView>("pl_view").then(setView);
win.isFocused().then((f) => root.classList.toggle("focused", f));
setInterval(async () => {
  status = await invoke<Status>("status");
  renderMiniTime();
}, 250);
