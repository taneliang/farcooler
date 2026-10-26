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
