//! The address a reply may keep beside a tunnel, and what counts as one:
//! out of `lib.rs` for its size budget (ov-828), and read by
//! [`super::accept_manifest`].

use super::{Reach, RunnerEntry};

/// The one shape [`RunnerEntry::alternative`] may take: none at all, or an
/// address beside a tunnel. An address beside an address is two routes to
/// choose between with nothing to say which, and a tunnel held in reserve is a
/// route a device without a node key would be offered and could never dial.
///
/// And the address must be one: a hostname, IPv4 or IPv6 literal with nothing
/// `ssh_config` would read as a second word or a token (no space, no `%`, no
/// control character), and a port from 1 to 65535. A Mac writes it as
/// `HostName`/`Port` in the first block of `~/.ssh/config`, where a bad value
/// is a parse error for every `ssh` on that Mac (ov-485 M1).
pub(crate) fn alternative_is_an_address_beside_a_tunnel(runner: &RunnerEntry) -> bool {
    match (&runner.reach, &runner.alternative) {
        (_, None) => true,
        (Reach::Tailcat { .. }, Some(Reach::Direct { host, port })) => {
            usable_address(host) && *port >= 1
        }
        _ => false,
    }
}

/// One word `ssh_config` reads as a host: letters, digits, `.`, `_`, `:` and
/// `-`, 1 to 253 of them, not led by `-`.
pub fn usable_address(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && !host.starts_with('-')
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
}
