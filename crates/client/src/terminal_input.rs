//! Composer and raw-terminal decisions for a pane's writer lease.

use serde::Serialize;

use crate::conversation_words::{NOT_CONNECTED, READ_ONLY, SEND_IN_FLIGHT, WAITING_FOR_INPUT};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WriterInput {
    Claiming,
    Writable,
    Viewing,
}

#[derive(Serialize)]
pub struct ComposerInput {
    pub can_send: bool,
    pub problem: Option<&'static str>,
}

pub fn composer(input: WriterInput, sending: bool, connected: bool, has_draft: bool) -> ComposerInput {
    let problem = match input {
        _ if sending => Some(SEND_IN_FLIGHT),
        WriterInput::Claiming => Some(WAITING_FOR_INPUT),
        WriterInput::Viewing => Some(READ_ONLY),
        WriterInput::Writable if !connected => Some(NOT_CONNECTED),
        WriterInput::Writable => None,
    };
    ComposerInput { can_send: problem.is_none() && has_draft, problem }
}

pub enum BufferedKeys {
    Keep,
    Wait,
    Flush,
    Clear,
    Refused,
}

pub fn writer_changed(previous: WriterInput, next: WriterInput, has_buffered_keys: bool) -> BufferedKeys {
    match (previous, next) {
        _ if previous == next => BufferedKeys::Keep,
        (_, WriterInput::Writable) => BufferedKeys::Flush,
        (_, WriterInput::Claiming) => BufferedKeys::Wait,
        (WriterInput::Claiming, WriterInput::Viewing) if has_buffered_keys => BufferedKeys::Refused,
        _ => BufferedKeys::Clear,
    }
}

pub enum RawKey {
    Ignore,
    Buffer,
    BufferFull,
    Write,
}

/// Hold up to a paste's worth while attaching or waiting for a writer lease.
pub fn raw_key(input: WriterInput, attaching: bool, buffered_bytes: usize, key_bytes: usize) -> RawKey {
    match input {
        WriterInput::Viewing => RawKey::Ignore,
        WriterInput::Writable if !attaching => RawKey::Write,
        _ if buffered_bytes.saturating_add(key_bytes) > 64 * 1024 => RawKey::BufferFull,
        _ => RawKey::Buffer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WriterInput::{Claiming, Viewing, Writable};

    #[test]
    fn an_outstanding_send_takes_precedence_over_a_lease_change() {
        let view = composer(Viewing, true, true, true);
        assert!(!view.can_send);
        assert_eq!(view.problem, Some(SEND_IN_FLIGHT));
        assert_eq!(composer(Viewing, false, true, true).problem, Some(READ_ONLY));
    }

    #[test]
    fn a_repeated_claim_preserves_buffered_keys_until_it_resolves() {
        assert!(matches!(writer_changed(Claiming, Claiming, true), BufferedKeys::Keep));
        assert!(matches!(writer_changed(Claiming, Writable, true), BufferedKeys::Flush));
        assert!(matches!(writer_changed(Claiming, Viewing, true), BufferedKeys::Refused));
        assert!(matches!(writer_changed(Claiming, Viewing, false), BufferedKeys::Clear));
    }

    #[test]
    fn the_key_buffer_limit_applies_during_attach_and_claim() {
        for (input, attaching) in [(Claiming, false), (Writable, true)] {
            assert!(matches!(raw_key(input, attaching, 65_535, 1), RawKey::Buffer));
            assert!(matches!(raw_key(input, attaching, 65_535, 2), RawKey::BufferFull));
        }
        assert!(matches!(raw_key(Writable, false, 0, 1), RawKey::Write));
        assert!(matches!(raw_key(Viewing, false, 0, 1), RawKey::Ignore));
    }
}
