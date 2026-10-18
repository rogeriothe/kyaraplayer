//! Terminal test player for the audio engine.
//!
//! cargo run -p audio-engine --example play -- <file>
//! Commands (type + Enter): p pause/resume · s stop · x play again · f/b ±5 s
//!                          + / - volume · l / r balance · q quit

use std::io::{BufRead, Write};
use std::time::Duration;

use audio_engine::{Engine, Event, PlayerState};

fn main() {
    env_logger::init();
    let path = std::env::args().nth(1).expect("usage: play <file>");

    let engine = Engine::new();
    let events = engine.events();
    engine.play(&path);

    let (line_tx, line_rx) = crossbeam_channel::unbounded::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    loop {
        crossbeam_channel::select! {
            recv(events) -> e => match e {
                Ok(Event::TrackStarted { track }) => println!(
                    "\nplaying {} — {} Hz, {} ch, {}",
                    track.path.display(), track.sample_rate, track.channels,
                    track.duration.map(fmt_time).unwrap_or_else(|| "?".into()),
                ),
                Ok(Event::TrackEnded) => { println!("\nend of track"); break; }
                Ok(Event::Error { message }) => { eprintln!("\nerror: {message}"); break; }
                Ok(Event::StreamTitle { title }) => println!("\nnow: {title}"),
                Ok(Event::Connecting { path }) => println!("connecting to {}", path.display()),
                Ok(Event::StateChanged { .. }) => {}
                Err(_) => break,
            },
            recv(line_rx) -> line => {
                let Ok(line) = line else { break };
                let st = engine.status();
                match line.trim() {
                    "p" => engine.toggle_pause(),
                    "s" => engine.stop(),
                    "x" => engine.play(&path),
                    "f" => engine.seek(st.position + 5.0),
                    "b" => engine.seek((st.position - 5.0).max(0.0)),
                    "+" => engine.set_volume(st.volume + 0.1),
                    "-" => engine.set_volume(st.volume - 0.1),
                    "l" => engine.set_balance(st.balance - 0.25),
                    "r" => engine.set_balance(st.balance + 0.25),
                    "q" => break,
                    _ => {}
                }
            },
            default(Duration::from_millis(250)) => {
                let st = engine.status();
                let total = st.track.as_ref().and_then(|t| t.duration).map(fmt_time).unwrap_or_default();
                let state = match st.state {
                    PlayerState::Playing => "▶",
                    PlayerState::Paused => "‖",
                    PlayerState::Stopped => "■",
                };
                print!(
                    "\r{state} {} / {total}  vol {:>3.0}%  bal {:+.2}   ",
                    fmt_time(st.position), st.volume * 100.0, st.balance,
                );
                let _ = std::io::stdout().flush();
            }
        }
    }
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
