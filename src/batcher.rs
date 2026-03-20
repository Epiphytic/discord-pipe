pub struct BatchBuffer {
    lines: Vec<String>,
    char_count: usize,
    max_lines: usize,
    max_chars: usize,
}

impl BatchBuffer {
    pub fn new(max_lines: usize, max_chars: usize) -> Self {
        Self {
            lines: Vec::new(),
            char_count: 0,
            max_lines,
            max_chars,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        self.char_count += line.chars().count();
        self.lines.push(line.to_owned());
    }

    pub fn should_flush(&self) -> bool {
        self.lines.len() >= self.max_lines || self.char_count >= self.max_chars
    }

    pub fn drain(&mut self) -> String {
        let content = self.lines.join("\n");
        self.lines.clear();
        self.char_count = 0;
        content
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[allow(dead_code)]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    #[allow(dead_code)]
    pub fn char_count(&self) -> usize {
        self.char_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_batch_is_empty() {
        let batch = BatchBuffer::new(50, 1800);
        assert!(batch.is_empty());
        assert_eq!(batch.line_count(), 0);
        assert_eq!(batch.char_count(), 0);
    }

    #[test]
    fn push_line_updates_counts() {
        let mut batch = BatchBuffer::new(50, 1800);
        batch.push_line("hello");
        assert_eq!(batch.line_count(), 1);
        assert_eq!(batch.char_count(), 5);
        assert!(!batch.is_empty());
    }

    #[test]
    fn triggers_on_line_count() {
        let mut batch = BatchBuffer::new(3, 1800);
        batch.push_line("a");
        assert!(!batch.should_flush());
        batch.push_line("b");
        assert!(!batch.should_flush());
        batch.push_line("c");
        assert!(batch.should_flush());
    }

    #[test]
    fn triggers_on_char_count() {
        let mut batch = BatchBuffer::new(50, 10);
        batch.push_line("12345678901"); // 11 chars > 10
        assert!(batch.should_flush());
    }

    #[test]
    fn drain_returns_content_and_resets() {
        let mut batch = BatchBuffer::new(50, 1800);
        batch.push_line("hello");
        batch.push_line("world");
        let content = batch.drain();
        assert_eq!(content, "hello\nworld");
        assert!(batch.is_empty());
        assert_eq!(batch.line_count(), 0);
    }

    #[test]
    fn counts_unicode_codepoints_not_bytes() {
        let mut batch = BatchBuffer::new(50, 10);
        batch.push_line("héllo"); // 5 codepoints, but 6 bytes
        assert_eq!(batch.char_count(), 5);
    }
}
