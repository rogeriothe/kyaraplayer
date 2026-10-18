// "Open URL" dialog (Ctrl+L): play or enqueue a stream or remote playlist.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const win = getCurrentWindow();
const input = document.getElementById("url") as HTMLInputElement;
const error = document.getElementById("error")!;
const buttons = document.querySelectorAll("button");

async function submit(play: boolean) {
  const url = input.value.trim();
  if (!url) return;
  buttons.forEach((b) => (b.disabled = true));
  error.textContent = "Conectando...";
  try {
    await invoke("pl_add_url", { url, play });
    await win.close();
  } catch (e) {
    error.textContent = String(e);
    buttons.forEach((b) => (b.disabled = false));
    input.focus();
  }
}

document.getElementById("form")!.addEventListener("submit", (e) => {
  e.preventDefault();
  void submit(true);
});
document.getElementById("enqueue")!.addEventListener("click", () => void submit(false));
document.getElementById("cancel")!.addEventListener("click", () => void win.close());
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void win.close();
});
input.focus();
