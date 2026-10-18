# Kyara Player — Análise do Winamp e Plano (Rust + Tauri)

## 1. O que é `winamp_latest_full.exe`

Instalador NSIS (PE32 i386), 13 MB, extraído com `7z` (1354 arquivos). Versão **Winamp 5.9.2 (26/04/2023)**.

| Parte | Conteúdo | Relevância p/ clone |
|---|---|---|
| `winamp.exe` (2,4 MB) | Núcleo: janelas clássicas (main/EQ/playlist), skin clássica embutida, menus, atalhos, API de plugins | **Núcleo do clone** |
| `Plugins/in_*.dll` (16) | Decoders: mp3, mp4/aac, flac, vorbis, wave, wm, midi, mod, cdda, avi/mkv/flv/nsv/swf, dshow, linein | Substituir por `symphonia` |
| `Plugins/out_*.dll` | Saída: DirectSound (crossfade), WASAPI, WaveOut, Disk Writer | Substituir por `cpal` + writer WAV |
| `Plugins/enc_*.dll` | Encoders lame/flac/vorbis/wav/wma (ripping/transcode) | Fase tardia |
| `Plugins/ml_*.dll` (18) + `gen_ml.dll`, `Shared/nde.dll` | Media Library (DB NDE, playlists, histórico, podcasts, ripping, ReplayGain, devices) | Fase 3, com SQLite |
| `Plugins/pmp_*.dll` | Sync com iPod/Android/USB/MTP/Wi‑Fi | Fora de escopo |
| `gen_ff.dll` + `$R0/Bento`, `Big Bento`, `Winamp Modern` | Engine de skins "modernas" (Wasabi/Freeform, XML + scripts MAKI) | Fase 4 / opcional |
| `gen_tray`, `gen_hotkeys` | Ícone na bandeja, hotkeys globais | Plugins Tauri |
| `vis_milk2.dll` + 477 presets, `vis_avs.dll` | Visualizações MilkDrop 2 e AVS | MilkDrop via `butterchurn` (WebGL) |
| `dsp_sps.dll` | Signal Processing Studio (DSP scriptável) | Fora de escopo |
| `System/*.w5s` | Componentes Wasabi (decoders de imagem/vídeo, playlist, albumart, rede) | — |
| Qt5 DLLs, `ml_wire`, `ml_online`, `ml_fanzone`, `ml_nft` | Serviços online | Fora de escopo |

### Skin clássica embutida (recursos BITMAP do `winamp.exe`)

Dimensões batem 1:1 com o formato de skin `.wsz` (2.x):

| ID | Tamanho | Arquivo de skin equivalente |
|---|---|---|
| 126 | 275×116 | `main.bmp` |
| 127 | 136×36 | `cbuttons.bmp` (prev/play/pause/stop/next/eject) |
| 129 | 42×9 | `playpaus.bmp` |
| 128 | 58×24 | `monoster.bmp` |
| 130 | 92×85 | `shufrep.bmp` |
| 132 | 99×13 | `numbers.bmp` |
| 133 / 197 | 68×433 | `volume.bmp` / `balance.bmp` |
| 134 | 155×74 | `text.bmp` (fonte bitmap 5×6) |
| 135 | 307×10 | `posbar.bmp` |
| 150 | 344×87 | `titlebar.bmp` |
| 194 / 1370 | 275×315 | `eqmain.bmp` / `eqmain_iso.bmp` |
| 199 | 280×186 | `pledit.bmp` |
| 249/250 | — | `gen.bmp` / `genex.bmp` (janelas genéricas) |

Outros arquivos de skin que o exe lê: `viscolor.txt` (24 cores da vis), `pledit.txt` (cores/fonte da playlist), `region.txt` (máscara de forma das janelas), `eq_ex.bmp`, `nums_ex.bmp`, `video.bmp` e ~20 cursores `.cur` (`Normal.cur`, `PSize.cur`, `EQSlid.cur`, `VolBal.cur` …).

### Comportamento observado (menus/strings do exe)

Menu principal e atalhos: Play File `L`, URL `Ctrl+L`, Folder `Shift+L`, Prev/Play/Pause/Stop/Next = `Z X C V B`, Stop c/ fadeout `Shift+V`, Stop after current `Ctrl+V`, ±5 s `←/→`, ±10 faixas `Num1/Num3`, Jump to time `Ctrl+J`, Jump to file `J`, Repeat `R`, Shuffle `S`, Always on top `Ctrl+A`, Double size `Ctrl+D`, EasyMove `Ctrl+E`, tempo decorrido/restante `Ctrl+T`, janelas `Alt+W/E/G/V`, prefs `Ctrl+P`, skins `Alt+S`, bookmarks `Ctrl+Alt+B/I`, file info `Alt+3`, vis `Ctrl+Shift+K / Ctrl+K / Alt+K`.
Config em `Winamp.ini` (`mw_open`, `pe_open`, `pe_width`, `pe_height`, `pe_height_ws`, `eq_open`, `eq_data`, `eq_limiter`, `eq_type`, `eq_frequencies`, `SkinDir` …). Presets EQ no formato binário `Winamp EQ library file v1.1` (`.q1`/`.eqf`). Playlists `.m3u`, `.m3u8`, `.pls` (+ `xspf` via componente).

> Base de referência para coordenadas de sprite e comportamento: projeto **Webamp** (MIT, reimplementação JS da skin clássica) — documenta todos os retângulos de cada BMP.

## 2. Aviso legal

- Bitmaps, cursores, ícones e textos do Winamp são **protegidos por copyright**. Não embutir os recursos extraídos no clone distribuído. Criar skin padrão própria e carregar `.wsz` fornecidos pelo usuário (inclusive o "Base Skin" que ele tenha).
- O código-fonte do Winamp publicado em 2024 usa licença restritiva (proíbe distribuir forks). Implementação deve ser **clean-room**: usar só comportamento observável e documentação pública de formatos.
- Nome "Winamp" e logo são marca registrada — projeto se chama **Kyara Player**.

## 3. Escopo de "idêntico"

| Fase | Entrega | Paridade |
|---|---|---|
| **1 – MVP clássico** | Main window + EQ + Playlist, skins `.wsz`, decodificação, EQ 10 bandas, vis espectro/osciloscópio | Visual pixel-a-pixel com skin clássica |
| **2 – Paridade clássica** | Docking/snap, windowshade, double size, region.txt, cursores, menus completos, todos atalhos, presets EQ, crossfade, streams Shoutcast/ICY, hotkeys globais, tray, MPRIS/SMTC | Comportamento idêntico ao 5.9 com skin clássica |
| **3 – Media Library** | Biblioteca local (scan, tags, busca, smart views), playlists salvas, histórico, ReplayGain, album art | Funcional, visual "gen.bmp" |
| **4 – Extras** | MilkDrop (butterchurn + presets `.milk`), vídeo, ripping CD, transcode | Parcial |
| **Fora** | Skins modernas MAKI (Bento), AVS, DSP SPS, pmp devices, serviços online, plugins `.dll` nativos | — |

Skins modernas (Wasabi XML + bytecode MAKI) exigem VM de script inteira; deixar para projeto separado.

## 4. Arquitetura

```
kyara-player/
├── Cargo.toml                 (workspace)
├── crates/
│   ├── audio-engine/          decode → DSP → saída, thread própria
│   ├── skin/                  parser .wsz (zip + bmp + txt + cur)
│   ├── playlist/              modelo, m3u/m3u8/pls/xspf, shuffle
│   ├── library/               SQLite, scanner, tags (fase 3)
│   └── config/                ini/toml, presets EQ .q1/.eqf
├── src-tauri/                 app Tauri v2: comandos, eventos, janelas, tray, hotkeys
└── ui/                        TypeScript + Vite (Solid ou Svelte), render via canvas/CSS sprites
```

### 4.1 Áudio (Rust, nunca no webview)

Motivo: formatos, gapless, tocar com janela minimizada, latência estável, hotkeys globais.

- Decode: `symphonia` (mp3, aac/m4a, flac, vorbis, opus via `symphonia-adapter-libopus`, wav, alac). MOD: `openmpt` bindings (opcional). MIDI: `rustysynth` (opcional).
- Resample: `rubato` (para taxa do device).
- Saída: `cpal`, buffer `ringbuf` entre thread decoder e callback.
- DSP em cadeia: pré-amp → EQ 10 bandas (biquad peaking em 70, 180, 320, 600, 1k, 3k, 6k, 12k, 14k, 16k Hz — bandas do Winamp 5, conforme o bitmap embutido; ±12 dB; modo ISO 31.5…16k opcional = `eq_frequencies`) → limiter (`eq_limiter`) → balance → volume → crossfade.
- Análise p/ vis: `rustfft` 512 pontos, 19 barras (main window) + 75 amostras de osciloscópio, enviado ~70 fps.
- Streams HTTP: `reqwest` blocking + parser ICY (`icy-metaint`) para título Shoutcast.
- Tags/arte: `lofty`.
- Controle: canal `crossbeam` com comandos `Play/Pause/Stop/Seek/SetVolume/SetEq/...`; eventos de estado de volta.

### 4.2 Tauri (janelas)

- 3 janelas `WebviewWindow` sem decoração, transparentes, `skip_taskbar` nas filhas:
  - main 275×116 (windowshade 275×14), EQ 275×116, playlist 275×116 mínimo, redimensiona em passos 25×29.
  - Double size = escala CSS 2× + resize da janela.
- Docking/snap: lógica em Rust (`on_window_event` Moved) — snap a 10 px entre janelas e bordas do monitor; janelas encaixadas movem juntas com a main.
- `region.txt`: janela transparente + `clip-path: polygon()` + ignorar cliques fora (`set_ignore_cursor_events` dinâmico).
- Arrastar: `startDragging()` na titlebar; EasyMove = arrastar de qualquer ponto.
- Menus de contexto: menus nativos Tauri (`Menu`) espelhando menu 101 do exe.
- Plugins: `tauri-plugin-global-shortcut`, `tauri-plugin-dialog`, `tauri-plugin-fs`, `tauri-plugin-single-instance` (abrir arquivo em instância existente), tray nativo, `souvlaki` (MPRIS/SMTC/media keys).
- Vis/estado: `tauri::ipc::Channel` (mais rápido que eventos) para frames de vis.

### 4.3 Skins

- Rust: abre `.wsz` (`zip`), busca nomes case-insensitive, decodifica BMP (`image`), converte para PNG e serve via protocolo custom `skin://current/main.png`.
- Parse `viscolor.txt`, `pledit.txt` (Normal, Current, NormalBG, SelectedBG, Font), `region.txt`, `.cur` → PNG + hotspot para CSS `cursor:url() x y`.
- Fallback por arquivo ausente: skin padrão própria.
- UI: cada controle = elemento com `background-position` do sprite; `image-rendering: pixelated`. Tabela de sprites (posição/estado normal/pressed/active) num JSON único.
- Texto: marquee com `text.bmp` (fonte 5×6, rolagem 1 px a cada ~ 50 ms, separador `***`); tempo com `numbers.bmp`/`nums_ex.bmp`; kbps/kHz com `text.bmp`.

### 4.4 Playlist

- Modelo em Rust (fonte da verdade), virtualização na UI (milhares de itens).
- Operações: add/rem/sel/misc/list (botões do pledit), ordenar, randomizar, inverter, remover duplicados/mortos, jump to file com busca, drag-drop de arquivos e reordenação.
- Formatos: m3u/m3u8 (`#EXTINF`), pls, xspf (`quick-xml`).
- Formato de título configurável (`%artist% - %title%`).

### 4.5 Config

- `config.toml` no diretório de dados da app (`directories`): janelas abertas/posições/tamanhos, skin, volume, EQ, modo tempo, repeat/shuffle, always on top.
- Import/export de presets EQ `.eqf`/`.q1` (header `Winamp EQ library file v1.1\x1a!--`, 257 bytes nome + 11 valores 0–63).

## 5. Marcos

| # | Marco | Critério de aceite |
|---|---|---|
| M0 | Workspace + Tauri v2 + janela frameless 275×116 | Abre, arrasta, fecha |
| M1 | `audio-engine` toca mp3/flac/ogg via CLI de teste | Play/pause/seek/volume, sem estalos |
| M2 | Parser `.wsz` + main window renderizada | Screenshot comparado com Webamp mesma skin, diff 0 px |
| M3 | Controles funcionais (botões, posbar, volume, balance, shuffle/repeat, tempo, marquee, kbps/kHz, mono/stereo) | Todos atalhos de teclado do menu |
| M4 | Janela EQ + DSP + presets | Curva EQ igual ao gráfico; on/auto/presets |
| M5 | Playlist window + formatos | Carrega 10k itens fluido; resize 25×29 |
| M6 | Vis (espectro, osciloscópio, cores viscolor, modos de pico) | ~70 fps, CPU < 5% |
| M7 | Docking, windowshade, double size, region, cursores | Comportamento igual ao 5.9 |
| M8 | Streams ICY, hotkeys globais, tray, media keys, single instance, associação de arquivos | — |
| M9 | Media Library (SQLite + scanner + views) | — |
| M10 | MilkDrop (butterchurn) em janela separada | Carrega `.milk` |
| M11 | Instaladores (`tauri build`: msi/nsis, deb/AppImage, dmg) | — |

## 6. Riscos

- **Transparência/região no Linux**: WebKitGTK + compositor variam; X11 sem compositor não tem transparência → fallback retangular.
- **Snap/docking**: eventos de move do Tauri são assíncronos; pode haver "tremor". Mitigar movendo janelas filhas em Rust no mesmo evento.
- **Pixel-perfect**: escala DPI do SO (Windows 125/150%) quebra sprites → forçar escala inteira e `pixelated`.
- **Latência UI↔áudio**: comandos via IPC são ~1 ms, aceitável; vis por Channel binário.
- **Formatos WMA/WMV**: sem decoder Rust maduro → `ffmpeg` opcional ou fora.

## 7. Próximos passos

1. Criar workspace (M0) e protótipo do `audio-engine` (M1) em paralelo.
2. Montar JSON de sprites da skin clássica a partir da documentação pública (Webamp).
3. Definir skin padrão própria (sem assets da Nullsoft).

## 8. Status

| Marco | Estado | Notas |
|---|---|---|
| M0 | Feito | Janela 275×116 sem borda. No Linux, `resizable:false` faz o GTK forçar 200 px de altura; usar `resizable:true` + min/max fixos |
| M1 | Feito | `crates/audio-engine`; reamostragem FFT band-limited (`rubato` 5, chunk 1024, atraso do filtro descontado, cauda descarregada no fim da faixa); linear só como fallback |
| M2 | Feito | `crates/skin` (`.wsz`/pasta, BMP→PNG, `viscolor.txt`, `pledit.txt`, reparo de `bfOffBits`); skin padrão original gerada por `tools/gen_default_skin.py`; protocolo `skin://`; janela principal com sprites |
| M3 | Feito | Botões, sliders, tempo, marquee, kbps/kHz, mono/stereo, prev/next, shuffle/repeat e atalhos. Menu nativo (botão de opções e clique direito, `ui/src/mainmenu.ts`): abrir arquivo/pasta, janelas, skins, sempre no topo (Ctrl+A, persistido), tamanho duplo, tempo decorrido/restante, reprodução. No GTK, menus popup do muda não exibem os atalhos (sem accel group) |
| M4 | Feito | EQ 10 bandas + preamp no callback de saída (parâmetros atômicos, sem lock), limitador; presets próprios + import/export `.eqf`/`.q1`; janela com sprites `eqmain.bmp`, curva Catmull-Rom com cores da skin; bandas clássicas ou ISO. "Auto" carrega o preset salvo para o nome do arquivo (ou endereço do stream) ao iniciar a faixa; menu Salvar → Automático, Excluir lista os automáticos |
| M6 | Feito | Tap lock-free na saída (pós-EQ, pré-volume); FFT 1024 + Hann (rustfft) → 75 colunas log; UI com 19 barras, queda e picos, osciloscópio; cores de `viscolor.txt`; clique alterna espectro/osciloscópio/desligado. Estilos (botão direito no vis): analisador normal/fogo/linha, barras grossas/finas, picos on/off; osciloscópio pontos/linhas/sólido (salvos em localStorage); vis 38×5 no modo compacto. Não verificado visualmente |
| M5 | Feito | `crates/playlist` (IDs estáveis, shuffle/repeat, m3u/m3u8/pls, tags via lofty); 2ª janela com sprites `pledit.bmp`, seleção, arrastar para reordenar, menus sprite, redimensionar 25×29, drop de arquivos/pastas; playlist, faixa atual, volume, balance e shuffle/repeat persistem |
| M7 | Parcial | Docking (grupo segue a principal, snap 10 px, bordas da tela) verificado. Windowshade das 3 janelas e double size (Ctrl+D) implementados, falta verificar. No Muffin, `outer_position` inclui `_NET_FRAME_EXTENTS` falso; usar `inner_position` + `cursor_position()`; no Wayland o arrasto cai para `startDragging`. Cursores `.cur` → PNG + hotspot (`--cur-*`, `cursors.css`); `region.txt` → `clipPath` SVG com janela transparente (verificado: alpha 0 fora do polígono). `.ani` não suportado |
| M8 | Parcial | Instância única (2ª execução passa os arquivos e sai; caminhos relativos ao cwd dela). MPRIS/SMTC/Now Playing via `souvlaki` (zbus): metadados "Artista - Título", estado, posição (atualizada a cada 1 s), play/pause/next/prev/stop/seek/SetPosition/OpenUri/Raise/Quit — verificado com `busctl`. Bandeja (AppIndicator no Linux; sem tooltip/clique esquerdo lá) com controles e mostrar/ocultar. Atalhos globais Ctrl+Alt+Home/Insert/End/PgUp/PgDn/setas (opção no menu, persistida; não funcionam no Wayland). Streams HTTP(S) com ICY (`StreamTitle` vira título da entrada), `.pls`/`.m3u` remotos, "Abrir endereço" (Ctrl+L, botão Add URL da playlist); leitura em thread própria com abort/timeout para stop não travar. Associação de arquivos no bundle (áudio, m3u/pls, wsz); arquivos abertos pela linha de comando, 2ª instância ou Finder (`RunEvent::Opened` no macOS): `.wsz` carrega skin, o resto substitui a playlist. `http://` usa cliente HTTP/1.0 próprio (aceita `ICY 200 OK` do Shoutcast v1, redirects, timeout por leitura); `https://` via ureq. Indicador de conexão: luz "working" do `playpaus.bmp` + "Conectando..." no visor. `.wsz` no Linux: XML do shared-mime-info instalado por deb/rpm. Logs com `env_logger` (`RUST_LOG`). Falta: HLS |
| Extras | Parcial | Janela de informações do arquivo (Alt+3, menu principal, Misc → File info da playlist): propriedades (formato, duração, bitrate, amostragem, canais, bits, tamanho) e edição de tags (título, artista, álbum, ano, faixa, gênero, comentário) via lofty na tag primária; salvar relê o título na playlist. |
