//! HTTP(S) audio streams (Shoutcast/Icecast radio) with ICY metadata.
//!
//! The connection runs on its own thread and hands audio bytes over a bounded
//! channel, so a stalled server never blocks the engine: reads give up after a
//! timeout or as soon as the stream is aborted (stop, new track).

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};

use crate::EngineError;

const CHUNK: usize = 8 * 1024;
/// About 0.5 MB of read-ahead.
const QUEUE: usize = 64;
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

/// True for locations the engine must fetch over the network.
pub fn is_url(location: &str) -> bool {
    let lower = location.get(..8).unwrap_or(location).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// What the server said about the stream.
#[derive(Debug, Clone, Default)]
pub struct StreamInfo {
    /// File-extension hint for the prober (`mp3`, `aac`, `ogg`, ...).
    pub hint: Option<&'static str>,
    pub content_type: Option<String>,
    /// Station name (`icy-name`).
    pub name: Option<String>,
    /// Nominal bitrate in kbps (`icy-br`).
    pub bitrate: Option<u32>,
}

/// Shared between the reader thread and the engine.
#[derive(Default)]
pub struct StreamState {
    pub abort: AtomicBool,
    /// Latest `StreamTitle`, taken by the engine when it changes.
    pub title: Mutex<Option<String>>,
}

pub struct StreamReader {
    rx: Receiver<io::Result<Vec<u8>>>,
    chunk: Vec<u8>,
    offset: usize,
    state: Arc<StreamState>,
}

/// Connect and wait for the response headers (abortable through `state`).
pub fn open(url: &str, state: Arc<StreamState>) -> Result<(StreamReader, StreamInfo), EngineError> {
    let (head_tx, head_rx) = bounded::<Result<StreamInfo, String>>(1);
    let (data_tx, data_rx) = bounded(QUEUE);
    let url = url.to_string();
    let thread_state = state.clone();
    std::thread::Builder::new()
        .name("stream".into())
        .spawn(move || run(&url, &thread_state, head_tx, data_tx))?;

    let deadline = Instant::now() + CONNECT_TIMEOUT * 2;
    let info = loop {
        match head_rx.recv_timeout(POLL) {
            Ok(Ok(info)) => break info,
            Ok(Err(e)) => return Err(EngineError::Stream(e)),
            Err(RecvTimeoutError::Disconnected) => return Err(EngineError::Stream("connection closed".into())),
            Err(RecvTimeoutError::Timeout) => {
                if state.abort.load(Ordering::Relaxed) {
                    return Err(EngineError::Stream("aborted".into()));
                }
                if Instant::now() > deadline {
                    state.abort.store(true, Ordering::Relaxed);
                    return Err(EngineError::Stream("connection timed out".into()));
                }
            }
        }
    };
    Ok((StreamReader { rx: data_rx, chunk: Vec::new(), offset: 0, state }, info))
}

fn run(
    url: &str,
    state: &StreamState,
    head_tx: Sender<Result<StreamInfo, String>>,
    data_tx: Sender<io::Result<Vec<u8>>>,
) {
    let (headers, mut body) = match connect(url) {
        Ok(r) => r,
        Err(e) => {
            let _ = head_tx.send(Err(e));
            return;
        }
    };
    let header = |name: &str| headers.get(name).map(|v| v.trim().to_string());
    let content_type = header("content-type");
    let info = StreamInfo {
        hint: content_type.as_deref().and_then(hint_for),
        name: header("icy-name").filter(|n| !n.is_empty()),
        bitrate: header("icy-br").and_then(|b| b.split(',').next()?.trim().parse().ok()),
        content_type,
    };
    let metaint = header("icy-metaint").and_then(|m| m.parse::<usize>().ok()).filter(|&m| m > 0);
    if head_tx.send(Ok(info)).is_err() {
        return;
    }

    let mut icy = metaint.map(IcyParser::new);
    let mut buf = vec![0u8; CHUNK];
    loop {
        if state.abort.load(Ordering::Relaxed) {
            return;
        }
        let n = match body.read(&mut buf) {
            Ok(0) => return, // dropping the sender signals end of stream
            Ok(n) => n,
            Err(e) => {
                let _ = data_tx.send(Err(e));
                return;
            }
        };
        let audio = match icy.as_mut() {
            Some(p) => {
                let (audio, title) = p.feed(&buf[..n]);
                if let Some(t) = title {
                    *state.title.lock().unwrap() = Some(t);
                }
                audio
            }
            None => buf[..n].to_vec(),
        };
        if !audio.is_empty() && data_tx.send(Ok(audio)).is_err() {
            return; // engine closed the stream
        }
    }
}

const USER_AGENT: &str = concat!("KyaraPlayer/", env!("CARGO_PKG_VERSION"));
const MAX_REDIRECTS: usize = 5;

type Headers = HashMap<String, String>;
type Body = Box<dyn Read + Send>;

/// Open `url`, following redirects. Plain HTTP uses a minimal HTTP/1.0 client because
/// classic Shoutcast servers answer `ICY 200 OK`, which HTTP libraries reject; it also
/// gives every read a timeout. HTTPS goes through ureq.
fn connect(url: &str) -> Result<(Headers, Body), String> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let next = if url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
            return connect_https(&url);
        } else {
            match connect_http(&url)? {
                Http::Ok(headers, body) => return Ok((headers, body)),
                Http::Redirect(location) => location,
                // Rare for streams; let the full client deal with it.
                Http::Chunked => return connect_https(&url),
            }
        };
        url = next;
    }
    Err("too many redirects".into())
}

fn connect_https(url: &str) -> Result<(Headers, Body), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(CONNECT_TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .into();
    let response = agent.get(url).header("Icy-MetaData", "1").call().map_err(|e| e.to_string())?;
    let headers = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string())))
        .collect();
    Ok((headers, Box::new(response.into_body().into_reader())))
}

enum Http {
    Ok(Headers, Body),
    Redirect(String),
    Chunked,
}

fn connect_http(url: &str) -> Result<Http, String> {
    let uri: ureq::http::Uri = url.parse().map_err(|e| format!("invalid address: {e}"))?;
    let host = uri.host().ok_or("address has no host")?;
    let port = uri.port_u16().unwrap_or(80);
    let path = uri.path_and_query().map_or("/", |p| p.as_str());

    let addrs = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?;
    let mut last_err = format!("{host}: no address");
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_err = format!("{host}: {e}"),
        }
    }
    let mut stream = stream.ok_or(last_err)?;
    stream.set_read_timeout(Some(STALL_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(|e| e.to_string())?;
    let host_header = if port == 80 { host.to_string() } else { format!("{host}:{port}") };
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: {host_header}\r\nUser-Agent: {USER_AGENT}\r\n\
         Accept: */*\r\nIcy-MetaData: 1\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let (status, headers) = read_head(&mut reader).map_err(|e| e.to_string())?;
    match status {
        200 => {}
        301 | 302 | 303 | 307 | 308 => {
            let location = headers.get("location").ok_or("redirect without location")?;
            return Ok(Http::Redirect(resolve_location(&uri, location)));
        }
        code => return Err(format!("server answered {code}")),
    }
    if headers.get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        return Ok(Http::Chunked);
    }
    Ok(Http::Ok(headers, Box::new(reader)))
}

/// Status line (`HTTP/1.x NNN ...` or `ICY NNN ...`) and headers, lowercased.
fn read_head(reader: &mut impl BufRead) -> io::Result<(u16, Headers)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut line = Vec::new();
    let mut next_line = |reader: &mut dyn BufRead| -> io::Result<String> {
        line.clear();
        // Cap header lines; this is a stream, not a web page.
        reader.take(8192).read_until(b'\n', &mut line)?;
        Ok(String::from_utf8_lossy(&line).trim_end().to_string())
    };
    let status_line = next_line(reader)?;
    let mut parts = status_line.split_whitespace();
    let proto = parts.next().unwrap_or_default();
    if !(proto.starts_with("HTTP/") || proto == "ICY") {
        return Err(bad("not an HTTP or ICY response"));
    }
    let status = parts.next().and_then(|c| c.parse().ok()).ok_or_else(|| bad("bad status line"))?;
    let mut headers = Headers::new();
    for _ in 0..100 {
        let l = next_line(reader)?;
        if l.is_empty() {
            return Ok((status, headers));
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Err(bad("too many headers"))
}

fn resolve_location(base: &ureq::http::Uri, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base.authority().map_or("", |a| a.as_str());
    if location.starts_with('/') {
        format!("{scheme}://{authority}{location}")
    } else {
        let dir = base.path().rsplit_once('/').map_or("", |(d, _)| d);
        format!("{scheme}://{authority}{dir}/{location}")
    }
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let started = Instant::now();
        while self.offset >= self.chunk.len() {
            match self.rx.recv_timeout(POLL) {
                Ok(Ok(chunk)) => {
                    self.chunk = chunk;
                    self.offset = 0;
                }
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
                Err(RecvTimeoutError::Timeout) => {
                    if self.state.abort.load(Ordering::Relaxed) {
                        // Not `Interrupted`: readers retry that kind forever.
                        return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "stream aborted"));
                    }
                    if started.elapsed() > STALL_TIMEOUT {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "stream stalled"));
                    }
                }
            }
        }
        let n = out.len().min(self.chunk.len() - self.offset);
        out[..n].copy_from_slice(&self.chunk[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        self.state.abort.store(true, Ordering::Relaxed);
    }
}

/// What an address points to.
#[derive(Debug)]
pub enum Remote {
    /// Text of a `.pls`/`.m3u` playlist, to be parsed by the caller.
    Playlist(String),
    /// Audio: play the address itself.
    Audio,
}

const PLAYLIST_TYPES: &[&str] = &[
    "audio/x-scpls", "application/pls+xml", "audio/x-mpegurl", "audio/mpegurl",
    "application/x-mpegurl", "application/vnd.apple.mpegurl",
];
const PLAYLIST_EXTENSIONS: &[&str] = &[".pls", ".m3u", ".m3u8"];
const MAX_PLAYLIST: u64 = 256 * 1024;

/// Find out whether `url` is a playlist (radio directories usually link those) or audio.
/// Blocking; call it off the UI thread.
pub fn resolve(url: &str) -> Result<Remote, EngineError> {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    let by_extension = PLAYLIST_EXTENSIONS.iter().any(|e| path.ends_with(e));
    let (headers, body) = connect(url).map_err(EngineError::Stream)?;
    let mime = headers
        .get("content-type")
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !by_extension && !PLAYLIST_TYPES.contains(&mime.as_str()) {
        return Ok(Remote::Audio); // dropping the body closes the connection
    }
    let mut bytes = Vec::new();
    body.take(MAX_PLAYLIST).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if text.contains("#EXT-X-") {
        return Err(EngineError::Unsupported("HLS streams".into()));
    }
    Ok(Remote::Playlist(text))
}

fn hint_for(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/mpeg3" => "mp3",
        "audio/aac" | "audio/aacp" | "audio/x-aac" => "aac",
        "audio/ogg" | "application/ogg" | "audio/vorbis" | "audio/opus" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        _ => return None,
    })
}

/// Splits an ICY stream (`icy-metaint` audio bytes, then a length byte and
/// `len * 16` bytes of metadata, repeated) into audio and `StreamTitle` values.
struct IcyParser {
    metaint: usize,
    /// Audio bytes left before the next metadata block.
    until_meta: usize,
    /// Metadata block being collected: (expected length, bytes so far).
    meta: Option<(usize, Vec<u8>)>,
}

impl IcyParser {
    fn new(metaint: usize) -> Self {
        Self { metaint, until_meta: metaint, meta: None }
    }

    fn feed(&mut self, mut data: &[u8]) -> (Vec<u8>, Option<String>) {
        let mut audio = Vec::with_capacity(data.len());
        let mut title = None;
        while !data.is_empty() {
            if let Some((len, block)) = self.meta.as_mut() {
                let take = (*len - block.len()).min(data.len());
                block.extend_from_slice(&data[..take]);
                data = &data[take..];
                if block.len() == *len {
                    if let Some(t) = parse_title(block) {
                        title = Some(t);
                    }
                    self.meta = None;
                    self.until_meta = self.metaint;
                }
            } else if self.until_meta == 0 {
                let len = data[0] as usize * 16;
                data = &data[1..];
                if len == 0 {
                    self.until_meta = self.metaint;
                } else {
                    self.meta = Some((len, Vec::with_capacity(len)));
                }
            } else {
                let take = self.until_meta.min(data.len());
                audio.extend_from_slice(&data[..take]);
                data = &data[take..];
                self.until_meta -= take;
            }
        }
        (audio, title)
    }
}

/// `StreamTitle='Artist - Song';StreamUrl='...';` (NUL-padded, often Latin-1).
fn parse_title(block: &[u8]) -> Option<String> {
    let end = block.iter().position(|&b| b == 0).unwrap_or(block.len());
    let text = match std::str::from_utf8(&block[..end]) {
        Ok(s) => s.to_string(),
        Err(_) => block[..end].iter().map(|&b| b as char).collect(),
    };
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let stop = rest.find("';").unwrap_or_else(|| rest.trim_end_matches('\'').len());
    Some(rest[..stop].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icy_stream(metaint: usize, blocks: &[(&[u8], &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (audio, meta) in blocks {
            assert_eq!(audio.len(), metaint);
            out.extend_from_slice(audio);
            let mut m = meta.as_bytes().to_vec();
            m.resize(m.len().div_ceil(16) * 16, 0);
            out.push((m.len() / 16) as u8);
            out.extend_from_slice(&m);
        }
        out
    }

    #[test]
    fn icy_parser_strips_metadata_across_chunk_boundaries() {
        let data = icy_stream(
            8,
            &[(b"AAAAAAAA", "StreamTitle='Band - Song';StreamUrl='';"), (b"BBBBBBBB", "")],
        );
        // Feed one byte at a time: the worst case for boundary handling.
        let mut p = IcyParser::new(8);
        let mut audio = Vec::new();
        let mut titles = Vec::new();
        for b in &data {
            let (a, t) = p.feed(std::slice::from_ref(b));
            audio.extend(a);
            titles.extend(t);
        }
        assert_eq!(audio, b"AAAAAAAABBBBBBBB");
        assert_eq!(titles, vec!["Band - Song".to_string()]);
    }

    #[test]
    fn title_parsing_handles_latin1_and_missing_terminator() {
        assert_eq!(parse_title(b"StreamTitle='Caf\xe9';\0\0").as_deref(), Some("Café"));
        assert_eq!(parse_title(b"StreamTitle='x'").as_deref(), Some("x"));
        assert_eq!(parse_title(b"StreamUrl='y';"), None);
    }

    #[test]
    fn reads_icy_and_http_heads() {
        let mut icy = io::Cursor::new(b"ICY 200 OK\r\nicy-name: Radio\r\nicy-metaint:8192\r\n\r\nAUDIO".to_vec());
        let (status, h) = read_head(&mut icy).unwrap();
        assert_eq!((status, h["icy-name"].as_str(), h["icy-metaint"].as_str()), (200, "Radio", "8192"));
        let mut rest = String::new();
        icy.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "AUDIO");
        let mut http = io::Cursor::new(b"HTTP/1.1 302 Found\nLocation: /live\n\n".to_vec());
        let (status, h) = read_head(&mut http).unwrap();
        assert_eq!((status, h["location"].as_str()), (302, "/live"));
        assert!(read_head(&mut io::Cursor::new(b"<html>".to_vec())).is_err());
    }

    #[test]
    fn resolves_redirect_locations() {
        let base: ureq::http::Uri = "http://a.example:8000/dir/stream?x=1".parse().unwrap();
        assert_eq!(resolve_location(&base, "/live"), "http://a.example:8000/live");
        assert_eq!(resolve_location(&base, "other"), "http://a.example:8000/dir/other");
        assert_eq!(resolve_location(&base, "https://b/x"), "https://b/x");
    }

    #[test]
    fn recognizes_urls_and_mime_types() {
        assert!(is_url("HTTP://radio.example/stream"));
        assert!(is_url("https://a"));
        assert!(!is_url("/home/u/http://x"));
        assert_eq!(hint_for("audio/aacp; charset=x"), Some("aac"));
        assert_eq!(hint_for("text/html"), None);
    }
}
