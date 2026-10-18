// File info window (Alt+3): stream properties and editable tags.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { mmss } from "./common";

interface Tags {
  title: string;
  artist: string;
  album: string;
  year: string;
  genre: string;
  track: string;
  comment: string;
}

interface Details {
  tags: Tags;
  format: string;
  duration: number | null;
  bitrate: number | null;
  sample_rate: number | null;
  channels: number | null;
  bit_depth: number | null;
  file_size: number | null;
  tag_format: string;
}

const win = getCurrentWindow();
const path = new URLSearchParams(location.search).get("path") ?? "";
const form = document.getElementById("form") as HTMLFormElement;
const status = document.getElementById("status")!;
const fields = form.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("#tags [name]");
const saveButton = document.getElementById("save") as HTMLButtonElement;

(document.getElementById("path") as HTMLInputElement).value = path;

function setStatus(text: string, error = false) {
  status.textContent = text;
  status.classList.toggle("error", error);
}

function showProps(d: Details) {
  const rows: [string, string | null][] = [
    ["Formato", d.format],
    ["Duração", d.duration !== null ? mmss(d.duration) : null],
    ["Bitrate", d.bitrate ? `${d.bitrate} kbps` : null],
    ["Amostragem", d.sample_rate ? `${d.sample_rate} Hz` : null],
    ["Canais", d.channels ? (d.channels === 1 ? "mono" : d.channels === 2 ? "estéreo" : `${d.channels}`) : null],
    ["Resolução", d.bit_depth ? `${d.bit_depth} bits` : null],
    ["Tamanho", d.file_size !== null ? `${(d.file_size / 1048576).toFixed(2)} MB` : null],
    ["Tags", d.tag_format],
  ];
  const dl = document.getElementById("props")!;
  dl.replaceChildren(
    ...rows
      .filter(([, v]) => v)
      .flatMap(([k, v]) => {
        const dt = document.createElement("dt");
        dt.textContent = k;
        const dd = document.createElement("dd");
        dd.textContent = v;
        return [dt, dd];
      }),
  );
}

async function load() {
  try {
    const d = await invoke<Details>("file_info", { path });
    for (const f of fields) f.value = d.tags[f.name as keyof Tags] ?? "";
    showProps(d);
    document.title = `${d.tags.title || path.split(/[\\/]/).pop()} - Informações`;
  } catch (e) {
    setStatus(String(e), true);
    fields.forEach((f) => (f.disabled = true));
    saveButton.disabled = true;
  }
}

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  const tags = Object.fromEntries([...fields].map((f) => [f.name, f.value])) as unknown as Tags;
  saveButton.disabled = true;
  setStatus("Salvando...");
  try {
    await invoke("file_info_save", { path, tags });
    await win.close();
  } catch (err) {
    setStatus(String(err), true);
    saveButton.disabled = false;
  }
});
document.getElementById("close")!.addEventListener("click", () => void win.close());
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void win.close();
});

void load();
