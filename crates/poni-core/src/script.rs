//! The Script page's teleprompter: a script cut into lines that fit the
//! island, read out at a pace in words a minute.
//!
//! Time is a monotonic clock in seconds supplied by the caller, and so is
//! measuring text (a closure), so the line breaks here are the ones the
//! island draws. The reader moves a line at a time: each line stays for as
//! long as its words take at the pace, so the app sleeps between lines.

use serde::{Deserialize, Serialize};

pub const DEFAULT_WPM: u32 = 140;
pub const MIN_WPM: u32 = 60;
pub const MAX_WPM: u32 = 300;
pub const WPM_STEP: u32 = 10;
/// The longest script kept, in characters: about 10,000 words, an hour at
/// 160 a minute.
pub const MAX_CHARS: usize = 60_000;
/// A text file bigger than this isn't a script (bytes).
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// A line never goes by faster than this (seconds), however few its words.
const MIN_LINE_S: f64 = 0.8;
/// The extra beat after a paragraph's last line (seconds).
const PARA_REST_S: f64 = 0.6;

/// What the store keeps: the script and its pace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Script {
    /// The words to read. A line break starts a new line, a blank line a
    /// new paragraph ([`clean`] keeps it to one).
    pub text: String,
    /// Reading pace, words a minute.
    pub wpm: u32,
}

impl Default for Script {
    fn default() -> Self {
        Self { text: String::new(), wpm: DEFAULT_WPM }
    }
}

impl Script {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The pace, within [`MIN_WPM`]..=[`MAX_WPM`] (the file may say anything).
    pub fn pace(&self) -> u32 {
        self.wpm.clamp(MIN_WPM, MAX_WPM)
    }
}

/// One line as the island shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub words: usize,
    /// Its first word's place in the whole script. The reader keeps this, so
    /// lines cut at another width put it back on the same words.
    pub first_word: usize,
    /// The paragraph's last line: a beat of rest follows it.
    pub para_end: bool,
}

impl Line {
    /// How long the line stays at `wpm` (seconds).
    pub fn secs(&self, wpm: u32) -> f64 {
        let read = self.words as f64 * 60.0 / f64::from(wpm.max(1));
        read.max(MIN_LINE_S) + if self.para_end { PARA_REST_S } else { 0.0 }
    }
}

/// Tidy pasted or loaded text into a script: one kind of line break, single
/// spaces, no control characters, at most one blank line between
/// paragraphs, at most [`MAX_CHARS`] characters (cut between words).
pub fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len().min(MAX_CHARS + 8));
    let mut blank = false;
    for line in text.split('\n') {
        let words: Vec<&str> = line.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        out.push_str(&words.join(" "));
        if out.chars().count() > MAX_CHARS {
            break;
        }
    }
    if out.chars().count() > MAX_CHARS {
        let cut: String = out.chars().take(MAX_CHARS).collect();
        // Back to the last whole word.
        let end = cut.rfind(char::is_whitespace).unwrap_or(cut.len());
        out = cut[..end].trim_end().to_string();
    }
    out
}

/// A text file's bytes as a script: UTF-8 (with or without its mark) or
/// UTF-16 with a byte-order mark. `None` for anything that looks binary or
/// holds no words.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&[b0, b1]| u16::from_le_bytes([b0, b1])).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&[b0, b1]| u16::from_be_bytes([b0, b1])).collect();
        String::from_utf16_lossy(&units)
    } else if bytes.contains(&0) {
        return None;
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let text = clean(&text);
    (!text.is_empty()).then_some(text)
}

/// A file a dropped script can come in: plain text or Markdown.
pub fn is_text_file(path: &str) -> bool {
    let ext = path.rsplit(['/', '\\']).next().and_then(|name| name.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
    matches!(ext.as_deref(), Some("txt" | "text" | "md" | "markdown"))
}

/// Cut a [`clean`] script into lines no wider than `width`, breaking
/// between words. `measure` gives a piece of text's width in the same units;
/// each word is measured on its own and a space as the difference between
/// "a a" and "aa". A word wider than the line gets a line to itself (the
/// island clips it).
pub fn wrap(text: &str, width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<Line> {
    let space = (measure("a a") - measure("aa")).max(0.0);
    let mut lines: Vec<Line> = Vec::new();
    let mut word_no = 0;
    for para in text.split("\n\n") {
        let before = lines.len();
        for row in para.split('\n') {
            let mut cur = String::new();
            let (mut cur_w, mut cur_words, mut first) = (0.0f32, 0usize, word_no);
            for word in row.split(' ').filter(|w| !w.is_empty()) {
                let w = measure(word);
                if cur_words > 0 && cur_w + space + w > width {
                    lines.push(Line { text: std::mem::take(&mut cur), words: cur_words, first_word: first, para_end: false });
                    (cur_w, cur_words) = (0.0, 0);
                }
                if cur_words == 0 {
                    first = word_no;
                } else {
                    cur.push(' ');
                    cur_w += space;
                }
                cur.push_str(word);
                cur_w += w;
                cur_words += 1;
                word_no += 1;
            }
            if cur_words > 0 {
                lines.push(Line { text: cur, words: cur_words, first_word: first, para_end: false });
            }
        }
        if lines.len() > before {
            if let Some(last) = lines.last_mut() {
                last.para_end = true;
            }
        }
    }
    lines
}

/// Where the reading is: the line under the camera, and whether it moves on
/// by itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reader {
    /// The current line's first word in the whole script (survives new line breaks).
    pub word: usize,
    /// Moving on by itself: when the current line began (mono seconds).
    started: Option<f64>,
    /// Paused partway: seconds of the current line already gone.
    held: f64,
    /// A reading is under way, from Read until Stop or the end. It keeps
    /// the island open and the pause shortcut on.
    pub active: bool,
    /// The last line has been read; Read starts again from the top.
    pub done: bool,
}

impl Reader {
    pub fn playing(&self) -> bool {
        self.started.is_some()
    }

    /// The current line in `lines`: the last one starting at or before [`Reader::word`].
    pub fn line(&self, lines: &[Line]) -> usize {
        lines.partition_point(|l| l.first_word <= self.word).saturating_sub(1)
    }

    pub fn play(&mut self, now: f64, lines: &[Line]) {
        if lines.is_empty() {
            return;
        }
        if self.done {
            *self = Reader::default();
        }
        self.started = Some(now - self.held);
        self.held = 0.0;
        self.active = true;
    }

    pub fn pause(&mut self, now: f64) {
        if let Some(t0) = self.started.take() {
            self.held = (now - t0).max(0.0);
        }
    }

    pub fn toggle(&mut self, now: f64, lines: &[Line]) {
        if self.playing() {
            self.pause(now);
        } else {
            self.play(now, lines);
        }
    }

    /// Pause and let go of the island; the place stays.
    pub fn stop(&mut self, now: f64) {
        self.pause(now);
        self.active = false;
    }

    /// Back to the first line, still playing if it was.
    pub fn restart(&mut self, now: f64) {
        self.word = 0;
        self.held = 0.0;
        self.done = false;
        if self.started.is_some() {
            self.started = Some(now);
        }
    }

    /// Go to line `i` (clamped): its start, still playing if it was.
    pub fn seek(&mut self, i: usize, lines: &[Line], now: f64) {
        let Some(l) = lines.get(i.min(lines.len().saturating_sub(1))) else { return };
        self.word = l.first_word;
        self.held = 0.0;
        self.done = false;
        if self.started.is_some() {
            self.started = Some(now);
        }
    }

    /// `delta` lines on (or back).
    pub fn step(&mut self, delta: isize, lines: &[Line], now: f64) {
        let i = self.line(lines) as isize + delta;
        self.seek(i.max(0) as usize, lines, now);
    }

    /// Move on by the clock. True when the line changed or the reading ended.
    pub fn tick(&mut self, now: f64, lines: &[Line], wpm: u32) -> bool {
        let Some(mut t0) = self.started else { return false };
        if lines.is_empty() {
            self.started = None;
            return false;
        }
        let mut i = self.line(lines);
        let mut moved = false;
        loop {
            let d = lines[i].secs(wpm);
            if now - t0 < d {
                break;
            }
            if i + 1 >= lines.len() {
                // The last line is read: the reading ends on it.
                self.word = lines[i].first_word;
                self.started = None;
                self.held = 0.0;
                self.active = false;
                self.done = true;
                return true;
            }
            t0 += d;
            i += 1;
            moved = true;
        }
        if moved {
            self.word = lines[i].first_word;
            self.started = Some(t0);
        }
        moved
    }

    /// When the line changes by itself next (mono seconds), while playing.
    pub fn next_change(&self, lines: &[Line], wpm: u32) -> Option<f64> {
        let t0 = self.started?;
        let l = lines.get(self.line(lines))?;
        Some(t0 + l.secs(wpm))
    }

    /// A new pace: the current line keeps the share of it already read.
    pub fn repace(&mut self, now: f64, lines: &[Line], old: u32, new: u32) {
        let Some(l) = lines.get(self.line(lines)) else { return };
        let (was, will) = (l.secs(old), l.secs(new));
        let rescale = |gone: f64| (gone / was).clamp(0.0, 1.0) * will;
        match self.started {
            Some(t0) => self.started = Some(now - rescale(now - t0)),
            None => self.held = rescale(self.held),
        }
    }

    /// Seconds left at `wpm`, from where the reading is to the end.
    pub fn remaining(&self, now: f64, lines: &[Line], wpm: u32) -> f64 {
        if self.done {
            return 0.0;
        }
        let gone = self.started.map_or(self.held, |t0| now - t0);
        let rest: f64 = lines.iter().skip(self.line(lines)).map(|l| l.secs(wpm)).sum();
        (rest - gone).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is one unit wide.
    fn chars(s: &str) -> f32 {
        s.chars().count() as f32
    }

    #[test]
    fn clean_keeps_lines_and_one_blank_line_between_paragraphs() {
        let t = clean("\r\n  Hello   there,\tfriends.\r\nSecond  line\r\n\r\n\r\n\u{7}Next para \n\n");
        assert_eq!(t, "Hello there, friends.\nSecond line\n\nNext para");
        assert_eq!(clean(" \n\t\n"), "");
    }

    #[test]
    fn clean_cuts_long_scripts_between_words() {
        let long = "word ".repeat(MAX_CHARS);
        let t = clean(&long);
        assert!(t.chars().count() <= MAX_CHARS);
        assert!(t.ends_with("word"));
    }

    #[test]
    fn decode_reads_utf8_and_utf16_and_refuses_binary() {
        assert_eq!(decode(b"\xEF\xBB\xBFHalo semua").as_deref(), Some("Halo semua"));
        assert_eq!(decode("Satu\r\nDua".as_bytes()).as_deref(), Some("Satu\nDua"));
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Hi you".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&le).as_deref(), Some("Hi you"));
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Hi".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&be).as_deref(), Some("Hi"));
        assert_eq!(decode(b"\x89PNG\r\n\x1a\n\0\0\0"), None);
        assert_eq!(decode(b"   \n "), None);
    }

    #[test]
    fn text_files_by_extension() {
        assert!(is_text_file(r"C:\Users\me\Desktop\Script.TXT"));
        assert!(is_text_file("/Users/me/intro.md"));
        assert!(!is_text_file(r"C:\a.b\video.mp4"));
        assert!(!is_text_file("README"));
    }

    #[test]
    fn wrap_breaks_between_words_and_marks_paragraph_ends() {
        let lines = wrap("one two three four five\nsix\n\nseven eight", 13.0, chars);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["one two three", "four five", "six", "seven eight"]);
        assert_eq!(lines.iter().map(|l| l.first_word).collect::<Vec<_>>(), [0, 3, 5, 6]);
        assert_eq!(lines.iter().map(|l| l.words).collect::<Vec<_>>(), [3, 2, 1, 2]);
        assert_eq!(lines.iter().map(|l| l.para_end).collect::<Vec<_>>(), [false, false, true, true]);
        assert!(lines.iter().all(|l| chars(&l.text) <= 13.0));
    }

    #[test]
    fn wrap_gives_a_long_word_its_own_line() {
        let lines = wrap("go to https://example.com/a/very/long/path now", 10.0, chars);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["go to", "https://example.com/a/very/long/path", "now"]);
    }

    #[test]
    fn a_line_lasts_its_words_at_the_pace_with_a_floor_and_a_paragraph_rest() {
        let l = Line { text: "a b c d e f g".into(), words: 7, first_word: 0, para_end: false };
        assert!((l.secs(140) - 3.0).abs() < 1e-9);
        let short = Line { words: 1, ..l.clone() };
        assert_eq!(short.secs(140), MIN_LINE_S);
        let end = Line { para_end: true, ..l };
        assert!((end.secs(140) - 3.6).abs() < 1e-9);
    }

    fn lines() -> Vec<Line> {
        // 7 words at 140 a minute = 3 s a line; the last line ends its paragraph (+0.6 s).
        let text = "a b c d e f g\nh i j k l m n\no p q r s t u";
        wrap(text, 100.0, chars)
    }

    #[test]
    fn the_reader_moves_a_line_at_a_time_and_ends_on_the_last() {
        let ls = lines();
        let mut r = Reader::default();
        assert!(!r.tick(10.0, &ls, 140), "paused readers don't move");
        r.play(0.0, &ls);
        assert!(r.active && r.playing());
        assert!(!r.tick(2.9, &ls, 140));
        assert_eq!(r.next_change(&ls, 140), Some(3.0));
        assert!(r.tick(3.0, &ls, 140));
        assert_eq!(r.line(&ls), 1);
        // A late frame skips over a line that went by meanwhile.
        assert!(r.tick(9.1, &ls, 140));
        assert_eq!(r.line(&ls), 2);
        assert!(r.tick(9.0 + 3.6, &ls, 140));
        assert!(r.done && !r.active && !r.playing());
        assert_eq!(r.line(&ls), 2);
        // Read again starts from the top.
        r.play(20.0, &ls);
        assert_eq!(r.line(&ls), 0);
        assert!(!r.done);
    }

    #[test]
    fn pausing_keeps_the_time_already_read() {
        let ls = lines();
        let mut r = Reader::default();
        r.play(0.0, &ls);
        r.pause(2.0);
        assert!(!r.tick(100.0, &ls, 140));
        r.play(50.0, &ls);
        assert_eq!(r.next_change(&ls, 140), Some(51.0));
        r.stop(50.5);
        assert!(!r.active && !r.playing());
    }

    #[test]
    fn seeking_and_stepping_stay_in_range() {
        let ls = lines();
        let mut r = Reader::default();
        r.seek(9, &ls, 0.0);
        assert_eq!(r.line(&ls), 2);
        r.step(-5, &ls, 0.0);
        assert_eq!(r.line(&ls), 0);
        r.step(1, &ls, 0.0);
        assert_eq!(r.line(&ls), 1);
        r.play(0.0, &ls);
        r.restart(4.0);
        assert_eq!((r.line(&ls), r.next_change(&ls, 140)), (0, Some(7.0)));
    }

    #[test]
    fn a_new_pace_keeps_the_share_of_the_line_read() {
        let ls = lines();
        let mut r = Reader::default();
        r.play(0.0, &ls);
        // Half of a 3 s line gone; at 70 a minute the line takes 6 s, so 3 s are left.
        r.repace(1.5, &ls, 140, 70);
        assert_eq!(r.next_change(&ls, 70), Some(4.5));
        assert!((r.remaining(1.5, &ls, 70) - (3.0 + 6.0 + 6.6)).abs() < 1e-9);
    }

    #[test]
    fn the_place_survives_new_line_breaks() {
        let text = "a b c d e f g h i j";
        let wide = wrap(text, 100.0, chars);
        let narrow = wrap(text, 5.0, chars);
        let mut r = Reader::default();
        r.seek(2, &narrow, 0.0);
        assert_eq!(narrow[r.line(&narrow)].text, "g h i");
        assert_eq!(wide[r.line(&wide)].first_word, 0);
        r.seek(0, &wide, 0.0);
        assert_eq!(r.line(&narrow), 0);
    }

    #[test]
    fn the_store_reads_scripts_written_before_it_had_one() {
        let s: Script = serde_json::from_str(r#"{"text":"Hi"}"#).unwrap();
        assert_eq!((s.text.as_str(), s.wpm), ("Hi", DEFAULT_WPM));
        assert_eq!(Script { wpm: 5000, ..Script::default() }.pace(), MAX_WPM);
    }
}
