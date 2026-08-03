//! Terminal replay buffer.
//!
//! A sequence number is a `uint64` BYTE OFFSET from the first byte of the
//! current epoch, so the 8 MiB buffer, the 1 MiB unacknowledged window, and the
//! 64 KiB frame cap all measure the same thing with no conversion. Frame
//! boundaries carry no protocol meaning: identical output yields identical
//! offsets no matter how the daemon chunked its reads.

use std::collections::VecDeque;

use farcooler_protocol::{REPLAY_BUFFER_BYTES, v1::GapReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// Epoch matched and the offset was still retained.
    At { sequence: u64 },
    /// Could not cover the request. The client clears its terminal model and
    /// applies the retained tail after a permanent visible marker.
    Gap { resumed_at_sequence: u64, lost_bytes: Option<u64>, reason: GapReason },
}

#[derive(Debug)]
pub struct ReplayBuffer {
    epoch: u64,
    capacity: u64,
    chunks: VecDeque<(u64, Vec<u8>)>, // (start_sequence, bytes)
    oldest_sequence: u64,
    next_sequence: u64,
}

impl ReplayBuffer {
    pub fn new(epoch: u64) -> Self {
        Self::with_capacity(epoch, REPLAY_BUFFER_BYTES)
    }

    pub fn with_capacity(epoch: u64, capacity: u64) -> Self {
        Self {
            epoch,
            capacity,
            chunks: VecDeque::new(),
            oldest_sequence: 0,
            next_sequence: 0,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn oldest_sequence(&self) -> u64 {
        self.oldest_sequence
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Bytes currently retained. Never exceeds the capacity.
    pub fn retained(&self) -> u64 {
        self.next_sequence - self.oldest_sequence
    }

    /// A new runtime means a new epoch: offsets restart at zero.
    pub fn start_epoch(&mut self, epoch: u64) {
        self.epoch = epoch;
        self.chunks.clear();
        self.oldest_sequence = 0;
        self.next_sequence = 0;
    }

    /// Append output. Returns the start offset of these bytes.
    pub fn push(&mut self, bytes: &[u8]) -> u64 {
        if bytes.is_empty() {
            return self.next_sequence;
        }
        let start = self.next_sequence;
        self.chunks.push_back((start, bytes.to_vec()));
        self.next_sequence += bytes.len() as u64;
        self.evict();
        start
    }

    fn evict(&mut self) {
        while self.retained() > self.capacity {
            let Some((start, chunk)) = self.chunks.front().cloned() else { break };
            let chunk_end = start + chunk.len() as u64;
            let overflow = self.retained() - self.capacity;

            if chunk_end <= self.oldest_sequence + overflow {
                // Drop the whole chunk.
                self.chunks.pop_front();
                self.oldest_sequence = chunk_end;
            } else {
                // Trim the front of this chunk exactly.
                let drop_n = overflow as usize;
                let (_, front) = self.chunks.front_mut().expect("checked above");
                front.drain(..drop_n);
                self.oldest_sequence += overflow;
                if let Some((s, _)) = self.chunks.front_mut() {
                    *s = self.oldest_sequence;
                }
            }
        }
    }

    /// Everything retained from `from` onward, slicing mid-chunk when the offset
    /// falls inside one.
    pub fn tail_from(&self, from: u64) -> Vec<u8> {
        let mut out = Vec::new();
        for (start, chunk) in &self.chunks {
            let end = start + chunk.len() as u64;
            if end <= from {
                continue;
            }
            let skip = from.saturating_sub(*start) as usize;
            out.extend_from_slice(&chunk[skip.min(chunk.len())..]);
        }
        out
    }

    /// Decide how a reconnecting client resumes.
    pub fn resume(&self, client_epoch: u64, last_acked: u64) -> Resume {
        if client_epoch != self.epoch {
            // Offsets are not comparable across epochs, so lost_bytes is absent.
            return Resume::Gap {
                resumed_at_sequence: self.oldest_sequence,
                lost_bytes: None,
                reason: GapReason::EpochChanged,
            };
        }
        if last_acked < self.oldest_sequence {
            return Resume::Gap {
                resumed_at_sequence: self.oldest_sequence,
                lost_bytes: Some(self.oldest_sequence - last_acked),
                reason: GapReason::ReplayEvicted,
            };
        }
        Resume::At { sequence: last_acked }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_bytes_not_frames() {
        let mut b = ReplayBuffer::new(1);
        assert_eq!(b.push(b"hello"), 0);
        assert_eq!(b.push(b" world"), 5);
        assert_eq!(b.next_sequence(), 11);
    }

    #[test]
    fn chunking_does_not_change_offsets() {
        let mut one = ReplayBuffer::new(1);
        one.push(b"abcdefghij");

        let mut many = ReplayBuffer::new(1);
        for c in b"abcdefghij" {
            many.push(&[*c]);
        }

        assert_eq!(one.next_sequence(), many.next_sequence());
        assert_eq!(one.tail_from(3), many.tail_from(3));
    }

    #[test]
    fn retained_never_exceeds_capacity() {
        let mut b = ReplayBuffer::with_capacity(1, 10);
        for _ in 0..10 {
            b.push(b"abcde");
        }
        assert!(b.retained() <= 10, "retained {} exceeded cap", b.retained());
        assert_eq!(b.next_sequence() - b.oldest_sequence(), b.retained());
    }

    #[test]
    fn eviction_advances_oldest_sequence() {
        let mut b = ReplayBuffer::with_capacity(1, 8);
        b.push(b"aaaaaaaa"); // exactly full
        assert_eq!(b.oldest_sequence(), 0);
        b.push(b"bb");
        assert_eq!(b.oldest_sequence(), 2, "two oldest bytes evicted");
        assert_eq!(b.tail_from(b.oldest_sequence()), b"aaaaaabb");
    }

    #[test]
    fn resume_within_buffer_is_exact() {
        let mut b = ReplayBuffer::new(1);
        b.push(b"hello world");
        assert_eq!(b.resume(1, 6), Resume::At { sequence: 6 });
        assert_eq!(b.tail_from(6), b"world");
    }

    #[test]
    fn overflow_reports_exact_lost_bytes() {
        let mut b = ReplayBuffer::with_capacity(1, 8);
        b.push(b"aaaaaaaaaa"); // 10 bytes into an 8 byte buffer
        // client had acked 0; oldest is now 2, so exactly 2 bytes were lost
        assert_eq!(
            b.resume(1, 0),
            Resume::Gap {
                resumed_at_sequence: 2,
                lost_bytes: Some(2),
                reason: GapReason::ReplayEvicted
            }
        );
    }

    #[test]
    fn epoch_change_has_no_comparable_byte_count() {
        let mut b = ReplayBuffer::new(2);
        b.push(b"fresh");
        match b.resume(1, 100) {
            Resume::Gap { lost_bytes, reason, .. } => {
                assert_eq!(lost_bytes, None, "offsets are not comparable across epochs");
                assert_eq!(reason, GapReason::EpochChanged);
            }
            other => panic!("expected a gap, got {other:?}"),
        }
    }

    #[test]
    fn new_epoch_restarts_offsets_at_zero() {
        let mut b = ReplayBuffer::new(1);
        b.push(b"old data");
        b.start_epoch(2);
        assert_eq!(b.next_sequence(), 0);
        assert_eq!(b.oldest_sequence(), 0);
        assert_eq!(b.push(b"new"), 0);
    }
}
