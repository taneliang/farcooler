//! The runner's stand-in agent, as `status` reports it (moved out of
//! `main.rs`, ov-454).

/// What every agent launch on this runner runs instead of the real agent, or
/// `None` when agents launch as themselves — every shipped install, and every
/// runner too old to say.
///
/// From `FARCOOLER_STAND_IN_AGENT` in the daemon's environment. A value that
/// leaked out of a test or a demo makes every agent run `sleep` or `false`,
/// and until the runner said so the only sign was one line in its own log.
pub(crate) fn stand_in_agent(host: &farcooler_protocol::v1::Host) -> Option<&str> {
    Some(host.stand_in_agent.as_str()).filter(|p| !p.is_empty())
}

/// Plain `status`'s line for a stand-in agent, or `None` when there is none.
///
/// Loud, like MISMATCH and UNAVAILABLE above it, because it is the same kind
/// of news: nothing is broken that an error would name, and every agent on
/// the runner is quietly not the agent.
pub(crate) fn stand_in_line(host: &farcooler_protocol::v1::Host) -> Option<String> {
    stand_in_agent(host).map(|program| {
        format!("agents        STAND-IN: {program} runs instead of the real agent (FARCOOLER_STAND_IN_AGENT)")
    })
}
