// Shared types and helpers for all windows.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

export type PlayerState = "stopped" | "playing" | "paused";

export interface TrackInfo {
  path: string;
  duration: number | null;
  bitrate: number | null;
  sample_rate: number;
  channels: number;
}

export interface Status {
  state: PlayerState;
  position: number;
  volume: number;
  balance: number;
  track: TrackInfo | null;
}

export type EngineEvent =
  | { type: "stateChanged"; state: PlayerState }
  | { type: "trackStarted"; track: TrackInfo }
  | { type: "trackEnded" }
  | { type: "error"; message: string }
  | { type: "connecting"; path: string }
  | { type: "streamTitle"; title: string };

export interface PlaylistEntry {
  id: number;
  title: string;
  duration: number | null;
}

export interface PlaylistView {
  entries: PlaylistEntry[];
  current: number | null;
  shuffle: boolean;
  repeat: boolean;
  total: number;
  total_complete: boolean;
}

export interface WindowsView {
  equalizer: boolean;
  playlist: boolean;
  main_shade: boolean;
  eq_shade: boolean;
  pl_shade: boolean;
  double: boolean;
  always_on_top: boolean;
}

export const APP_TITLE = "Kyara Player 0.1";
export const AUDIO_EXTENSIONS = [
  "mp3", "mp2", "mp1", "flac", "ogg", "oga", "opus", "wav", "aiff", "aif", "m4a", "mp4", "aac",
  "alac", "mka", "webm", "caf",
];
export const PLAYLIST_EXTENSIONS = ["m3u", "m3u8", "pls"];
export const SKIN_EXTENSIONS = ["wsz", "zip"];

export const ext = (path: string) => path.split(".").pop()?.toLowerCase() ?? "";
export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function mmss(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Long durations as h:mm:ss, like the playlist's running time. */
export function hmmss(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 3600) return mmss(s);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return `${h}:${String(m).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** Pick audio/playlist files. */
export async function pickFiles(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    filters: [
      { name: "Áudio e playlists", extensions: [...AUDIO_EXTENSIONS, ...PLAYLIST_EXTENSIONS] },
      { name: "Todos os arquivos", extensions: ["*"] },
    ],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

export async function pickFolder(): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false });
  return typeof picked === "string" ? picked : null;
}

/** Play button semantics: resume when paused, otherwise (re)start the current entry. */
export function playPressed(st: Status) {
  return st.state === "paused" ? invoke("toggle_pause") : invoke("pl_play", { id: null });
}

/** Playback keys shared by every window (classic bindings). Returns true if handled. */
export function handlePlaybackKey(e: KeyboardEvent, status: () => Status): boolean {
  if (e.ctrlKey || e.altKey || e.metaKey) return false;
  switch (e.key.toLowerCase()) {
    case "z": invoke("pl_prev"); break;
    case "x": playPressed(status()); break;
    case "c": invoke("toggle_pause"); break;
    case "v": invoke("stop"); break;
    case "b": invoke("pl_next"); break;
    default:
      if (e.code === "Numpad1") invoke("pl_jump", { delta: -10 });
      else if (e.code === "Numpad3") invoke("pl_jump", { delta: 10 });
      else return false;
  }
  return true;
}

/**
 * Drag the window from any non-control area. The backend reads the cursor itself,
 * moves the docked group and snaps edges; the page only signals movement.
 */
export function enableWindowDrag(root: HTMLElement, label: string) {
  root.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".ctl")) return;
    root.setPointerCapture(e.pointerId);
    const started = invoke<boolean>("drag_start", { label }).then((managed) => {
      if (!managed) {
        root.releasePointerCapture(e.pointerId);
        getCurrentWindow().startDragging();
      }
      return managed;
    });
    let dirty = false;
    let inFlight: Promise<unknown> | null = null;
    // At most one move in flight; coalesce the rest.
    const pump = () => {
      if (inFlight || !dirty) return;
      dirty = false;
      inFlight = started.then((managed) => managed && invoke("drag_move")).finally(() => {
        inFlight = null;
        pump();
      });
    };
    const move = () => {
      dirty = true;
      pump();
    };
    const up = async () => {
      root.removeEventListener("pointermove", move);
      root.removeEventListener("pointerup", up);
      root.removeEventListener("pointercancel", up);
      dirty = true;
      pump();
      while (inFlight) await inFlight;
      if (await started) invoke("drag_end");
    };
    root.addEventListener("pointermove", move);
    root.addEventListener("pointerup", up);
    root.addEventListener("pointercancel", up);
  });
}
