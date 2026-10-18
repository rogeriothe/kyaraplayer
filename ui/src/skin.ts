// Classic skin sheets: URLs, bitmap font and sprite helpers.
//
// Sprite coordinates follow the classic (2.x) skin format, as documented by the
// Webamp project (MIT, https://github.com/captbaritone/webamp).

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface PlaylistStyle {
  normal: string;
  current: string;
  normal_bg: string;
  selected_bg: string;
  font: string;
}

export interface SkinInfo {
  name: string;
  sheets: string[];
  viscolors: [number, number, number][];
  playlist: PlaylistStyle;
  has_nums_ex: boolean;
  /** Hotspot of each cursor the skin ships, by lowercase name (e.g. "titlebar"). */
  cursors: Record<string, { x: number; y: number }>;
  /** region.txt polygons by lowercase section ("normal", "windowshade", "equalizer", "equalizerws"). */
  regions: Record<string, [number, number][][]>;
  generation: number;
}

const REGION_SECTIONS = ["normal", "windowshade", "equalizer", "equalizerws"];
const SVG_NS = "http://www.w3.org/2000/svg";

export const SHEETS = [
  "main", "cbuttons", "titlebar", "shufrep", "text", "volume", "balance", "monoster",
  "playpaus", "numbers", "nums_ex", "posbar", "eqmain", "eq_ex", "pledit", "gen", "genex",
] as const;

let current: SkinInfo | null = null;

export function skin(): SkinInfo | null {
  return current;
}

/** Expose every sheet as a CSS variable, e.g. `--sheet-main: url(...)`. */
function apply(info: SkinInfo) {
  current = info;
  const root = document.documentElement.style;
  for (const sheet of SHEETS) {
    const url = `${convertFileSrc(`${sheet}.png`, "skin")}?g=${info.generation}`;
    root.setProperty(`--sheet-${sheet}`, info.sheets.includes(sheet) ? `url("${url}")` : "none");
  }
  // Digits come from nums_ex.bmp when the skin has it.
  root.setProperty("--sheet-digits", `var(--sheet-${info.has_nums_ex ? "nums_ex" : "numbers"})`);
  document.documentElement.classList.toggle("nums-ex", info.has_nums_ex);
  applyCursors(info);
  applyRegions(info);
}

let cursorVars: string[] = [];

/** `--cur-<name>` for every skin cursor; cursors.css maps them onto the controls. */
function applyCursors(info: SkinInfo) {
  const root = document.documentElement.style;
  for (const v of cursorVars) root.removeProperty(v);
  cursorVars = Object.entries(info.cursors).map(([name, { x, y }]) => {
    const url = `${convertFileSrc(`cursor/${name}.png`, "skin")}?g=${info.generation}`;
    root.setProperty(`--cur-${name}`, `url("${url}") ${x} ${y}, default`);
    return `--cur-${name}`;
  });
}

/**
 * Window shapes from region.txt as SVG clip paths (`--region-<section>`). The union
 * of the polygons stays visible; the window itself is transparent around it.
 */
function applyRegions(info: SkinInfo) {
  let svg = document.getElementById("skin-regions") as SVGSVGElement | null;
  if (!svg) {
    svg = document.createElementNS(SVG_NS, "svg") as SVGSVGElement;
    svg.id = "skin-regions";
    svg.setAttribute("width", "0");
    svg.setAttribute("height", "0");
    svg.style.position = "absolute";
    document.body.appendChild(svg);
  }
  const root = document.documentElement.style;
  const paths: Element[] = [];
  for (const section of REGION_SECTIONS) {
    const polygons = info.regions[section];
    if (!polygons?.length) {
      root.removeProperty(`--region-${section}`);
      continue;
    }
    const clip = document.createElementNS(SVG_NS, "clipPath");
    clip.id = `region-${section}`;
    for (const points of polygons) {
      const poly = document.createElementNS(SVG_NS, "polygon");
      poly.setAttribute("points", points.map(([x, y]) => `${x},${y}`).join(" "));
      clip.appendChild(poly);
    }
    paths.push(clip);
    root.setProperty(`--region-${section}`, `url(#region-${section})`);
  }
  svg.replaceChildren(...paths);
  // Only shaped skins need a see-through window; others keep the black backdrop.
  document.documentElement.classList.toggle("has-region", paths.length > 0);
}

export async function initSkin(onChange: (info: SkinInfo) => void) {
  apply(await invoke<SkinInfo>("skin_info"));
  onChange(current!);
  await listen<SkinInfo>("skin-changed", ({ payload }) => {
    apply(payload);
    onChange(payload);
  });
}

export function loadSkin(path: string | null): Promise<SkinInfo> {
  return invoke<SkinInfo>("load_skin", { path });
}

// ---------------------------------------------------------------- tiles

const images = new Map<string, Promise<HTMLImageElement>>();

function sheetImage(sheet: string): Promise<HTMLImageElement> {
  const url = `${convertFileSrc(`${sheet}.png`, "skin")}?g=${current?.generation ?? 0}`;
  let img = images.get(url);
  if (!img) {
    img = new Promise((resolve, reject) => {
      const el = new Image();
      el.crossOrigin = "anonymous";
      el.onload = () => resolve(el);
      el.onerror = reject;
      el.src = url;
    });
    images.set(url, img);
  }
  return img;
}

/**
 * Crop a rectangle of a sheet into a standalone image URL. CSS can only repeat whole
 * images, so tiled frame parts (playlist borders) need their own image.
 */
export async function cropSprite(sheet: string, x: number, y: number, w: number, h: number): Promise<string> {
  const img = await sheetImage(sheet);
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  canvas.getContext("2d")!.drawImage(img, x, y, w, h, 0, 0, w, h);
  return canvas.toDataURL();
}

// ---------------------------------------------------------------- bitmap font

const CHAR_W = 5;
const CHAR_H = 6;

/** Cell (row, column) of each character in text.bmp. */
const FONT: Record<string, [number, number]> = {};
"abcdefghijklmnopqrstuvwxyz\"@".split("").forEach((c, i) => (FONT[c] = [0, i]));
FONT[" "] = [0, 30];
"0123456789….:()-'!_+\\/[]^&%,=$#".split("").forEach((c, i) => (FONT[c] = [1, i]));
"åöä?*".split("").forEach((c, i) => (FONT[c] = [2, i]));
Object.assign(FONT, { "<": FONT["["], ">": FONT["]"], "{": FONT["["], "}": FONT["]"] });

/** Map characters the font lacks to something close, then to a space. */
function fontCell(ch: string): [number, number] {
  const lower = ch.toLowerCase();
  if (FONT[lower]) return FONT[lower];
  const plain = lower.normalize("NFD").replace(/[̀-ͯ]/g, "");
  return FONT[plain] ?? FONT[" "];
}

/** Fill `el` with one 5x6 sprite per character of `text`. */
export function renderText(el: HTMLElement, text: string) {
  const frag = document.createDocumentFragment();
  for (const ch of text) {
    const [row, col] = fontCell(ch);
    const span = document.createElement("span");
    span.className = "ch";
    span.style.backgroundPosition = `-${col * CHAR_W}px -${row * CHAR_H}px`;
    frag.appendChild(span);
  }
  el.replaceChildren(frag);
}

export const textWidth = (text: string) => [...text].length * CHAR_W;

/** Show a 9x13 digit sprite (0-9) on `el`; `null` uses the blank cell after the 9. */
export function setDigit(el: HTMLElement, digit: number | null) {
  el.style.backgroundPosition = `-${(digit ?? 10) * 9}px 0`;
}
