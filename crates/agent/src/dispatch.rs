//! One backend, chosen at startup.
//!
//! An enum rather than `dyn AgentBackend`: async fn in traits is not
//! dyn-compatible, so a trait object would cost a new dependency and a boxed
//! future per call. The set is closed by design — every config-added adapter is
//! ACP — so adding a variant is a code change either way, and an enum at least
//! makes the compiler enumerate the places that have to change.

use farcooler_acp::backend::AcpBackend;
use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities, Launch};
use farcooler_agent_core::event::{AgentEvent, PromptImage};
use farcooler_core::activity::{AdapterBackend, AdapterSpec};

pub use farcooler_acp::handshake::HANDSHAKE_TIMEOUT;

/// Resolve an adapter's program to something spawnable.
///
/// Separate from the handshake because the backends do not do this: a backend
/// crate takes a resolved `Launch` so it never has to depend on core. It also
/// separates two failures that need different things done about them — a
/// program that is not installed, and one that is installed and crashes on
/// launch. Reporting both as "could not start" used to hide that difference.
pub fn resolve(spec: &AdapterSpec) -> Result<Launch, String> {
    if spec.program.trim().is_empty() {
        return Err("no program to run".to_string());
    }
    // Resolved rather than spawned by name, for the reason `programs` exists: a
    // Dock-launched daemon inherits launchd's `PATH` and finds no `npx`, no
    // `opencode`, nothing a package manager installed. That reported a FALSE
    // failure rather than breaking anything — a pane launches its adapter
    // through the user's login shell, so chat mode worked while the Test button
    // said the adapter could not start.
    let program = farcooler_core::programs::find(spec.program.trim())
        .ok_or_else(|| format!("could not find `{}` on this machine", spec.program.trim()))?;
    Ok(Launch { program, args: spec.args.clone(), env: spec.env.clone() })
}

/// Prove an adapter can start and complete its own handshake.
///
/// One implementation, reached both by the Test button in the machine-settings
/// editor and by `every_built_in_backend_completes_a_handshake`. Keeping those
/// two the same code is why the handshake was hoisted out of the test file in
/// the first place, and the crate split must not quietly undo it.
pub fn handshake(
    preset: &str,
    spec: &AdapterSpec,
    timeout: std::time::Duration,
) -> Result<String, String> {
    let launch = resolve(spec)?;
    match spec.backend {
        AdapterBackend::Acp => {
            farcooler_acp::handshake::handshake(&launch, timeout).map(|h| h.reported)
        }
        // Native needs the PRESET as well as the backend, because "native" is
        // not one protocol: codex speaks app-server and claude speaks
        // stream-json, and they share nothing. An agent with no native backend
        // says so rather than falling back to ACP, which would report a working
        // adapter for a protocol that was never spoken to.
        AdapterBackend::Native => match preset {
            "codex" => farcooler_codex::handshake::handshake(&launch, timeout),
            "claude" => farcooler_claude::handshake::handshake(&launch, timeout),
            other => Err(format!(
                "`{other}` has no native backend; set backend = \"acp\" under [adapters.{other}]"
            )),
        },
    }
}

/// Every variant is boxed, so the enum is one pointer wide.
///
/// Unboxed these are 552, 328 and 288 bytes, and an enum is as large as its
/// largest variant — so every holder paid for Claude whichever backend was
/// actually running, and the value is moved on each of the eight dispatch
/// methods below. Boxing all three rather than only the big ones: the lint is
/// about the DIFFERENCE between variants, so leaving one inline just makes that
/// one the outlier.
///
/// Every match arm binds by reference and calls a method, so auto-deref makes
/// the box invisible everywhere except the three places one is built.
pub enum Backend {
    Acp(Box<AcpBackend>),
    Codex(Box<farcooler_codex::backend::CodexBackend>),
    Claude(Box<farcooler_claude::backend::ClaudeBackend>),
}

/// One frame from whichever backend is running.
///
/// Exists so a caller can receive and handle as two steps. Receiving is
/// cancellation safe and handling is not — handling answers the agent's
/// approval and file requests, and being cancelled mid-answer leaves the agent
/// waiting forever on something that is never coming.
pub enum Frame {
    Acp(farcooler_acp::conn::Incoming),
    Codex(farcooler_codex::conn::Incoming),
    Claude(farcooler_claude::conn::Incoming),
}

impl Backend {
    /// Wait for one frame, and do nothing else. Safe in a `select!`.
    pub async fn recv_frame(&mut self) -> Result<Frame, BackendError> {
        match self {
            Backend::Acp(b) => b.session_mut().recv_frame().await.map(Frame::Acp).map_err(Into::into),
            Backend::Codex(b) => b.recv_frame().await.map(Frame::Codex),
            Backend::Claude(b) => b.recv_frame().await.map(Frame::Claude),
        }
    }

    /// Act on one frame. Must run to completion outside any `select!`.
    pub async fn handle(&mut self, frame: Frame) -> Result<Vec<AgentEvent>, BackendError> {
        match (self, frame) {
            (Backend::Acp(b), Frame::Acp(f)) => {
                b.session_mut().handle(f).await.map_err(Into::into)
            }
            (Backend::Codex(b), Frame::Codex(f)) => b.handle(f).await,
            (Backend::Claude(b), Frame::Claude(f)) => b.handle(f).await,
            // Unreachable by construction — `recv_frame` only ever produces the
            // variant matching the backend it was called on — and reported
            // rather than panicked, because a panic in the pump task would take
            // the pane's whole conversation with it.
            _ => Err(BackendError::Closed),
        }
    }
}

impl AgentBackend for Backend {
    fn capabilities(&self) -> Capabilities {
        match self {
            Backend::Acp(b) => b.capabilities(),
            Backend::Codex(b) => b.capabilities(),
            Backend::Claude(b) => b.capabilities(),
        }
    }

    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.prompt(text, images).await,
            Backend::Codex(b) => b.prompt(text, images).await,
            Backend::Claude(b) => b.prompt(text, images).await,
        }
    }

    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.steer(text, images).await,
            Backend::Codex(b) => b.steer(text, images).await,
            Backend::Claude(b) => b.steer(text, images).await,
        }
    }

    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.answer(request_id, option_id).await,
            Backend::Codex(b) => b.answer(request_id, option_id).await,
            Backend::Claude(b) => b.answer(request_id, option_id).await,
        }
    }

    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.set_config_option(id, value).await,
            Backend::Codex(b) => b.set_config_option(id, value).await,
            Backend::Claude(b) => b.set_config_option(id, value).await,
        }
    }

    async fn cancel(&mut self) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.cancel().await,
            Backend::Codex(b) => b.cancel().await,
            Backend::Claude(b) => b.cancel().await,
        }
    }

    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        match self {
            Backend::Acp(b) => b.next_events().await,
            Backend::Codex(b) => b.next_events().await,
            Backend::Claude(b) => b.next_events().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_is_not_installed_says_so_rather_than_could_not_start() {
        // "could not FIND" rather than "could not start", because resolution
        // happens before the spawn. The distinction is the useful half: a
        // program that is not installed and one that is installed and crashes
        // need different things done about them, and this used to report both
        // as the same failure.
        let spec = AdapterSpec {
            backend: AdapterBackend::Acp,
            program: "farcooler-no-such-program".into(),
            args: vec![],
            env: Default::default(),
        };
        let failure = handshake("codex", &spec, std::time::Duration::from_secs(5)).expect_err("no program");
        assert!(failure.contains("could not find"), "{failure}");
        assert!(failure.contains("farcooler-no-such-program"), "names it: {failure}");
    }

    #[test]
    fn an_adapter_with_no_program_is_refused_without_spawning_anything() {
        let spec = AdapterSpec {
            backend: AdapterBackend::Acp,
            program: "  ".into(),
            args: vec![],
            env: Default::default(),
        };
        assert_eq!(
            handshake("codex", &spec, std::time::Duration::from_secs(5)).expect_err("refused"),
            "no program to run"
        );
    }

    #[test]
    fn an_agent_with_no_native_backend_says_so_rather_than_testing_acp() {
        // Silently falling back would be worse than refusing: the form would
        // report a working adapter for a protocol never spoken to. opencode has
        // no native backend and, unlike cursor, is not expected to grow one.
        let spec = AdapterSpec {
            backend: AdapterBackend::Native,
            program: "sh".into(),
            args: vec![],
            env: Default::default(),
        };
        let failure =
            handshake("opencode", &spec, std::time::Duration::from_secs(5)).expect_err("no backend");
        assert!(failure.contains("no native backend"), "{failure}");
        assert!(failure.contains("opencode"), "names the agent: {failure}");
    }

    #[test]
    fn every_built_in_adapter_ships_speaking_acp() {
        // Native backends exist before they can hold a conversation. Defaulting
        // any preset to one merely because the crate compiles would turn chat
        // mode off for that agent, which is the opposite of shipping a feature.
        for rules in farcooler_core::activity::Registry::built_in().all() {
            let Some(spec) = &rules.adapter else { continue };
            assert_eq!(
                spec.backend,
                AdapterBackend::Acp,
                "{} must ship on acp until its native backend can hold a conversation",
                rules.preset
            );
        }
    }

    #[test]
    fn a_native_spec_is_representable_even_though_nothing_ships_one_yet() {
        // The field is not decoration: a user can set it today, and Task 9 and
        // 10 read it. This asserts the type actually carries the choice.
        let spec = AdapterSpec {
            backend: AdapterBackend::Native,
            program: "codex".into(),
            args: Vec::new(),
            env: Default::default(),
        };
        assert_eq!(spec.backend, AdapterBackend::Native);
    }
}
