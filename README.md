# Kyara Player

Player de áudio para desktop com a interface clássica de três janelas (principal, equalizador e playlist) no estilo dos players dos anos 2000, compatível com skins clássicas `.wsz`. Escrito em Rust com [Tauri v2](https://tauri.app); a interface é TypeScript + Vite.

Implementação clean-room: nenhum bitmap, ícone ou código de terceiros é embutido. A skin padrão é desenhada por código (`tools/gen_default_skin.py`) e qualquer skin `.wsz` fornecida pelo usuário pode substituí-la.

## Recursos

- **Formatos:** MP3, FLAC, Ogg Vorbis, Opus, WAV, AIFF, AAC/M4A/ALAC, MKA (via [symphonia](https://github.com/pdeljanov/Symphonia)).
- **Streams:** rádios HTTP/HTTPS com metadados ICY (Shoutcast/Icecast), `.pls` e `.m3u` remotos.
- **Equalizador:** 10 bandas + preamp, bandas clássicas ou ISO, limitador, presets próprios, import/export `.eqf`/`.q1` e presets automáticos por arquivo.
- **Visualização:** analisador de espectro (FFT) e osciloscópio, em vários estilos, com as cores da skin.
- **Playlist:** `.m3u`, `.m3u8` e `.pls`; shuffle/repeat; arrastar e soltar arquivos e pastas; edição de tags.
- **Skins:** `.wsz` ou pasta, com `viscolor.txt`, `pledit.txt`, `region.txt` (janelas com forma) e cursores `.cur`.
- **Janelas:** docking e snap entre janelas, windowshade, tamanho duplo (Ctrl+D), sempre no topo (Ctrl+A).
- **Integração com o sistema:** MPRIS (Linux) e SMTC (Windows), ícone na bandeja, atalhos globais, instância única e associação de arquivos.

### Atalhos globais (opcionais, ativados pelo menu)

| Atalho | Ação |
|---|---|
| Ctrl+Alt+Home | Play |
| Ctrl+Alt+Insert | Pause |
| Ctrl+Alt+End | Stop |
| Ctrl+Alt+PgUp / PgDn | Faixa anterior / próxima |
| Ctrl+Alt+↑ / ↓ | Volume |
| Ctrl+Alt+← / → | Voltar / avançar 5 s |

No Wayland os atalhos globais não funcionam.

## Download

Os binários de cada versão ficam em [Releases](https://github.com/rogeriothe/kyaraplayer/releases):

- **Linux:** `.AppImage`, `.deb` e `.rpm`
- **Windows:** instalador `.exe` (NSIS) e `.msi`

## Build

### Pré-requisitos

- [Rust](https://rustup.rs) estável
- [Node.js](https://nodejs.org) 22 ou mais recente
- Dependências de sistema do Tauri: veja [os pré-requisitos do Tauri](https://tauri.app/start/prerequisites/)

No Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libasound2-dev patchelf
```

No Windows são necessários o Microsoft C++ Build Tools e o WebView2 (já incluso no Windows 10/11).

### Desenvolvimento

```sh
npm ci
npm run tauri dev
```

### Gerar instaladores

```sh
npm run tauri build
```

Os pacotes ficam em `target/release/bundle/` (`appimage/`, `deb/`, `rpm/` no Linux; `nsis/`, `msi/` no Windows).

### Testes

```sh
cargo test --workspace
```

### Ferramentas auxiliares

```sh
# Tocar um arquivo direto pelo motor de áudio, sem interface
cargo run -p audio-engine --example play -- musica.flac

# Extrair os sprites de uma skin para PNG
cargo run -p skin --example dump -- skin.wsz saida/

# Regerar a skin padrão (requer Pillow)
python3 tools/gen_default_skin.py
```

Para logs detalhados, defina `RUST_LOG`, por exemplo `RUST_LOG=debug npm run tauri dev`.

## Estrutura

```
crates/
  audio-engine/   decodificação, reamostragem, EQ, FFT e saída de áudio (cpal)
  skin/           leitura de skins .wsz e skin padrão
  playlist/       modelo da playlist, m3u/pls, tags
src-tauri/        aplicação Tauri: comandos, janelas, bandeja, atalhos, MPRIS/SMTC
ui/               interface (TypeScript): janelas principal, EQ, playlist, visualização
tools/            gerador da skin padrão
```

O plano de implementação e o estado de cada etapa estão em [`PLANO_IMPLEMENTACAO.md`](PLANO_IMPLEMENTACAO.md).

## Publicar uma versão

O workflow `.github/workflows/release.yml` compila para Linux e Windows e publica uma Release quando uma tag `v*` é enviada:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Antes, atualize a versão em `Cargo.toml` (workspace), `src-tauri/tauri.conf.json` e `package.json`.

## Licença

MIT.
