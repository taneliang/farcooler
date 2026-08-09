//! What every backend has to be able to do, and how it fails.

use crate::event::{AgentEvent, PromptImage};

/// Which protocol a session is running on.
///
/// For messages and logs only — never a reason to branch on behavior. What a
/// backend can DO is `Capabilities`, and code that switches on this instead
/// will be wrong the first time two backends share a trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Acp,
    Claude,
    Codex,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Acp => "acp",
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
        }
    }
}

/// What a backend can do, as distinct from what it currently offers.
///
/// Deliberately behavioral only. Modes, models and config options are NOT
/// capabilities — they arrive dynamically on `SessionStarted`, and a client
/// renders whatever is in that list without knowing in advance what is in it.
/// Putting them here would recreate exactly the coupling `ConfigOption` exists
/// to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: BackendKind,
    /// The backend can inject a prompt into a turn already running.
    ///
    /// False means the neutral layer's queue emulates it by holding the prompt
    /// until `TurnEnded`. The distinction has to reach the UI, because a
    /// composer that says "sent" about a prompt still sitting in a queue is
    /// telling the user something untrue.
    pub native_steer: bool,
    /// The backend can rejoin a session it did not itself start.
    ///
    /// False makes the neutral layer emit `AgentGapReason::LoadUnsupported`
    /// rather than attempt a request it already knows will fail — which is
    /// what that variant has always meant, decided one layer lower.
    pub replay: bool,
    /// The backend asks the CLIENT to perform file operations, so every path
    /// it reports is untrusted until `fs_guard::confine` has agreed it is
    /// inside the worktree.
    pub client_side_fs: bool,
}

impl Capabilities {
    /// ACP's shape: no steering, confinement required, and replay decided per
    /// connection by what `initialize` advertised.
    pub fn acp() -> Self {
        Capabilities {
            backend: BackendKind::Acp,
            native_steer: false,
            replay: false,
            client_side_fs: true,
        }
    }
}

/// How to start an agent process, with the program already resolved.
///
/// The search happens in `farcooler-agent`, which can reach
/// `farcooler_core::programs::find`; a backend takes the ANSWER rather than the
/// search, so a backend crate never has to depend on core. That resolution is
/// not optional politeness — a Dock-launched daemon inherits launchd's `PATH`
/// and finds no `npx`, no `opencode`, nothing a package manager installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Why a backend could not do what was asked.
///
/// Four of these existed already under other names — `Status::AdapterMissing`,
/// `Status::AdapterSilent`, and two `AcpError` variants. `Incompatible` is the
/// one genuinely new failure, and it is native-only: a generated protocol has
/// a version, and the installed CLI may not match it.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("could not start the agent")]
    Spawn,
    #[error("the agent started but never answered")]
    Silent,
    #[error("the agent closed its connection")]
    Closed,
    /// Carries the agent's own message, because the caller usually cannot say
    /// anything more useful than the agent already did.
    #[error("the agent refused: {0}")]
    Refused(String),
    /// The installed CLI speaks a protocol these generated types do not cover.
    ///
    /// Both versions, because a user reading this has to be able to tell which
    /// side is behind without running anything else — the fix is "update Far
    /// Cooler" in one direction and "update the agent" in the other.
    #[error("this agent speaks protocol {found}, but this build was generated against {expected}")]
    Incompatible { found: String, expected: String },
}

/// One live agent conversation, whatever protocol carries it.
///
/// There is deliberately no `set_mode` and no `set_model`. The comment on
/// `ConfigOption` already argues the case: an agent advertises its selectors
/// and the client renders one control each, "rather than the client knowing in
/// advance that 'mode' and 'model' exist". So `mode` and `model` are
/// well-known ids on `set_config_option`, not methods of their own.
///
/// Dispatched through an enum rather than as a trait object: async fn in
/// traits is not dyn-compatible, so `dyn AgentBackend` would cost a new
/// dependency and a boxed future per call. This trait states the contract —
/// and lets a test fake implement it — while `Backend` performs it.
pub trait AgentBackend: Send {
    fn capabilities(&self) -> Capabilities;

    /// Start a turn.
    fn prompt(
        &mut self,
        text: &str,
        images: &[PromptImage],
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    /// Inject into the turn already running.
    ///
    /// Only called when `capabilities().native_steer` is true. A backend that
    /// reports false never sees this, because the neutral layer's queue holds
    /// the prompt instead.
    fn steer(
        &mut self,
        text: &str,
        images: &[PromptImage],
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn answer(
        &mut self,
        request_id: &str,
        option_id: &str,
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn set_config_option(
        &mut self,
        id: &str,
        value: &str,
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn cancel(&mut self) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    /// Block until the backend has something to say.
    ///
    /// Never returns an empty vector. A caller that has to tell "nothing yet"
    /// apart from "an event carrying nothing" will get it wrong, and an empty
    /// return inside a loop is a spin rather than a wait.
    fn next_events(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<AgentEvent>, BackendError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_incompatible_backend_names_both_versions() {
        // The whole point of this variant: a user reading it has to be able to
        // tell which side is behind without running anything else.
        let e = BackendError::Incompatible {
            found: "0.152.0".into(),
            expected: "0.146.0".into(),
        };
        let text = e.to_string();
        assert!(text.contains("0.152.0"), "{text}");
        assert!(text.contains("0.146.0"), "{text}");
    }

    #[test]
    fn a_refusal_carries_the_agents_own_words() {
        // Anything else substitutes our description for the agent's, and the
        // agent usually said something more useful than we can.
        let e = BackendError::Refused("no rollout found for thread id".into());
        assert!(e.to_string().contains("no rollout found for thread id"));
    }

    #[test]
    fn acp_does_not_claim_native_steering() {
        // ACP has no way to inject into a running turn. Claiming otherwise
        // makes the composer tell the user a queued prompt was delivered.
        assert!(!Capabilities::acp().native_steer);
    }

    #[test]
    fn acp_always_needs_path_confinement() {
        // The agent asks US to write files. Every path it names is untrusted
        // until confine() has agreed it is inside the worktree.
        assert!(Capabilities::acp().client_side_fs);
    }
}
