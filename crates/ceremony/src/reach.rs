//! How a granted runner is reached: the enum a ceremony reply writes and a
//! connection reads.
//!
//! Here rather than in `farcooler-client`'s `ssh` module, where it was defined
//! until the ceremony moved into a crate the browser can build (ov-764): the
//! reply carries one per runner, so the rules that read a reply need it, and
//! `farcooler_client::ssh::Reach` re-exports this one.

/// How to reach a runner: an address, or a connection token that dials the
/// tunnel that runner holds open. Which one this device has is decided when the
/// runner is set up, not per connection.
///
/// **A token comes from one of two calls and from nowhere else.** A ceremony
/// is the ordinary one: the device's offer carries its node key, the granting
/// side sends it in `client.enroll`, and the runner answers with
/// `ClientEnrollResult.conn_blob` — the token it is now serving, which the
/// granting side puts in the reply as this variant. The other is
/// `client.set_node_key`, for a device that already holds a session on a runner
/// it can reach directly and wants a tunnel route as well; that is the
/// migration path for a fleet enrolled before the tunnel existed.
///
/// A ceremony still produces `Direct` for every device that offers no node key,
/// and for every runner whose tunnel could not be started — which is not a
/// failure of the pairing and must never be made into one.
///
/// One reach per runner, and no fallback. A `Direct` runner never quietly
/// tries the tunnel and a `Tailcat` runner never quietly tries an address —
/// because a transport that races two paths reports the wrong failure, and the
/// failure message is most of what this product is.
/// Serialized because a ceremony reply carries one per runner: this is the
/// enum `farcooler_client::ssh` re-exports rather than redefining, so the
/// shape a manifest writes is the shape a `Destination` reads.
///
/// Internally tagged on `kind` so a third variant is additive on the wire —
/// two optional fields would admit "both set" and "neither set", and then
/// something downstream picks a winner.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reach {
    /// You exchanged a key yourself and you know where the box is.
    Direct { host: String, port: u16 },
    /// The runner told you its token and you told it your node key.
    Tailcat {
        token: String,
        /// This device's OWN tailcat node private key, and the one field here
        /// that is a secret.
        ///
        /// `skip` rather than a convention that callers blank it: a manifest
        /// is photographable — the design says so in as many words — and a
        /// private key in one would be draft three's mistake with a new
        /// name. Nothing else needs it on a wire either, because the key is
        /// per DEVICE and not per runner: whoever dials fills in the key it
        /// already holds. A reply therefore decodes with this empty, which is
        /// not "unknown" — it is "yours, and you have it".
        #[serde(skip)]
        client_key: String,
    },
    /// The tunnel, through the CLI that carries it: `<cli> runner pipe <id>`
    /// spawned and spoken SSH over, as a `ProxyCommand` is.
    ///
    /// For a client that has no tunnel linked of its own (the Mac app), and
    /// that reads the command from the owner's `ssh_config`. **Never on a
    /// wire**: `serde(skip)` makes a manifest naming a program to run
    /// undecodable, and serializing it an error. Only
    /// `farcooler_client::pipe::argv_is_a_runner_pipe` shapes are spawned.
    #[serde(skip)]
    #[cfg(target_os = "macos")]
    Pipe { argv: Vec<String> },
}

impl Reach {
    /// What to name in an error. Never a token: it is long, it is meaningless
    /// to a person, and it is the one field here worth stealing.
    pub fn label(&self) -> String {
        match self {
            Self::Direct { host, port } => format!("{host}:{port}"),
            Self::Tailcat { .. } => "the tunnel".to_string(),
            #[cfg(target_os = "macos")]
            Self::Pipe { .. } => "the tunnel".to_string(),
        }
    }
}

/// How a connection on one reach finds out that the far end has gone quiet.
///
/// After `probe_after` with nothing received, the client sends one probe
/// that asks for an answer (OpenSSH's `keepalive@openssh.com`), and again
/// every `probe_after` while nothing arrives. Once more than `unanswered`
/// probes have gone unanswered, the connection is over. ANY byte received
/// resets both the clock and the count, so a connection that is carrying
/// output sends no probes at all: the cost is paid only while it is idle.
///
/// Here, beside `Reach`, so the native client and the browser read one
/// definition. Each runs it on its own clock: the native client hands these
/// numbers to russh, and the browser, which has no clock russh can use, has
/// to run its own timer (ov-772).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Liveness {
    pub probe_after: std::time::Duration,
    pub unanswered: usize,
}

impl Liveness {
    /// An address. A dead host there is rare and the probe is paid for in
    /// radio wake-ups on a phone, so it is noticed in two minutes. This is
    /// the setting every direct connection already had; a tunnel's
    /// tighter one must not leak onto it.
    pub const DIRECT: Liveness =
        Liveness { probe_after: std::time::Duration::from_secs(30), unanswered: 3 };

    /// The tunnel. A runner crash or restart, or a relay drop, sends
    /// nothing to the device: its TCP lives inside the tunnel, so no reset
    /// ever arrives and the session hangs until a probe goes unanswered
    /// (ov-781). Three seconds with two allowed to go unanswered notices
    /// within nine, and still waits six seconds for an answer before giving
    /// up, which a slow cellular round trip through the relay fits inside.
    pub const TUNNEL: Liveness =
        Liveness { probe_after: std::time::Duration::from_secs(3), unanswered: 2 };

    /// The longest a dead link goes unnoticed, counted from the last byte
    /// received: the probe that is one past `unanswered` is the one that
    /// ends it, so it is `probe_after` times `unanswered + 1`.
    pub fn noticed_within(&self) -> std::time::Duration {
        self.probe_after * (self.unanswered as u32 + 1)
    }
}

impl Reach {
    /// How a connection on this reach notices a far end that went quiet.
    ///
    /// `Pipe` is the tunnel too, carried by the CLI: a dead tunnel leaves the
    /// CLI's pipe open and silent, exactly as it leaves a phone's socket.
    pub fn liveness(&self) -> Liveness {
        match self {
            Self::Direct { .. } => Liveness::DIRECT,
            Self::Tailcat { .. } => Liveness::TUNNEL,
            #[cfg(target_os = "macos")]
            Self::Pipe { .. } => Liveness::TUNNEL,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Liveness, Reach};

    #[test]
    fn a_tunnel_notices_a_dead_far_end_within_ten_seconds() {
        let tunnel = Reach::Tailcat { token: "tc-x".into(), client_key: String::new() };
        assert_eq!(tunnel.liveness(), Liveness::TUNNEL);
        assert!(
            tunnel.liveness().noticed_within() <= Duration::from_secs(10),
            "a dead tunnel goes unnoticed for {:?}",
            tunnel.liveness().noticed_within()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macs_pipe_is_a_tunnel_too() {
        let pipe = Reach::Pipe { argv: vec!["/x/farcooler".into()] };
        assert_eq!(pipe.liveness(), Liveness::TUNNEL);
    }

    #[test]
    fn an_address_keeps_the_probe_it_always_had() {
        let direct = Reach::Direct { host: "box".into(), port: 22 };
        assert_eq!(
            direct.liveness(),
            Liveness { probe_after: Duration::from_secs(30), unanswered: 3 }
        );
        assert_eq!(direct.liveness().noticed_within(), Duration::from_secs(120));
    }
}
