use std::io::BufRead;

use crate::ansi::strip_ansi;

pub struct LineReader<R: BufRead> {
    reader: R,
    strip_ansi: bool,
}

impl<R: BufRead> LineReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            strip_ansi: false,
        }
    }

    pub fn with_ansi_strip(reader: R, strip: bool) -> Self {
        Self {
            reader,
            strip_ansi: strip,
        }
    }
}

impl<R: BufRead> Iterator for LineReader<R> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => {
                let line = buf.trim_end_matches('\n').trim_end_matches('\r');
                if self.strip_ansi {
                    Some(strip_ansi(line))
                } else {
                    Some(line.to_string())
                }
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_lines_from_buffered_input() {
        let input = Cursor::new("line1\nline2\nline3\n");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn handles_empty_input() {
        let input = Cursor::new("");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert!(lines.is_empty());
    }

    #[test]
    fn handles_line_without_trailing_newline() {
        let input = Cursor::new("no newline");
        let reader = LineReader::new(input);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["no newline"]);
    }

    #[test]
    fn strips_ansi_when_enabled() {
        let input = Cursor::new("\x1b[31mred\x1b[0m\n");
        let reader = LineReader::with_ansi_strip(input, true);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["red"]);
    }

    #[test]
    fn preserves_ansi_when_disabled() {
        let input = Cursor::new("\x1b[31mred\x1b[0m\n");
        let reader = LineReader::with_ansi_strip(input, false);
        let lines: Vec<String> = reader.collect();
        assert_eq!(lines, vec!["\x1b[31mred\x1b[0m"]);
    }
}
