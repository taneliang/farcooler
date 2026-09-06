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
    /// The agent has no credentials, and the fix is not in Far Cooler.
    ///
    /// Its own variant because it is the one failure a user can act on without
    /// touching a config file: they run the agent's login command on the
    /// runner. Folded into `Closed` — which is where it used to land — it
    /// reached the screen as "the ACP adapter closed its connection", which
    /// sends whoever reads it looking at the wrong thing entirely.
    #[error("the agent needs you to sign in: {0}")]
    NotAuthenticated(String),
    /// The installed CLI speaks a protocol these generated types do not cover.
    ///
    /// Both versions, because a user reading this has to be able to tell which
    /// side is behind without running anything else — the fix is "update Far
    /// Cooler" in one direction and "update the agent" in the other.
    #[error("this agent speaks protocol {found}, but this build was generated against {expected}")]
    Incompatible { found: String, expected: String },
}

/// Why a pane in agent mode has no agent in it.
///
/// **A stable machine word, and never a sentence.** It leaves the shim on the
/// daemon link, crosses the protocol on `Terminal.agent_failure`, and each app
/// owns the words a person reads — the same rule `TunnelError::code` states
/// for the tunnel. A Rust error string must never reach a screen, which is
/// exactly what happened while the only report of a failed adapter was a line
/// printed to a pane's stdout that the transcript view then covered up.
///
/// Four words rather than one, because the advice differs for each and a
/// single "it broke" is the state the spinner already communicated. Adding a
/// variant means adding a sentence in every app, which is the cost that keeps
/// this list short.
///
/// Serialized in kebab-case so the JSON on the daemon link is the same word
/// `code` returns; `the_wire_word_and_the_ffi_word_are_one_word` holds the two
/// together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentFailure {
    /// No adapter is configured for this pane's preset. The fix is config.
    NoAdapter,
    /// The adapter refused for want of credentials. The fix is a login, on
    /// the runner, in the agent's own CLI.
    NotAuthenticated,
    /// It started and then said nothing at all, until the shim gave up. The
    /// fix is usually environmental, and there is nothing to read anywhere.
    AdapterSilent,
    /// Everything else: it would not spawn, it closed, it spoke a protocol
    /// this build does not, or it refused for a reason of its own.
    AdapterFailed,
}

impl AgentFailure {
    /// The word that crosses to the daemon and then to the apps.
    ///
    /// Hyphenated to match the words named in the ruling this was built
    /// against. A rename is a breaking change for a shipped app, not a
    /// tidy-up.
    pub fn code(self) -> &'static str {
        match self {
            Self::NoAdapter => "no-adapter",
            Self::NotAuthenticated => "not-authenticated",
            Self::AdapterSilent => "adapter-silent",
            Self::AdapterFailed => "adapter-failed",
        }
    }

    /// The word back, for the daemon reading a shim's report.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "no-adapter" => Some(Self::NoAdapter),
            "not-authenticated" => Some(Self::NotAuthenticated),
            "adapter-silent" => Some(Self::AdapterSilent),
            "adapter-failed" => Some(Self::AdapterFailed),
            _ => None,
        }
    }
}

impl From<&BackendError> for AgentFailure {
    /// Every backend failure becomes exactly one word.
    ///
    /// Spelled out rather than defaulted, so a new `BackendError` variant is a
    /// compile error here and somebody has to decide which sentence a user
    /// should read — instead of it silently joining `adapter-failed`.
    fn from(e: &BackendError) -> Self {
        match e {
            BackendError::NotAuthenticated(_) => Self::NotAuthenticated,
            BackendError::Silent => Self::AdapterSilent,
            BackendError::Spawn
            | BackendError::Closed
            | BackendError::Refused(_)
            | BackendError::Incompatible { .. } => Self::AdapterFailed,
        }
    }
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

    /// The word on the daemon link and the word crossing to an app are the
    /// same word.
    ///
    /// Two derivations of one string — serde's `rename_all` and `code` — and
    /// nothing but this holds them together. If they drift, a shim reports
    /// `not-authenticated` and the daemon hands an app something it has no
    /// sentence for, so the pane says nothing at all: the exact silence this
    /// whole path exists to end.
    #[test]
    fn the_wire_word_and_the_ffi_word_are_one_word() {
        for failure in [
            AgentFailure::NoAdapter,
            AgentFailure::NotAuthenticated,
            AgentFailure::AdapterSilent,
            AgentFailure::AdapterFailed,
        ] {
            let json = serde_json::to_string(&failure).expect("encodes");
            assert_eq!(json, format!("\"{}\"", failure.code()));
            assert_eq!(AgentFailure::from_code(failure.code()), Some(failure));
        }
    }

    /// The words themselves, written out.
    ///
    /// A rename is a breaking change for an app in the field — it renders a
    /// sentence per word and has no fallback for one it does not know — so
    /// the strings are pinned here rather than only being derived.
    #[test]
    fn every_failure_has_a_stable_word() {
        assert_eq!(AgentFailure::NoAdapter.code(), "no-adapter");
        assert_eq!(AgentFailure::NotAuthenticated.code(), "not-authenticated");
        assert_eq!(AgentFailure::AdapterSilent.code(), "adapter-silent");
        assert_eq!(AgentFailure::AdapterFailed.code(), "adapter-failed");
        assert_eq!(AgentFailure::from_code("nonsense"), None);
    }

    /// An adapter that will not authenticate does not read as one that hung
    /// up.
    ///
    /// This is the flattening the whole change is about, at its last hop:
    /// `BackendError::Closed` and `BackendError::NotAuthenticated` must not
    /// arrive at an app as the same word, because their fixes have nothing in
    /// common — one is a login on the runner, the other is anybody's guess.
    #[test]
    fn a_refusal_to_authenticate_keeps_its_own_word() {
        assert_eq!(
            AgentFailure::from(&BackendError::NotAuthenticated("Not authenticated".into())),
            AgentFailure::NotAuthenticated
        );
        assert_eq!(AgentFailure::from(&BackendError::Closed), AgentFailure::AdapterFailed);
        assert_eq!(AgentFailure::from(&BackendError::Silent), AgentFailure::AdapterSilent);
    }
}
