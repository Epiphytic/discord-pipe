use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, PollWatcher, RecursiveMode, Watcher};

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

pub fn tail_file(
    path: &Path,
    sender: mpsc::Sender<String>,
    strip_ansi_codes: bool,
    shutdown: Arc<AtomicBool>,
) {
    let existed_at_start = path.exists();

    while !path.exists() {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    if existed_at_start && file.seek(SeekFrom::End(0)).is_err() {
        return;
    }

    let (notify_tx, notify_rx) = mpsc::channel();
    let config = Config::default().with_poll_interval(Duration::from_millis(200));

    let mut watcher = match PollWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = notify_tx.send(event);
            }
        },
        config,
    ) {
        Ok(w) => w,
        Err(_) => return,
    };

    let watch_path = path.parent().unwrap_or(Path::new("."));
    if watcher
        .watch(watch_path, RecursiveMode::NonRecursive)
        .is_err()
    {
        return;
    }

    let mut partial = String::new();

    while !shutdown.load(Ordering::Relaxed) {
        match notify_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).unwrap_or(0) == 0 {
            continue;
        }

        let text = String::from_utf8_lossy(&buf);
        partial.push_str(&text);

        while let Some(pos) = partial.find('\n') {
            let line = &partial[..pos];
            let line = line.trim_end_matches('\r');
            let output = if strip_ansi_codes {
                strip_ansi(line)
            } else {
                line.to_string()
            };
            if sender.send(output).is_err() {
                return;
            }
            partial = partial[pos + 1..].to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

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

    #[test]
    fn tail_reads_new_lines_appended_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "existing\n").unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let path_clone = path.clone();
        let handle = std::thread::spawn(move || {
            tail_file(&path_clone, tx, true, shutdown_clone);
        });

        std::thread::sleep(Duration::from_millis(500));
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "new line").unwrap();
        f.flush().unwrap();

        let line = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(line, "new line");

        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn tail_starts_from_end_of_file_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "old line\n").unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let path_clone = path.clone();
        let _handle = std::thread::spawn(move || {
            tail_file(&path_clone, tx, true, shutdown_clone);
        });

        let result = rx.recv_timeout(Duration::from_millis(1000));
        assert!(result.is_err());

        shutdown.store(true, Ordering::Relaxed);
    }

    #[test]
    fn tail_handles_file_not_existing_yet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.log");

        let (tx, rx) = std::sync::mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let path_clone = path.clone();
        let _handle = std::thread::spawn(move || {
            tail_file(&path_clone, tx, true, shutdown_clone);
        });

        std::thread::sleep(Duration::from_millis(500));
        use std::io::Write;
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "appeared").unwrap();
        f.flush().unwrap();

        let line = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(line, "appeared");

        shutdown.store(true, Ordering::Relaxed);
    }
}
