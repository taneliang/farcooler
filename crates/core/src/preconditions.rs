//! Generic envelope preconditions, validated BEFORE domain logic runs.
//!
//! A mutation of an existing resource targets that resource and requires its
//! `expected_resource_version`. A create mutation targets and versions its
//! PARENT. Writer-lease mutations compare both the resource version and the
//! lease generation.

use crate::error::{DomainError, Result};

/// What the dispatcher knows about the targeted resource right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentVersions {
    pub resource_version: u64,
    pub lease_generation: Option<u64>,
}

/// Validate optimistic-concurrency and fencing preconditions.
pub fn check(
    current: CurrentVersions,
    expected_resource_version: Option<u64>,
    expected_lease_generation: Option<u64>,
) -> Result<()> {
    if let Some(expected) = expected_resource_version
        && expected != current.resource_version
    {
        return Err(DomainError::ResourceConflict);
    }

    if let Some(expected) = expected_lease_generation {
        match current.lease_generation {
            Some(actual) if actual == expected => {}
            // A terminal that has no lease cannot satisfy a fenced mutation.
            _ => return Err(DomainError::ResourceConflict),
        }
    }

    Ok(())
}

/// Terminal input carries the granted lease generation so delayed input from a
/// former writer is rejected before any byte reaches the PTY.
pub fn check_input_lease(current_generation: u64, carried_generation: u64) -> Result<()> {
    if current_generation == carried_generation {
        Ok(())
    } else {
        Err(DomainError::ResourceConflict)
    }
}

/// Mutating requests require a client-generated idempotency key.
pub fn require_idempotency_key(key: Option<&str>) -> Result<&str> {
    match key {
        Some(k) if !k.is_empty() => Ok(k),
        _ => Err(DomainError::InvalidArgument { what: "idempotency_key" }),
    }
}

/// Repeating a key with the same canonical hash returns the original result;
/// reusing it with a different hash is rejected.
pub fn check_idempotency_replay(stored_hash: Option<&str>, incoming_hash: &str) -> Result<bool> {
    match stored_hash {
        None => Ok(false),
        Some(h) if h == incoming_hash => Ok(true),
        Some(_) => Err(DomainError::IdempotencyMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current(v: u64, lease: Option<u64>) -> CurrentVersions {
        CurrentVersions { resource_version: v, lease_generation: lease }
    }

    #[test]
    fn matching_version_passes() {
        assert!(check(current(5, None), Some(5), None).is_ok());
    }

    #[test]
    fn stale_version_conflicts() {
        let e = check(current(6, None), Some(5), None).unwrap_err();
        assert!(matches!(e, DomainError::ResourceConflict));
    }

    #[test]
    fn absent_expectation_skips_the_check() {
        assert!(check(current(6, None), None, None).is_ok());
    }

    #[test]
    fn matching_lease_generation_passes() {
        assert!(check(current(5, Some(2)), Some(5), Some(2)).is_ok());
    }

    #[test]
    fn stale_lease_generation_conflicts() {
        assert!(check(current(5, Some(3)), Some(5), Some(2)).is_err());
    }

    #[test]
    fn fenced_mutation_on_an_unleased_terminal_conflicts() {
        assert!(check(current(5, None), Some(5), Some(1)).is_err());
    }

    #[test]
    fn delayed_input_from_a_former_writer_is_rejected() {
        assert!(check_input_lease(4, 3).is_err(), "old generation must not reach the PTY");
        assert!(check_input_lease(4, 4).is_ok());
    }

    #[test]
    fn idempotency_key_is_required_for_mutations() {
        assert!(require_idempotency_key(None).is_err());
        assert!(require_idempotency_key(Some("")).is_err());
        assert!(require_idempotency_key(Some("k1")).is_ok());
    }

    #[test]
    fn same_key_same_hash_replays() {
        assert_eq!(check_idempotency_replay(Some("abc"), "abc").unwrap(), true);
    }

    #[test]
    fn same_key_different_hash_is_rejected() {
        let e = check_idempotency_replay(Some("abc"), "xyz").unwrap_err();
        assert!(matches!(e, DomainError::IdempotencyMismatch));
    }

    #[test]
    fn unseen_key_is_not_a_replay() {
        assert_eq!(check_idempotency_replay(None, "abc").unwrap(), false);
    }
}
