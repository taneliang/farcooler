//! Where `terminal send` goes (ov-557).

/// Where `terminal send` goes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SendRoute {
    /// `--if-empty` (ov-557): through the daemon's composer gate, on this
    /// runner or another (`compose::run` dials it itself), which checks the
    /// box right before it types.
    Compose,
    /// Raw bytes for a remote runner's own CLI, over ssh.
    Proxy,
    /// Raw bytes into this runner's pane.
    Local,
}

/// The route of a `terminal send`. `--if-empty` comes first: a remote runner
/// must never get it as raw bytes with the flag dropped and no Return.
pub(crate) fn send_route(if_empty: bool, remote: bool) -> SendRoute {
    if if_empty {
        SendRoute::Compose
    } else if remote {
        SendRoute::Proxy
    } else {
        SendRoute::Local
    }
}
