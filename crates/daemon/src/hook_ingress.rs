//! Where a live agent session reports what it is doing.
//!
//! **One socket per daemon, not one per terminal**, which is the departure
//! from `agent_supervisor::socket_path` worth explaining. That socket is per
//! terminal because the shim is launched by the daemon and told which terminal
//! it is. A hook is launched by the AGENT and knows only its own `session_id`
//! and its worktree, so the routing has to happen on this side.
//!
//! The path is short for `agent_supervisor`'s reason, unchanged: `sun_path` is
//! 104 bytes on macOS and the default runtime directory already spends 66 of
//! them. A bind that fails here is silent in exactly the way that costs a
//! week, so it is logged loudly.
//!
//! Silence is the shape of every failure on this side, and that is worth
//! saying plainly. `farcooler hook` bounds its whole conversation at 400ms,
//! exits 0 and prints nothing whatever happens — which is the property that
//! keeps Far Cooler out of the way of somebody's agent, and also means a
//! listener that is slow, wrong or absent looks from the agent's end exactly
//! like a listener that is working. Nothing over there will ever complain
//! about anything done here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use farcooler_agent::event::AgentEvent;
use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::assemble::MessageAssembler;
use farcooler_agent_hooks::facts::{Facts, facts};
use farcooler_agent_hooks::wire::{HookLine, decode_line};
use farcooler_store::Store;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

#[derive(Clone)]
pub struct HookIngress {
    store: Arc<Store>,
    /// One assembler per terminal. Claude's prose arrives in pieces and the
    /// pieces of two panes must never be added to each other.
    ///
    /// Entries are added when a hook first routes to a terminal and removed by
    /// `forget`, which `Service` calls when a terminal's record is deleted.
    /// Without that call this map would hold an entry for every terminal this
    /// daemon has ever seen a hook from, each holding a marker for every
    /// message it ever displayed — a leak that grows with uptime and that
    /// nothing else in the process is positioned to notice.
    assemblers: Arc<Mutex<HashMap<Uuid, MessageAssembler>>>,
}

impl HookIngress {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store, assemblers: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Short, for `agent_supervisor::socket_path`'s reason.
    pub fn socket_path(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("h.sock")
    }

    /// The terminal a session belongs to, or `None`.
    ///
    /// `None` is not a failure. A person running an agent in a pane Far Cooler
    /// does not know about is an ordinary thing, and the only wrong answer
    /// here is a confident one — attaching a conversation to the wrong pane is
    /// invisible by construction, because a transcript from another session
    /// still looks like a transcript.
    ///
    /// Two answers are therefore refused as firmly as no answer: a session id
    /// two terminals both claim, and a worktree holding two panes of the same
    /// agent with nothing to tell them apart. This is `set_pane_mode`'s
    /// adoption rule ("an ambiguous lookup refuses rather than attaching a
    /// chat to the wrong conversation") arriving by the new route.
    pub fn terminal_for(&self, f: &Facts, agent: Agent) -> Option<Uuid> {
        let session = f.session_id.as_deref()?;
        let claimants = self.store.terminals_with_agent_session(session).ok()?;
        match claimants.as_slice() {
            [only] => return Some(only.id),
            [] => {}
            many => {
                tracing::debug!(
                    session,
                    claimants = many.len(),
                    "a session two terminals both claim reaches neither"
                );
                return None;
            }
        }
        self.announced_terminal(f, agent)
    }

    /// The terminal an agent that announces itself is sitting in.
    ///
    /// Claude declares its session at launch — `preset_command` passes
    /// `--session-id` and `create_terminal` mints it — so a claude session no
    /// row claims is a claude somebody started by hand, and guessing a
    /// worktree-mate for it would be the mistake `session_discovery`'s
    /// ambiguity refusal exists to avoid. Codex and cursor have no such flag
    /// and can only announce, so for those two a worktree match is the only
    /// binding there is.
    ///
    /// Keyed on the agent rather than on a `SessionStart` event name, because
    /// only codex sends one: cursor has no session-start hook at all, and
    /// gating on the name would leave every cursor session permanently
    /// unattached. The match is therefore standing rather than one-shot, which
    /// it can afford to be — nothing here writes, so re-resolving a session on
    /// its every hook costs two indexed reads and reaches the same answer.
    ///
    /// A pane that already names a session is never a candidate. It is
    /// speaking for a conversation, and handing it a second one would draw two
    /// sessions into one transcript.
    fn announced_terminal(&self, f: &Facts, agent: Agent) -> Option<Uuid> {
        if agent == Agent::Claude {
            return None;
        }
        let cwd = canonical(f.cwd.as_deref()?);
        let workspaces = self.store.list_all_workspaces().ok()?;

        let mut candidates: Vec<Uuid> = Vec::new();
        for ws in workspaces.iter().filter(|w| canonical(Path::new(&w.worktree_path)) == cwd) {
            let terminals = self.store.list_terminals_for_workspace(ws.id).ok()?;
            candidates.extend(
                terminals
                    .into_iter()
                    .filter(|t| t.agent_session_id.is_none())
                    .filter(|t| preset_agent(&t.command_preset) == Some(agent))
                    .map(|t| t.id),
            );
        }

        match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            many => {
                tracing::debug!(
                    agent = agent.as_str(),
                    worktree = %cwd.display(),
                    candidates = many.len(),
                    "two panes of one agent in one worktree; the announcement binds to neither"
                );
                None
            }
        }
    }

    /// Fold one hook firing into events for a terminal it has already been
    /// routed to.
    ///
    /// Separate from `serve` so the state it accumulates can be reached
    /// without a socket, which is what lets a test put a terminal into this
    /// map and then watch `Service` delete that terminal — the only way to
    /// prove `forget` is reached by anything other than itself.
    pub fn accept(
        &self,
        terminal: Uuid,
        agent: Agent,
        event: &str,
        payload: &serde_json::Value,
    ) -> Vec<AgentEvent> {
        let mut assemblers = self.assemblers.lock().unwrap_or_else(|e| e.into_inner());
        assemblers.entry(terminal).or_default().accept(agent, event, payload)
    }

    /// Drop the assembly state held for a terminal whose record is gone.
    ///
    /// Called from `Service`'s one delete path. A terminal that has been
    /// deleted can never be routed to again — `terminal_for` reads the same
    /// rows — so nothing here is reachable afterwards, and everything here is
    /// a partly-assembled message that will never be finished.
    pub fn forget(&self, terminal: Uuid) {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
    }

    /// Whether any assembly state is held for a terminal. For tests and for
    /// logs.
    pub fn is_tracking(&self, terminal: Uuid) -> bool {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
    }

    /// Bind this daemon's one hook socket and serve it until something goes
    /// wrong with the listener itself.
    ///
    /// Returns only on an error, so a caller spawns it. A failure to bind is
    /// the one worth being loud about: every hook then connects to nothing,
    /// exits 0 and prints nothing, and every live session on the runner is
    /// silently invisible with no symptom anywhere but here.
    pub async fn listen<F>(&self, runtime_dir: &Path, on_events: F) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<AgentEvent>) + Send + Sync + 'static,
    {
        let path = Self::socket_path(runtime_dir);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).inspect_err(|e| {
            tracing::error!(
                error = %e,
                path = %path.display(),
                bytes = path.as_os_str().len(),
                limit = crate::agent_supervisor::MAX_SOCKET_PATH,
                "could not bind the hook socket; no live session will report anything"
            );
        })?;
        let on_events = Arc::new(on_events);

        loop {
            let (stream, _) = listener.accept().await?;
            let this = self.clone();
            let on_events = on_events.clone();
            // One task per connection: a hook that is waiting on a decision
            // must not hold up every other hook on the runner.
            tokio::spawn(async move {
                if let Err(e) = this.serve(stream, on_events.as_ref()).await {
                    tracing::debug!(error = %e, "a hook connection ended");
                }
            });
        }
    }

    async fn serve<F>(&self, stream: UnixStream, on_events: &F) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<AgentEvent>),
    {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            // The half of the framing contract this side owns. `hook.rs`'s
            // `converse` may be cut off mid-write by its own deadline and says
            // so: "the daemon reads whole lines, and half a frame with no
            // newline is discarded at EOF rather than acted on".
            //
            // `read_line` cannot tell those apart on its own — it returns the
            // bytes it found before EOF exactly as it returns a finished line —
            // and a truncated frame can still be valid JSON, so nothing further
            // down would notice either. The terminator is the whole of the
            // check.
            if !line.ends_with('\n') {
                return Ok(());
            }
            let Ok(hook) = decode_line::<HookLine>(line.trim()) else {
                // A shape we cannot read is not worth a log line per flush.
                continue;
            };
            let f = facts(hook.agent, &hook.payload);
            let Some(terminal) = self.terminal_for(&f, hook.agent) else {
                tracing::debug!(session = ?f.session_id, "a hook from a session no terminal claims");
                continue;
            };

            let events = self.accept(terminal, hook.agent, &hook.event, &hook.payload);
            if !events.is_empty() {
                on_events(terminal, events);
            }
        }
    }
}

/// Which agent a command preset runs, when it is one of the three that hook.
///
/// A preset is `claude`, or `claude:opus` — `preset_command` splits the model
/// off the same way, and a preset naming a model is still the same agent.
fn preset_agent(preset: &str) -> Option<Agent> {
    preset.split(':').next()?.parse().ok()
}

/// A worktree path in one spelling.
///
/// `/tmp` is a symlink to `/private/tmp` on macOS and `$TMPDIR` sits under
/// another one, so the `cwd` an agent reports and the path git reported for
/// the worktree routinely differ by a symlink neither side chose. Comparing
/// the strings would leave every session in such a worktree unattached, and it
/// would do it silently, which is the failure this whole module is prone to.
///
/// A path that cannot be resolved — the worktree is gone, or it never existed,
/// as in a store-level test — falls back to what it was given, which compares
/// exactly as it would have before.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_names_its_agent_with_or_without_a_model() {
        assert_eq!(preset_agent("codex"), Some(Agent::Codex));
        assert_eq!(preset_agent("codex:gpt-5.6-terra"), Some(Agent::Codex));
        assert_eq!(preset_agent("cursor"), Some(Agent::Cursor));
        assert_eq!(preset_agent("shell"), None, "a shell pane hosts no agent");
        assert_eq!(preset_agent(""), None);
    }

    /// The socket has to fit, and the reason it might not is the runtime
    /// directory rather than anything chosen here.
    #[test]
    fn the_hook_socket_fits_in_a_unix_socket_path() {
        let real = Path::new("/Users/somebody/Library/Application Support/com.farcooler.Far Cooler");
        let path = HookIngress::socket_path(real);
        assert!(
            path.as_os_str().len() <= crate::agent_supervisor::MAX_SOCKET_PATH,
            "{} is {} bytes, over the {} a bind accepts",
            path.display(),
            path.as_os_str().len(),
            crate::agent_supervisor::MAX_SOCKET_PATH
        );
    }
}
