// Main window menu (options button and right click), modeled on the classic one.

import { Menu, type MenuItemOptions } from "@tauri-apps/api/menu";

export interface MainMenuState {
  equalizer: boolean;
  playlist: boolean;
  double: boolean;
  alwaysOnTop: boolean;
  globalHotkeys: boolean;
  showRemaining: boolean;
  repeat: boolean;
  shuffle: boolean;
  hasSkin: boolean;
}

export interface MainMenuActions {
  about(): void;
  openFile(): void;
  openFolder(): void;
  openUrl(): void;
  fileInfo(): void;
  toggleEqualizer(): void;
  togglePlaylist(): void;
  loadSkin(): void;
  defaultSkin(): void;
  toggleDouble(): void;
  toggleAlwaysOnTop(): void;
  toggleGlobalHotkeys(): void;
  setRemaining(on: boolean): void;
  toggleRepeat(): void;
  toggleShuffle(): void;
  prev(): void;
  play(): void;
  pause(): void;
  stop(): void;
  next(): void;
  seekBy(secs: number): void;
  jump(delta: number): void;
  exit(): void;
}

// Accelerators only label the shortcuts; the keys themselves are handled by the page.
const item = (text: string, action: () => void, accelerator?: string): MenuItemOptions => ({
  text,
  action: () => action(),
  accelerator,
});
const check = (text: string, checked: boolean, action: () => void, accelerator?: string) => ({
  text,
  checked,
  action: () => action(),
  accelerator,
});
const separator = { item: "Separator" } as const;

export async function showMainMenu(s: MainMenuState, a: MainMenuActions) {
  const menu = await Menu.new({
    items: [
      item("Kyara Player...", a.about),
      separator,
      item("Abrir arquivo...", a.openFile, "L"),
      item("Abrir pasta...", a.openFolder, "Shift+L"),
      item("Abrir endereço...", a.openUrl, "Ctrl+L"),
      item("Informações do arquivo...", a.fileInfo, "Alt+3"),
      separator,
      check("Equalizador", s.equalizer, a.toggleEqualizer, "Alt+G"),
      check("Playlist", s.playlist, a.togglePlaylist, "Alt+E"),
      separator,
      {
        text: "Skins",
        items: [
          item("Carregar skin...", a.loadSkin, "Alt+S"),
          check("Skin padrão", !s.hasSkin, a.defaultSkin),
        ],
      },
      {
        text: "Opções",
        items: [
          check("Sempre no topo", s.alwaysOnTop, a.toggleAlwaysOnTop, "Ctrl+A"),
          check("Tamanho duplo", s.double, a.toggleDouble, "Ctrl+D"),
          check("Atalhos globais (Ctrl+Alt+...)", s.globalHotkeys, a.toggleGlobalHotkeys),
          separator,
          check("Tempo decorrido", !s.showRemaining, () => a.setRemaining(false), "Ctrl+T"),
          check("Tempo restante", s.showRemaining, () => a.setRemaining(true), "Ctrl+T"),
          separator,
          check("Repetir", s.repeat, a.toggleRepeat, "R"),
          check("Aleatório", s.shuffle, a.toggleShuffle, "S"),
        ],
      },
      {
        text: "Reprodução",
        items: [
          item("Anterior", a.prev, "Z"),
          item("Play", a.play, "X"),
          item("Pause", a.pause, "C"),
          item("Stop", a.stop, "V"),
          item("Próxima", a.next, "B"),
          separator,
          item("Voltar 5 s", () => a.seekBy(-5), "Left"),
          item("Avançar 5 s", () => a.seekBy(5), "Right"),
          item("Voltar 10 faixas", () => a.jump(-10)),
          item("Avançar 10 faixas", () => a.jump(10)),
        ],
      },
      separator,
      item("Sair", a.exit),
    ],
  });
  await menu.popup();
}
