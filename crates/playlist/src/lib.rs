//! Playlist model: ordered entries with stable ids, current track, shuffle and repeat.

use std::path::{Path, PathBuf};

use serde::Serialize;

pub mod formats;
pub mod meta;

/// File extensions the player can decode.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "mp2", "mp1", "flac", "ogg", "oga", "opus", "wav", "aiff", "aif", "m4a", "mp4", "aac",
    "alac", "mka", "webm", "caf",
];

/// Playlist file extensions understood by [`formats`].
pub const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];

pub type EntryId = u64;

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: EntryId,
    pub path: PathBuf,
    /// Display title; the file name until tags are read.
    pub title: String,
    pub duration: Option<f64>,
    /// Tags were read (successfully or not).
    pub meta_loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    FileName,
    Path,
}

#[derive(Debug, Default)]
pub struct Playlist {
    entries: Vec<Entry>,
    current: Option<EntryId>,
    next_id: EntryId,
    shuffle: bool,
    repeat: bool,
    /// Play order while shuffling (a permutation of entry ids).
    shuffle_order: Vec<EntryId>,
    rng: u64,
}

pub fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn file_title(path: &Path) -> String {
    // Streams show their address until the server sends a title.
    if let Some(url) = path.to_str().filter(|p| p.contains("://")) {
        return url.to_string();
    }
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Playlist {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self { rng: seed | 1, ..Default::default() }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: EntryId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn index_of(&self, id: EntryId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    pub fn current(&self) -> Option<&Entry> {
        self.current.and_then(|id| self.get(id))
    }

    pub fn current_id(&self) -> Option<EntryId> {
        self.current
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> bool {
        self.repeat
    }

    pub fn set_repeat(&mut self, on: bool) {
        self.repeat = on;
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if on {
            self.reshuffle();
        }
    }

    /// Sum of known durations, and whether every duration is known.
    pub fn total_duration(&self) -> (f64, bool) {
        let total = self.entries.iter().filter_map(|e| e.duration).sum();
        (total, self.entries.iter().all(|e| e.duration.is_some()))
    }

    /// Append (or insert at `at`) files. Directories are scanned recursively and
    /// playlist files are expanded. Returns the new ids in order.
    pub fn add_paths(&mut self, paths: &[PathBuf], at: Option<usize>) -> Vec<EntryId> {
        let mut files = Vec::new();
        for p in paths {
            collect(p, &mut files);
        }
        self.insert(files, at)
    }

    /// Add already-resolved items (a downloaded playlist, for instance).
    pub fn add_items(&mut self, items: Vec<formats::Item>, at: Option<usize>) -> Vec<EntryId> {
        self.insert(items.into_iter().map(|i| (i.path, i.title, i.duration)).collect(), at)
    }

    fn insert(&mut self, files: Vec<(PathBuf, Option<String>, Option<f64>)>, at: Option<usize>) -> Vec<EntryId> {
        let new: Vec<Entry> = files
            .into_iter()
            .map(|(path, title, duration)| {
                self.next_id += 1;
                Entry {
                    id: self.next_id,
                    title: title.unwrap_or_else(|| file_title(&path)),
                    path,
                    duration,
                    meta_loaded: false,
                }
            })
            .collect();
        let ids: Vec<EntryId> = new.iter().map(|e| e.id).collect();
        let at = at.unwrap_or(self.entries.len()).min(self.entries.len());
        self.entries.splice(at..at, new);
        if self.shuffle {
            self.reshuffle();
        }
        ids
    }

    pub fn remove(&mut self, ids: &[EntryId]) {
        self.entries.retain(|e| !ids.contains(&e.id));
        self.after_removal();
    }

    /// Keep only `ids` ("crop" in the classic UI).
    pub fn crop(&mut self, ids: &[EntryId]) {
        self.entries.retain(|e| ids.contains(&e.id));
        self.after_removal();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.after_removal();
    }

    /// Remove entries whose file no longer exists. Returns how many were removed.
    pub fn remove_dead(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.path.exists());
        self.after_removal();
        before - self.entries.len()
    }

    /// Remove later duplicates of the same path.
    pub fn remove_duplicates(&mut self) -> usize {
        let before = self.entries.len();
        let mut seen = std::collections::HashSet::new();
        self.entries.retain(|e| seen.insert(e.path.clone()));
        self.after_removal();
        before - self.entries.len()
    }

    fn after_removal(&mut self) {
        if self.current.is_some_and(|id| self.get(id).is_none()) {
            self.current = None;
        }
        self.shuffle_order.retain(|id| self.entries.iter().any(|e| e.id == *id));
    }

    /// Move `ids` (kept in their relative order) so the first one lands at `to`,
    /// where `to` is an index in the list *before* the move.
    pub fn move_to(&mut self, ids: &[EntryId], to: usize) {
        let to = to.min(self.entries.len());
        let before = self.entries[..to].iter().filter(|e| ids.contains(&e.id)).count();
        let (moving, mut rest): (Vec<Entry>, Vec<Entry>) =
            self.entries.drain(..).partition(|e| ids.contains(&e.id));
        let at = (to - before).min(rest.len());
        rest.splice(at..at, moving);
        self.entries = rest;
    }

    pub fn sort(&mut self, key: SortKey) {
        let k = |e: &Entry| match key {
            SortKey::Title => e.title.to_lowercase(),
            SortKey::FileName => e.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
            SortKey::Path => e.path.to_string_lossy().to_lowercase(),
        };
        self.entries.sort_by_cached_key(k);
    }

    pub fn reverse(&mut self) {
        self.entries.reverse();
    }

    pub fn randomize(&mut self) {
        for i in (1..self.entries.len()).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            self.entries.swap(i, j);
        }
    }

    pub fn set_current(&mut self, id: EntryId) -> Option<&Entry> {
        self.get(id)?;
        self.current = Some(id);
        self.current()
    }

    /// Advance. `auto` is true when a track finished by itself: without repeat the
    /// list then stops at its end, while a manual "next" wraps around.
    pub fn next(&mut self, auto: bool) -> Option<&Entry> {
        let order = self.order();
        if order.is_empty() {
            return None;
        }
        let pos = self.current.and_then(|id| order.iter().position(|x| *x == id));
        let next = match pos {
            None => 0,
            Some(p) if p + 1 < order.len() => p + 1,
            Some(_) if auto && !self.repeat => return None,
            Some(_) => {
                if self.shuffle {
                    self.reshuffle();
                }
                0
            }
        };
        let id = self.order()[next];
        self.set_current(id)
    }

    pub fn prev(&mut self) -> Option<&Entry> {
        let order = self.order();
        if order.is_empty() {
            return None;
        }
        let pos = self.current.and_then(|id| order.iter().position(|x| *x == id)).unwrap_or(0);
        let id = order[if pos == 0 { order.len() - 1 } else { pos - 1 }];
        self.set_current(id)
    }

    /// Move the current track by `delta` positions in list order (Num1/Num3 in the original).
    pub fn jump(&mut self, delta: isize) -> Option<&Entry> {
        let len = self.entries.len() as isize;
        if len == 0 {
            return None;
        }
        let pos = self.current.and_then(|id| self.index_of(id)).unwrap_or(0) as isize;
        let id = self.entries[(pos + delta).clamp(0, len - 1) as usize].id;
        self.set_current(id)
    }

    fn order(&self) -> Vec<EntryId> {
        if self.shuffle && self.shuffle_order.len() == self.entries.len() {
            self.shuffle_order.clone()
        } else {
            self.entries.iter().map(|e| e.id).collect()
        }
    }

    /// New random order that starts with the current track, so toggling shuffle
    /// does not jump.
    fn reshuffle(&mut self) {
        let mut order: Vec<EntryId> = self.entries.iter().map(|e| e.id).collect();
        for i in (1..order.len()).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        if let Some(cur) = self.current {
            if let Some(p) = order.iter().position(|x| *x == cur) {
                order.swap(0, p);
            }
        }
        self.shuffle_order = order;
    }

    fn next_random(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Expand a dropped/added path into playable files with optional title/duration hints.
fn collect(path: &Path, out: &mut Vec<(PathBuf, Option<String>, Option<f64>)>) {
    if path.is_dir() {
        let Ok(read) = std::fs::read_dir(path) else { return };
        let mut children: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        for child in children {
            // Playlists inside folders are skipped: their tracks are usually there too.
            if child.is_dir() || has_extension(&child, AUDIO_EXTENSIONS) {
                collect(&child, out);
            }
        }
    } else if has_extension(path, PLAYLIST_EXTENSIONS) {
        if let Ok(items) = formats::read(path) {
            out.extend(items.into_iter().map(|i| (i.path, i.title, i.duration)));
        }
    } else {
        out.push((path.to_path_buf(), None, None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> (Playlist, Vec<EntryId>) {
        let mut p = Playlist::new();
        let paths: Vec<PathBuf> = (0..n).map(|i| PathBuf::from(format!("/m/{i}.mp3"))).collect();
        let ids = p.add_paths(&paths, None);
        (p, ids)
    }

    fn titles(p: &Playlist) -> Vec<String> {
        p.entries().iter().map(|e| e.title.clone()).collect()
    }

    #[test]
    fn next_stops_at_end_unless_repeat_or_manual() {
        let (mut p, ids) = list(3);
        assert_eq!(p.next(true).unwrap().id, ids[0]);
        p.next(true);
        p.next(true);
        assert!(p.next(true).is_none());
        assert_eq!(p.next(false).unwrap().id, ids[0]);
        p.set_current(ids[2]);
        p.set_repeat(true);
        assert_eq!(p.next(true).unwrap().id, ids[0]);
    }

    #[test]
    fn prev_wraps_to_last() {
        let (mut p, ids) = list(3);
        p.set_current(ids[0]);
        assert_eq!(p.prev().unwrap().id, ids[2]);
    }

    #[test]
    fn shuffle_visits_every_track_once_starting_at_current() {
        let (mut p, ids) = list(20);
        p.set_current(ids[7]);
        p.set_shuffle(true);
        let mut seen = vec![ids[7]];
        while let Some(e) = p.next(true) {
            seen.push(e.id);
        }
        seen.sort();
        assert_eq!(seen, ids);
    }

    #[test]
    fn move_to_keeps_relative_order() {
        let (mut p, ids) = list(5); // 0 1 2 3 4
        p.move_to(&[ids[0], ids[1]], 4); // before "4"
        assert_eq!(titles(&p), ["2", "3", "0", "1", "4"]);
        p.move_to(&[ids[4]], 0);
        assert_eq!(titles(&p), ["4", "2", "3", "0", "1"]);
        p.move_to(&[ids[2]], 5); // to the end
        assert_eq!(titles(&p), ["4", "3", "0", "1", "2"]);
    }

    #[test]
    fn removing_current_clears_it() {
        let (mut p, ids) = list(3);
        p.set_current(ids[1]);
        p.remove(&[ids[1]]);
        assert!(p.current().is_none());
        p.crop(&[ids[0]]);
        assert_eq!(titles(&p), ["0"]);
    }

    #[test]
    fn insert_at_index() {
        let (mut p, _) = list(2);
        p.add_paths(&[PathBuf::from("/m/x.mp3")], Some(1));
        assert_eq!(titles(&p), ["0", "x", "1"]);
    }

    #[test]
    fn duplicates_and_total_duration() {
        let (mut p, ids) = list(2);
        p.add_paths(&[PathBuf::from("/m/0.mp3")], None);
        assert_eq!(p.remove_duplicates(), 1);
        p.get_mut(ids[0]).unwrap().duration = Some(60.0);
        assert_eq!(p.total_duration(), (60.0, false));
    }
}
