// Classic visualizer: spectrum analyzer or oscilloscope in the 76x16 main window
// area (38x5 in windowshade mode), colored by the skin's viscolor.txt.
//
// Styles follow the original's options: analyzer normal/fire/line, thick or thin
// bands, falling peaks; oscilloscope dots/lines/solid.

import { invoke } from "@tauri-apps/api/core";
import { Menu } from "@tauri-apps/api/menu";

import type { PlayerState } from "./common";

type Rgb = [number, number, number];
export type VisMode = "spectrum" | "scope" | "off";
type AnalyzerStyle = "normal" | "fire" | "line";
type ScopeStyle = "dots" | "lines" | "solid";

interface VisFrame {
  spectrum: number[]; // 75 columns, 0..255
  wave: number[]; // 75 points, 0..255, 128 = center
}

interface Options {
  mode: VisMode;
  analyzer: AnalyzerStyle;
  thin: boolean;
  peaks: boolean;
  scope: ScopeStyle;
}

const W = 76;
const H = 16;
const SHADE_W = 38;
const SHADE_H = 5;
const BAR_FALL = 40; // px per second (16 px scale)
const PEAK_HOLD = 0.4; // seconds
const PEAK_FALL = 14; // px per second
const MODES: VisMode[] = ["spectrum", "scope", "off"];
const STORAGE_KEY = "kyara.vis";
const DEFAULTS: Options = { mode: "spectrum", analyzer: "normal", thin: false, peaks: true, scope: "lines" };

export class Visualizer {
  private ctx: CanvasRenderingContext2D;
  private shadeCtx: CanvasRenderingContext2D | null;
  private colors: string[] = Array(24).fill("#000");
  /** Bar heights and peaks on a 0..H scale, one per analyzer column (up to 75). */
  private bars = new Float32Array(75);
  private peaks = new Float32Array(75);
  private peakAge = new Float32Array(75);
  private last = performance.now();
  private running = false;
  private opts: Options = { ...DEFAULTS };

  constructor(
    canvas: HTMLCanvasElement,
    private state: () => PlayerState,
    shadeCanvas?: HTMLCanvasElement,
    private shaded: () => boolean = () => false,
  ) {
    this.ctx = canvas.getContext("2d")!;
    this.shadeCtx = shadeCanvas?.getContext("2d") ?? null;
    try {
      const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as Partial<Options> | null;
      if (saved) this.opts = { ...DEFAULTS, ...saved };
      if (!MODES.includes(this.opts.mode)) this.opts.mode = "spectrum";
    } catch {
      // storage unavailable or corrupt: keep the defaults
    }
    for (const c of [canvas, shadeCanvas]) {
      c?.addEventListener("click", () => this.cycle());
      c?.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        e.stopPropagation(); // not the main window menu
        void this.showMenu();
      });
    }
  }

  get mode(): VisMode {
    return this.opts.mode;
  }

  setColors(viscolors: Rgb[]) {
    this.colors = viscolors.map(([r, g, b]) => `rgb(${r},${g},${b})`);
    this.drawBackground();
  }

  private set(changes: Partial<Options>) {
    this.opts = { ...this.opts, ...changes };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.opts));
    } catch {
      // not persisted; fine
    }
    this.bars.fill(0);
    this.peaks.fill(0);
    this.drawBackground();
  }

  cycle() {
    this.set({ mode: MODES[(MODES.indexOf(this.opts.mode) + 1) % MODES.length] });
  }

  private async showMenu() {
    const o = this.opts;
    const check = (text: string, checked: boolean, changes: Partial<Options>) => ({
      text,
      checked,
      action: () => this.set(changes),
    });
    const menu = await Menu.new({
      items: [
        check("Analisador de espectro", o.mode === "spectrum", { mode: "spectrum" }),
        check("Osciloscópio", o.mode === "scope", { mode: "scope" }),
        check("Desligado", o.mode === "off", { mode: "off" }),
        { item: "Separator" },
        {
          text: "Analisador",
          items: [
            check("Normal", o.analyzer === "normal", { analyzer: "normal" }),
            check("Fogo", o.analyzer === "fire", { analyzer: "fire" }),
            check("Linha", o.analyzer === "line", { analyzer: "line" }),
            { item: "Separator" },
            check("Barras grossas", !o.thin, { thin: false }),
            check("Barras finas", o.thin, { thin: true }),
            { item: "Separator" },
            check("Mostrar picos", o.peaks, { peaks: !o.peaks }),
          ],
        },
        {
          text: "Osciloscópio",
          items: [
            check("Pontos", o.scope === "dots", { scope: "dots" }),
            check("Linhas", o.scope === "lines", { scope: "lines" }),
            check("Sólido", o.scope === "solid", { scope: "solid" }),
          ],
        },
      ],
    });
    await menu.popup();
  }

  start() {
    if (this.running) return;
    this.running = true;
    const loop = async () => {
      await this.tick();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  private async tick() {
    const now = performance.now();
    const dt = Math.min(0.1, (now - this.last) / 1000);
    this.last = now;
    const state = this.state();
    if (this.opts.mode === "off" || state === "stopped") {
      this.bars.fill(0);
      this.peaks.fill(0);
      this.drawBackground();
      return;
    }
    // Paused: no new audio, let the bars fall.
    const frame =
      state === "playing"
        ? await invoke<VisFrame>("vis_frame").catch(() => null)
        : { spectrum: Array(75).fill(0), wave: Array(75).fill(128) };
    if (!frame) return;
    this.drawBackground();
    if (this.opts.mode === "spectrum") {
      this.updateBars(frame.spectrum, dt);
      this.drawSpectrum();
      this.drawShadeSpectrum();
    } else {
      this.drawScope(frame.wave);
      this.drawShadeScope(frame.wave);
    }
  }

  private drawBackground() {
    const { ctx } = this;
    ctx.fillStyle = this.colors[0];
    ctx.fillRect(0, 0, W, H);
    ctx.fillStyle = this.colors[1];
    for (let y = 1; y < H; y += 2) for (let x = 1; x < W; x += 2) ctx.fillRect(x, y, 1, 1);
    if (this.shadeCtx) {
      this.shadeCtx.fillStyle = this.colors[0];
      this.shadeCtx.fillRect(0, 0, SHADE_W, SHADE_H);
    }
  }

  /** Thick: 19 bands of 4 columns (3 px + gap). Thin: all 75 columns. */
  private columns() {
    return this.opts.thin ? { count: 75, step: 1, width: 1 } : { count: 19, step: 4, width: 3 };
  }

  private updateBars(spectrum: number[], dt: number) {
    const { count, step } = this.columns();
    for (let i = 0; i < count; i++) {
      const v = Math.max(...spectrum.slice(i * step, i * step + step)) / 255;
      this.bars[i] = Math.max(v * H, this.bars[i] - BAR_FALL * dt);
      if (this.bars[i] >= this.peaks[i]) {
        this.peaks[i] = this.bars[i];
        this.peakAge[i] = 0;
      } else {
        this.peakAge[i] += dt;
        if (this.peakAge[i] > PEAK_HOLD) this.peaks[i] = Math.max(0, this.peaks[i] - PEAK_FALL * dt);
      }
    }
  }

  /**
   * Row color for a bar of height `h` at row `y` (0 = top). viscolor 2 is the top row,
   * 17 the bottom one. Fire colors from the bar's top down; line paints the whole bar
   * with the color of its top.
   */
  private barColor(y: number, h: number, height: number): string {
    const top = height - h;
    const scale = (n: number) => Math.round((n * 15) / (height - 1 || 1));
    switch (this.opts.analyzer) {
      case "fire":
        return this.colors[2 + scale(y - top)];
      case "line":
        return this.colors[2 + scale(top)];
      default:
        return this.colors[2 + scale(y)];
    }
  }

  private drawSpectrum() {
    const { ctx } = this;
    const { count, step, width } = this.columns();
    for (let i = 0; i < count; i++) {
      const h = Math.round(this.bars[i]);
      const x = i * step;
      for (let y = H - h; y < H; y++) {
        ctx.fillStyle = this.barColor(y, h, H);
        ctx.fillRect(x, y, width, 1);
      }
      const p = Math.round(this.peaks[i]);
      if (this.opts.peaks && p > 0) {
        ctx.fillStyle = this.colors[23];
        ctx.fillRect(x, H - p, width, 1);
      }
    }
  }

  private drawShadeSpectrum() {
    const ctx = this.shadeCtx;
    if (!ctx || !this.shaded()) return;
    const { count } = this.columns();
    for (let x = 0; x < SHADE_W; x++) {
      // Sample the bars across the narrower area.
      const h = Math.round((this.bars[Math.floor((x * count) / SHADE_W)] / H) * SHADE_H);
      for (let y = SHADE_H - h; y < SHADE_H; y++) {
        ctx.fillStyle = this.barColor(y, h, SHADE_H);
        ctx.fillRect(x, y, 1, 1);
      }
    }
  }

  private scopeColor(y: number, height: number) {
    // Brightest at the center line, like the original's palette 18..22.
    const mid = (height - 1) / 2;
    return this.colors[18 + Math.min(4, Math.floor((Math.abs(y - mid) * 8) / height))];
  }

  private drawScopeOn(ctx: CanvasRenderingContext2D, wave: number[], width: number, height: number) {
    const yOf = (v: number) => Math.min(height - 1, Math.max(0, Math.round(((255 - v) / 255) * (height - 1))));
    const mid = Math.round((height - 1) / 2);
    const at = (x: number) => wave[Math.floor((x * wave.length) / width)];
    let prev = yOf(at(0));
    for (let x = 0; x < width; x++) {
      const y = yOf(at(x));
      let from = y;
      let to = y;
      if (this.opts.scope === "lines") [from, to] = [Math.min(prev, y), Math.max(prev, y)];
      else if (this.opts.scope === "solid") [from, to] = [Math.min(mid, y), Math.max(mid, y)];
      for (let yy = from; yy <= to; yy++) {
        ctx.fillStyle = this.scopeColor(yy, height);
        ctx.fillRect(x, yy, 1, 1);
      }
      prev = y;
    }
  }

  private drawScope(wave: number[]) {
    this.drawScopeOn(this.ctx, wave, wave.length, H);
  }

  private drawShadeScope(wave: number[]) {
    if (this.shadeCtx && this.shaded()) this.drawScopeOn(this.shadeCtx, wave, SHADE_W, SHADE_H);
  }
}
