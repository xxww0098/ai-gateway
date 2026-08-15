//! Bounded accumulator for streaming upstream bodies.
//!
//! OWNER: worker `provider-openai`.
//!
//! It accumulates an upstream streaming response for terminal usage parsing
//! while bounding memory: it retains the first [`DEFAULT_STREAM_HEAD_LIMIT`]
//! bytes and the last [`DEFAULT_STREAM_TAIL_LIMIT`] bytes, discarding only the
//! (potentially unbounded) middle of content deltas. That is safe for every
//! provider's usage parser because the token-count events live at the *ends* of
//! the stream, never the middle:
//!
//! - Anthropic (Claude): `input_tokens` in the leading `message_start`,
//!   `output_tokens` in the trailing `message_delta` — both ends are retained.
//! - OpenAI / Codex: the terminal `data: {...,"usage":...}` event — tail.
//! - Gemini / Vertex: the final cumulative `usageMetadata` chunk — tail.
//!
//! A long (or malicious / never-closing) stream therefore costs at most
//! `head + tail` bytes instead of growing without bound × concurrent streams.

/// Head window.
pub const DEFAULT_STREAM_HEAD_LIMIT: usize = 32 * 1024;
/// Tail window.
pub const DEFAULT_STREAM_TAIL_LIMIT: usize = 64 * 1024;

/// Head/tail-windowed byte accumulator.
#[derive(Debug, Clone)]
pub struct StreamUsageBuffer {
    head_limit: usize,
    tail_limit: usize,
    head: Vec<u8>,
    /// Retains (lazily compacted) the last `tail_limit` bytes of the stream.
    tail: Vec<u8>,
    total: u64,
}

impl Default for StreamUsageBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamUsageBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_STREAM_HEAD_LIMIT, DEFAULT_STREAM_TAIL_LIMIT)
    }

    /// Same accumulator with explicit windows. Used by tests to exercise the
    /// drop path without allocating ~96 KiB of fixture.
    #[must_use]
    pub fn with_limits(head_limit: usize, tail_limit: usize) -> Self {
        Self {
            head_limit,
            tail_limit,
            head: Vec::new(),
            tail: Vec::new(),
            total: 0,
        }
    }

    /// Total number of bytes ever written (not the retained size).
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Appends `p`, retaining only the head and tail windows.
    pub fn write(&mut self, p: &[u8]) {
        // Fill the head window until full, then freeze it.
        if self.head.len() < self.head_limit {
            let room = (self.head_limit - self.head.len()).min(p.len());
            self.head.extend_from_slice(&p[..room]);
        }
        // Maintain the tail window: append, then compact lazily once it has
        // grown well past the limit so the trim cost is amortized across writes.
        self.tail.extend_from_slice(p);
        if self.tail.len() > 2 * self.tail_limit {
            let drop = self.tail.len() - self.tail_limit;
            self.tail.drain(..drop);
        }
        self.total += p.len() as u64;
    }

    /// Returns the retained buffer for the usage parsers.
    ///
    /// When the whole stream fits within head+tail it is reconstructed exactly;
    /// otherwise the dropped middle is represented by a single newline
    /// separator so partial boundary lines stay separate (and unparseable)
    /// rather than fusing into a spuriously-valid record.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let tail: &[u8] = if self.tail.len() > self.tail_limit {
            &self.tail[self.tail.len() - self.tail_limit..]
        } else {
            &self.tail
        };

        // Whole stream fit in the head window: head holds everything.
        if self.total <= self.head_limit as u64 {
            return self.head.clone();
        }

        // No middle dropped: head + the non-overlapping remainder of tail
        // reconstructs the original bytes exactly.
        if self.total <= self.head_limit as u64 + self.tail_limit as u64 {
            let overlap = (self.head_limit as u64 + tail.len() as u64).saturating_sub(self.total);
            let overlap = overlap as usize;
            let mut out = Vec::with_capacity(self.head.len() + tail.len() - overlap);
            out.extend_from_slice(&self.head);
            out.extend_from_slice(&tail[overlap..]);
            return out;
        }

        // Middle dropped: join head and tail with a newline separator.
        let mut out = Vec::with_capacity(self.head.len() + 1 + tail.len());
        out.extend_from_slice(&self.head);
        out.push(b'\n');
        out.extend_from_slice(tail);
        out
    }
}

#[cfg(test)]
#[path = "streambuf_tests.rs"]
mod tests;
