//! A small in-memory ring buffer of recent log lines, for the bug reporter (000256).
//!
//! The project has no logging stack, so there is nothing to tail into a report. [`LogBuffer`]
//! is a bounded, shareable sink: clones share one underlying buffer (an `Arc<Mutex<…>>`), so
//! the standalone can install it as a panic hook (capturing crashes — the most useful thing
//! a bug report can carry) and hand the same handle to the editor, whose reporter snapshots
//! the last N lines into the issue body. It keeps only the most recent `capacity` lines, so
//! it never grows without bound.
//!
//! The lines are scrubbed for secrets by [`seclab_ai::scrub`] before they leave the machine
//! (the reporter does this when it assembles the issue), so a token that lands in a log can
//! never reach the report.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// A bounded, clonable ring buffer of recent log lines.
///
/// Cloning shares the same underlying storage, so every holder sees the same lines.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogBuffer {
    /// A buffer that retains at most `capacity` of the most recent lines (minimum 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            capacity: capacity.max(1),
        }
    }

    /// Appends a line, dropping the oldest when the buffer is full. A poisoned lock is
    /// recovered (a logging buffer must never panic the caller).
    pub fn push(&self, line: impl Into<String>) {
        let mut buf = match self.inner.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };
        buf.push_back(line.into());
        while buf.len() > self.capacity {
            buf.pop_front();
        }
    }

    /// The most recent `max_lines` lines joined by newlines (all of them when fewer are
    /// held), or an empty string when nothing has been logged.
    #[must_use]
    pub fn tail(&self, max_lines: usize) -> String {
        let buf = match self.inner.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };
        let start = buf.len().saturating_sub(max_lines);
        buf.iter()
            .skip(start)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Number of lines currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(b) => b.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// Whether the buffer holds no lines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Installs this buffer as a panic hook: each panic message is captured (so a crash
    /// shows up in the next bug report) and the previously installed hook still runs, so the
    /// usual backtrace still prints. Call once, early in `main`.
    pub fn install_panic_hook(&self) {
        let sink = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            sink.push(format!("panic: {info}"));
            previous(info);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_tail_round_trip() {
        let log = LogBuffer::new(10);
        assert!(log.is_empty());
        assert_eq!(log.tail(5), "");
        log.push("first");
        log.push("second");
        assert_eq!(log.len(), 2);
        assert_eq!(log.tail(5), "first\nsecond");
    }

    #[test]
    fn tail_returns_only_the_last_n() {
        let log = LogBuffer::new(10);
        for i in 1..=5 {
            log.push(format!("line{i}"));
        }
        assert_eq!(log.tail(2), "line4\nline5");
    }

    #[test]
    fn capacity_drops_the_oldest_lines() {
        let log = LogBuffer::new(3);
        for i in 1..=5 {
            log.push(format!("l{i}"));
        }
        assert_eq!(log.len(), 3, "only the capacity is retained");
        assert_eq!(log.tail(10), "l3\nl4\nl5", "oldest two dropped");
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let log = LogBuffer::new(0);
        log.push("a");
        log.push("b");
        assert_eq!(log.tail(10), "b");
    }

    #[test]
    fn clones_share_one_buffer() {
        let a = LogBuffer::new(10);
        let b = a.clone();
        a.push("from a");
        b.push("from b");
        // Both handles see both lines in order.
        assert_eq!(a.tail(10), "from a\nfrom b");
        assert_eq!(b.len(), 2);
    }
}
