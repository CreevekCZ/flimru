//! A bounded plain-text transcript of the same PTY output used by the console.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const CAPACITY: usize = 1024 * 1024;
pub const MAX_READ: usize = 64 * 1024;

#[derive(Default)]
pub struct SessionLog {
    parser: vte::Parser,
    text: Transcript,
}

#[derive(Default)]
struct Transcript {
    bytes: VecDeque<u8>,
    start: u64,
    carriage_return: bool,
}

#[derive(Deserialize, Serialize)]
pub struct LogPage {
    pub text: String,
    /// UTF-8 byte offset in the normalized transcript.
    pub cursor: u64,
    pub end_cursor: u64,
    pub truncated: bool,
}

impl SessionLog {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.text, bytes);
        self.text.trim();
    }

    pub fn finish(&mut self) {
        if std::mem::take(&mut self.text.carriage_return) {
            self.text.bytes.push_back(b'\n');
            self.text.trim();
        }
        self.parser = vte::Parser::new();
    }

    /// Without a cursor, return the latest page. With a cursor, resume at the
    /// earliest still-available byte and report any retention gap.
    pub fn read(&self, cursor: Option<u64>, limit: usize, until: Option<u64>) -> LogPage {
        let Transcript { bytes, start, .. } = &self.text;
        let limit = limit.clamp(4, MAX_READ);
        let end_cursor = *start + bytes.len() as u64;
        let end = until.unwrap_or(end_cursor).clamp(*start, end_cursor);
        let requested = cursor.unwrap_or(end.saturating_sub(limit as u64));
        let mut from = (requested.clamp(*start, end) - start) as usize;
        let end_index = (end - start) as usize;
        while from < end_index && bytes.get(from).is_some_and(|byte| continuation(*byte)) {
            from += 1;
        }
        let mut to = (from + limit).min(end_index);
        while to > from && bytes.get(to).is_some_and(|byte| continuation(*byte)) {
            to -= 1;
        }
        let text: Vec<_> = bytes.range(from..to).copied().collect();
        LogPage {
            text: String::from_utf8_lossy(&text).into_owned(),
            cursor: *start + to as u64,
            end_cursor: end,
            truncated: requested < *start || (cursor.is_none() && from != 0),
        }
    }
}

impl Transcript {
    fn trim(&mut self) {
        let mut trim = self.bytes.len().saturating_sub(CAPACITY);
        while self.bytes.get(trim).is_some_and(|byte| continuation(*byte)) {
            trim += 1;
        }
        self.bytes.drain(..trim);
        self.start += trim as u64;
    }
}

fn continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

impl vte::Perform for Transcript {
    fn print(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if std::mem::take(&mut self.carriage_return) {
            self.bytes.push_back(b'\n');
        }
        self.bytes.extend(ch.encode_utf8(&mut [0; 4]).as_bytes());
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.carriage_return = true,
            b'\n' => {
                self.carriage_return = false;
                self.bytes.push_back(b'\n');
            }
            b'\t' => {
                if std::mem::take(&mut self.carriage_return) {
                    self.bytes.push_back(b'\n');
                }
                self.bytes.push_back(byte);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_cursor_reads_preserve_unicode_and_strip_terminal_commands() {
        let mut log = SessionLog::default();
        log.feed(b"build\r");
        log.feed(b"\r\n\x1b[31m\xf0\x9f");
        log.feed(b"\x9a\x80\x1b[0m\x1b]52;c;ignored\x07\r");
        log.feed(b"next\tline\r");
        log.finish();
        let first = log.read(Some(0), 9, None);
        assert_eq!(first.text, "build\n");
        let rest = log.read(Some(first.cursor), MAX_READ, Some(first.end_cursor));
        assert_eq!(rest.text, "🚀\nnext\tline\n");
        assert_eq!(rest.cursor, rest.end_cursor);
        // A tail read can start in the middle of a Unicode scalar.
        assert_eq!(log.read(None, 4, Some(11)).text, "\n");
        for _ in 0..=CAPACITY / 8192 {
            log.feed(&[b'.'; 8192]);
        }
        let resumed = log.read(Some(rest.cursor), 12, None);
        assert!(resumed.truncated);
        assert_eq!(resumed.text, "............");
        assert!(log.text.bytes.len() <= CAPACITY);
    }
}
