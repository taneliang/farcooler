//! The process a pane runs in agent pane mode.
//!
//! It exists so that a headless agent still has a tagged tmux pane, which keeps
//! `derive.rs` the single authority on whether anything is alive. It also owns
//! the event ring, because it is the process that lives exactly as long as the
//! pane does — a daemon restart therefore costs no history.
//!
//! What it prints to its own stdout is not decoration. The pane is real and
//! attachable, so this log is what a user sees when they go looking.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use farcooler_agent::acp::conn::AcpConnection;
use farcooler_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use farcooler_agent::ring::{AgentReplay, AgentRing};
use farcooler_agent::session::AgentSession;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

/// The error type every command in this binary already uses: a boxed
/// `std::error::Error`, chosen because `crates/cli` has no `anyhow`
/// dependency and this shim is one more command dispatched from `main.rs`,
/// not a reason to add one.
type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub enum Status {
    /// `preset` is what to call the agent (`claude`, `cursor`, …); `command` is
    /// what was actually run. Every npx-wrapped adapter shares the same
    /// `program` — `npx` — so a message keyed on `program` alone cannot say
    /// which of the four agents failed. It has to be `command` (`program` plus
    /// its args, which for an npx adapter is where the real package name
    /// lives) and `preset` together.
    AdapterMissing { preset: String, command: String },
    /// Started, then said nothing. Distinct from missing, because the advice is
    /// different and "it did not start" would be a lie.
    AdapterSilent { preset: String, command: String },
    Connected { session_id: String },
}

pub fn status_line(status: &Status) -> String {
    match status {
        Status::AdapterMissing { preset, command } => format!(
            "farcooler: could not start the ACP adapter for `{preset}` (`{command}`).\n\
             Install it, or switch this terminal back to terminal mode — \
             terminal mode needs no adapter and is unaffected."
        ),
        Status::AdapterSilent { preset, command } => format!(
            "farcooler: the ACP adapter for `{preset}` (`{command}`) started but never answered.\n\
             Nothing is wrong with this terminal — switch it back to terminal mode \
             and it will work as it always has.\n\
             One known cause: the Claude SDK refuses to launch inside another \
             Claude Code session, and neither answers nor exits. Check that the \
             daemon's environment has no CLAUDECODE variable set."
        ),
        Status::Connected { session_id } => {
            format!("farcooler: agent session {session_id} connected. Rendering natively.")
        }
    }
}

/// The adapter for a preset, or nothing.
///
/// Nothing rather than a default, deliberately. The shim used to know exactly
/// one adapter, so a pane hosting any other agent got Claude: a codex pane
/// switched to chat started a brand new Claude session in the same worktree and
/// rendered that, with nothing anywhere saying the agent had been swapped.
/// Refusing is a small disappointment; a different agent wearing the same pane
/// is a much larger one.
pub fn resolve(
    registry: &farcooler_core::activity::Registry,
    preset: Option<&str>,
) -> Option<farcooler_core::activity::AdapterSpec> {
    registry.adapter(preset?).cloned()
}

/// Whether this preset needs `CLAUDE_CODE_EXECUTABLE` resolved for it.
///
/// Claude alone. `claude_executable` probes the filesystem for Claude Code, so
/// applying it generally would put a Claude-specific variable into every other
/// agent's environment.
pub fn wants_claude_executable(preset: &str) -> bool {
    preset == "claude"
}

/// Where the Claude Agent SDK should find Claude Code.
///
/// Two failures made this necessary, and the second is why it is worth the
/// code. Without it the SDK reports "Claude native binary not found" — fair
/// enough. But when a WRAPPER script is first on `PATH` (a `claude` that is
/// really a shim around the real one), the SDK neither finds the binary nor
/// reports that it did not: `session/new` simply never answers, and an agent
/// pane sits blank forever with nothing to read.
///
/// So the real executable is resolved here and passed explicitly. An existing
/// `CLAUDE_CODE_EXECUTABLE` always wins — someone who set it meant it.
pub fn claude_executable() -> Option<String> {
    if let Ok(explicit) = std::env::var("CLAUDE_CODE_EXECUTABLE") {
        if !explicit.trim().is_empty() {
            return Some(explicit);
        }
    }
    // The installer's own location, preferred over `PATH` precisely because
    // `PATH` is where the wrapper lives.
    let home = std::env::var("HOME").ok()?;
    let candidate = std::path::Path::new(&home).join(".local/bin/claude");
    candidate.exists().then(|| candidate.display().to_string())
}

pub async fn run(
    terminal: Uuid,
    socket: PathBuf,
    worktree: PathBuf,
    session: Option<String>,
    preset: Option<String>,
) -> Fallible {
    let registry = farcooler_core::config::load_registry();
    let Some(spec) = resolve(&registry, preset.as_deref()) else {
        // Reached only if the daemon's check and this one disagree, which
        // means a config file changed between them. Loud, because the symptom
        // is otherwise an empty pane.
        let name = preset.unwrap_or_else(|| "this pane".to_string());
        println!(
            "farcooler: no ACP adapter is configured for `{name}`.\n\
             Switch this terminal back to terminal mode — it needs no adapter \
             and is unaffected."
        );
        std::future::pending::<()>().await;
        unreachable!()
    };
    let (program, args) = (spec.program.clone(), spec.args.clone());

    // Named here rather than read off `program` at the point of failure:
    // `resolve` only returns `Some(spec)` when `preset` was `Some`, so this is
    // always the real agent name, and it is what a missing/silent report needs
    // to say — `program` alone is `npx` for three of the four built-ins and
    // tells nobody which agent that was.
    let agent_label = preset.clone().unwrap_or_else(|| "this agent".to_string());
    let command = std::iter::once(program.as_str())
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    for (key, value) in &spec.env {
        // SAFETY: set before any thread is spawned that reads the environment,
        // and this process exists to host exactly one adapter.
        unsafe { std::env::set_var(key, value) };
    }

    if preset.as_deref().is_some_and(wants_claude_executable) {
        if let Some(executable) = claude_executable() {
            // SAFETY: as above.
            unsafe { std::env::set_var("CLAUDE_CODE_EXECUTABLE", executable) };
        }
    }

    let conn = match AcpConnection::spawn(&program, &args, &worktree).await {
        Ok(c) => c,
        Err(_) => {
            println!(
                "{}",
                status_line(&Status::AdapterMissing {
                    preset: agent_label,
                    command
                })
            );
            // Stay alive so the pane does not vanish and derive as an exit the
            // user never caused. They read the message and switch modes.
            std::future::pending::<()>().await;
            unreachable!()
        }
    };

    // Bounded, because an adapter that starts but never answers `initialize`
    // is a real state and it looks like nothing at all: the process is alive,
    // the pane is `running`, and the screen stays blank forever with no way for
    // a user to tell whether it is slow or wedged. Observed in practice when
    // the Claude SDK refuses to launch nested inside another Claude Code
    // session — it neither answers nor exits.
    //
    // Generous, because a cold `npx` genuinely has to fetch a package on first
    // use, and killing that would be worse than waiting.
    let started = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        AgentSession::start(conn, session),
    )
    .await;

    let (agent, prelude) = match started {
        Ok(result) => result?,
        Err(_) => {
            println!(
                "{}",
                status_line(&Status::AdapterSilent {
                    preset: agent_label,
                    command
                })
            );
            // Alive on purpose, exactly as `AdapterMissing` is: a pane that
            // exits here derives as an exit nobody caused, and the message the
            // user needs to read goes with it.
            std::future::pending::<()>().await;
            unreachable!()
        }
    };
    println!("{}", status_line(&Status::Connected { session_id: agent.session_id.clone() }));

    // Captured before `into_running()` consumes `agent`: `RunningSession`
    // does not carry these, since nothing after startup needs to send them
    // anywhere but the `Established` message a fresh daemon connection wants.
    let session_id = agent.session_id.clone();
    let available_modes = agent.available_modes.clone();

    let ring = Arc::new(Mutex::new(AgentRing::new()));
    {
        let mut ring = ring.lock().expect("ring mutex");
        for event in prelude {
            ring.push(event);
        }
    }

    // The daemon has nobody to answer `fs/*` and permission requests while it
    // is down or reconnecting, so pumping cannot live inside the daemon-link
    // loop below — that loop reconnects and can be entirely absent for
    // arbitrary stretches, and the agent must keep working through all of
    // them. This task is the ONLY place `RunningSession` is touched, and it
    // runs independent of whether anyone is subscribed to the ring.
    let mut running = agent.into_running();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<DaemonMessage>();
    // Signals the daemon-link loop that the ring has new events, so a
    // connected daemon does not have to poll. `notify_one` (not
    // `notify_waiters`) because a permit must survive the daemon being
    // disconnected at push time — the very case Bug 2 is about — and be
    // delivered to whichever `serve` call is listening when it reconnects.
    let notify = Arc::new(Notify::new());

    {
        let ring = ring.clone();
        let notify = notify.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Both branches are `mpsc` receives and NOTHING ELSE. That
                    // is the whole point, and it is not a stylistic choice.
                    //
                    // `recv_frame` only receives; the frame is handled below,
                    // outside the select, where nothing can cancel it. Racing
                    // `next_events` here instead would reintroduce the bug this
                    // structure exists to kill: it awaits `respond` after
                    // `handle_fs_write` has already touched the disk, so losing
                    // the race leaves the file written and the agent waiting
                    // forever for an answer that never comes.
                    frame = running.recv_frame() => {
                        let frame = match frame {
                            Ok(frame) => frame,
                            Err(e) => {
                                tracing::warn!(terminal = %terminal, error = %e, "agent adapter closed; nothing left to pump");
                                return;
                            }
                        };
                        // Handled to completion, uncancellable by construction.
                        match running.handle(frame).await {
                            Ok(events) => {
                                if events.is_empty() { continue; }
                                let mut ring = ring.lock().expect("ring mutex");
                                for event in events {
                                    ring.push(event);
                                }
                                drop(ring);
                                notify.notify_one();
                            }
                            Err(e) => {
                                tracing::warn!(terminal = %terminal, error = %e, "agent adapter closed; nothing left to pump");
                                return;
                            }
                        }
                    }
                    cmd = cmd_rx.recv() => {
                        let Some(cmd) = cmd else { return }; // every sender (all daemon-link tasks) is gone: shim is shutting down
                        // A command can produce events of its own — a queued
                        // prompt is not something the agent will ever announce,
                        // because the agent has not been told about it.
                        let mut produced: Vec<farcooler_agent::event::AgentEvent> = Vec::new();
                        let result = match cmd {
                            DaemonMessage::Prompt { text, images } => match running.prompt(&text, images).await {
                                Ok(events) => {
                                    produced = events;
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            },
                            DaemonMessage::EditQueued { id, text } => {
                                produced = running.edit_queued(&id, &text);
                                Ok(())
                            }
                            DaemonMessage::CancelQueued { id } => {
                                produced = running.cancel_queued(&id);
                                Ok(())
                            }
                            DaemonMessage::SteerQueued { id } => {
                                match running.steer_queued(&id).await {
                                    Ok(events) => {
                                        produced = events;
                                        Ok(())
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            DaemonMessage::Answer { request_id, option_id } => {
                                let id: serde_json::Value = serde_json::from_str(&request_id)
                                    .unwrap_or(serde_json::Value::String(request_id));
                                running.answer(id, &option_id).await
                            }
                            DaemonMessage::SetMode { agent_mode } => running.set_mode(&agent_mode).await,
                            DaemonMessage::SetModel { model } => running.set_model(&model).await,
                            DaemonMessage::SetConfig { id, value } => {
                                running.set_config_option(&id, &value).await
                            }
                            DaemonMessage::Cancel => running.cancel().await,
                            // The daemon-link loop answers `Subscribe` itself
                            // by reading the ring directly; it never reaches
                            // this channel.
                            DaemonMessage::Subscribe { .. } => Ok(()),
                        };
                        if let Err(e) = result {
                            tracing::warn!(terminal = %terminal, error = %e, "a daemon command could not reach the agent");
                        }
                        if !produced.is_empty() {
                            let mut ring = ring.lock().expect("ring mutex");
                            for event in produced {
                                ring.push(event);
                            }
                            drop(ring);
                            notify.notify_one();
                        }
                    }
                }
            }
        });
    }

    // The daemon may restart under us, or simply not be up yet. Reconnect
    // forever; the ring plus `notify` is what makes that free — nothing
    // above this loop depends on a connection existing.
    let mut cursor = 0u64;
    loop {
        let Ok(stream) = UnixStream::connect(&socket).await else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        if let Err(e) =
            serve(&session_id, &available_modes, &ring, &notify, &cmd_tx, stream, &mut cursor).await
        {
            tracing::warn!(terminal = %terminal, error = %e, "daemon link dropped; will reconnect");
        }
    }
}

/// One daemon connection's lifetime.
///
/// This function never touches `RunningSession` — it only reads the shared
/// ring and forwards commands over `cmd_tx` to the task that owns the
/// session. That is what lets a daemon reconnect (a brand new call to this
/// function, with a brand new socket) without any risk of two places racing
/// to write to the agent's stdin at once.
async fn serve(
    session_id: &str,
    available_modes: &[String],
    ring: &Arc<Mutex<AgentRing>>,
    notify: &Notify,
    cmd_tx: &mpsc::UnboundedSender<DaemonMessage>,
    stream: UnixStream,
    cursor: &mut u64,
) -> Fallible {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    write_half
        .write_all(
            encode_line(&ShimMessage::Established {
                session_id: session_id.to_string(),
                available_modes: available_modes.to_vec(),
            })?
            .as_bytes(),
        )
        .await?;

    // Nothing is pushed until the daemon has said where it wants to start.
    //
    // A connection begins with the daemon clearing its transcript and asking
    // for the whole ring, because a shim numbers from zero and the only honest
    // cursor for a new stream is 0. But `notify` holds a permit from every
    // event that arrived while no daemon was connected — so the push arm fired
    // first, sent the ring, and then the `Subscribe` reply sent the very same
    // events again. The daemon appended both, and every message in the pane
    // appeared TWICE after a toggle.
    //
    // Waiting makes the order of a connection definite: established, asked,
    // answered once, and only then streamed.
    let mut subscribed = false;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                match decode_line::<DaemonMessage>(&line)? {
                    DaemonMessage::Subscribe { from_seq } => {
                        subscribed = true;
                        let message = {
                            let ring = ring.lock().expect("ring mutex");
                            let message = match ring.since(from_seq) {
                                AgentReplay::At { events } => ShimMessage::Events { events },
                                AgentReplay::Gap { resumed_at, dropped, events } =>
                                    ShimMessage::Trimmed { resumed_at, dropped, events },
                            };
                            *cursor = ring.next_seq();
                            message
                        };
                        write_half.write_all(encode_line(&message)?.as_bytes()).await?;
                    }
                    // Prompt/Answer/SetMode/Cancel all need `RunningSession`,
                    // which only the session task in `run` is allowed to
                    // touch — forwarding rather than handling inline is what
                    // keeps a reconnect from ever racing that task.
                    other => {
                        let _ = cmd_tx.send(other);
                    }
                }
            }
            _ = notify.notified(), if subscribed => {
                let message = {
                    let ring = ring.lock().expect("ring mutex");
                    match ring.since(*cursor) {
                        AgentReplay::At { events } => {
                            if events.is_empty() { continue }
                            *cursor = ring.next_seq();
                            ShimMessage::Events { events }
                        }
                        AgentReplay::Gap { resumed_at, dropped, events } => {
                            *cursor = ring.next_seq();
                            ShimMessage::Trimmed { resumed_at, dropped, events }
                        }
                    }
                };
                write_half.write_all(encode_line(&message)?.as_bytes()).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole ring, exactly once — not once because it was pushed and again
    /// because it was asked for.
    ///
    /// A shim buffers events while no daemon is connected, and every one of
    /// them leaves a `Notify` permit behind. So a fresh connection had two
    /// things ready to send the same events: the push arm, holding that permit,
    /// and the reply to the daemon's opening `Subscribe { from_seq: 0 }`. Both
    /// fired. The daemon appended both — it numbers by position and has nothing
    /// to deduplicate against by design — and every message in the pane
    /// rendered twice after a toggle.
    #[tokio::test]
    async fn a_connection_sends_each_event_once_even_with_a_notify_already_pending() {
        use farcooler_agent::event::{AgentEvent, Role};

        let ring = Arc::new(Mutex::new(AgentRing::new()));
        let notify = Arc::new(Notify::new());
        for text in ["hello", "world"] {
            ring.lock().expect("ring").push(AgentEvent::Message {
                role: Role::User,
                text: text.into(), parent: None });
            // Exactly what the session task does, and what leaves the permit
            // sitting there with nobody connected to receive it.
            notify.notify_one();
        }

        let (daemon_side, shim_side) = tokio::net::UnixStream::pair().expect("socketpair");
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let served = {
            let ring = Arc::clone(&ring);
            let notify = Arc::clone(&notify);
            tokio::spawn(async move {
                let mut cursor = 0;
                let _ = serve("s", &[], &ring, &notify, &cmd_tx, shim_side, &mut cursor).await;
            })
        };

        let (read_half, mut write_half) = daemon_side.into_split();
        let mut lines = BufReader::new(read_half).lines();

        let established = lines.next_line().await.expect("read").expect("established");
        assert!(established.contains("\"established\""), "{established}");

        // The daemon asks from zero, because it just cleared its own
        // transcript — see `AgentSupervisor::serve`.
        write_half
            .write_all(
                encode_line(&DaemonMessage::Subscribe { from_seq: 0 }).expect("encode").as_bytes(),
            )
            .await
            .expect("subscribe");

        // Read everything the shim has to say for a moment, then count.
        let mut delivered = 0;
        while let Ok(Ok(Some(line))) =
            tokio::time::timeout(std::time::Duration::from_millis(150), lines.next_line()).await
        {
            delivered += line.matches("\"hello\"").count();
        }
        served.abort();

        assert_eq!(delivered, 1, "the ring was sent twice: once pushed, once asked for");
    }

    #[test]
    fn a_status_line_is_readable_by_a_person_looking_at_the_pane() {
        // The pane is a real pane and the user can attach to it. When the
        // adapter cannot start, what is written here is the entire error
        // message they get, so it has to stand alone.
        let line = status_line(&Status::AdapterMissing {
            preset: "cursor".into(),
            command: "npx -y cursor-agent-acp".into(),
        });
        assert!(line.contains("cursor"), "must name the agent, not just the runner: {line}");
        assert!(line.contains("npx -y cursor-agent-acp"));
        assert!(line.contains("terminal mode"), "must name the working fallback: {line}");
    }

    #[test]
    fn the_status_line_names_the_agent_not_just_npx() {
        // The failure this replaced: `AdapterMissing`/`AdapterSilent` used to
        // carry only `program`, which is `npx` for three of the four built-ins.
        // Claude, codex and cursor all failing with an identical message —
        // "could not start the ACP adapter `npx`" — told a user nothing about
        // which agent had actually broken.
        let claude = status_line(&Status::AdapterMissing {
            preset: "claude".into(),
            command: "npx -y @agentclientprotocol/claude-agent-acp".into(),
        });
        let codex = status_line(&Status::AdapterMissing {
            preset: "codex".into(),
            command: "npx -y @agentclientprotocol/codex-acp".into(),
        });
        assert!(claude.contains("claude"));
        assert!(codex.contains("codex"));
        assert_ne!(claude, codex, "two different agents must not read as the same failure");
    }

    #[test]
    fn the_preset_chooses_the_adapter() {
        let registry = farcooler_core::activity::Registry::built_in();
        let spec = resolve(&registry, Some("codex")).expect("codex has an adapter");
        assert_eq!(spec.program, "npx");
        assert!(spec.args.iter().any(|a| a == "@agentclientprotocol/codex-acp"));
        // The maintained package, not the renamed one. npm reports
        // `@zed-industries/codex-acp` deprecated and it stalled at 0.16.0.
        assert!(!spec.args.iter().any(|a| a.contains("@zed-industries/")));
    }

    #[test]
    fn an_unknown_preset_resolves_to_nothing_rather_than_to_claude() {
        // The failure this prevents: the shim knew one adapter, so a codex pane
        // switched to chat started a brand new CLAUDE session in the same worktree
        // and drew that, with nothing saying the agent had been swapped.
        let registry = farcooler_core::activity::Registry::built_in();
        assert!(resolve(&registry, Some("nothing-by-this-name")).is_none());
        assert!(resolve(&registry, None).is_none());
    }

    #[test]
    fn only_claude_gets_the_claude_executable_treatment() {
        // It probes the filesystem for Claude Code specifically. Applying it to
        // every adapter would set a Claude variable in codex's environment.
        assert!(wants_claude_executable("claude"));
        assert!(!wants_claude_executable("codex"));
        assert!(!wants_claude_executable("opencode"));
    }

    #[test]
    fn a_silent_adapter_is_not_reported_as_a_missing_one() {
        // Different cause, different advice. Telling a user it "could not
        // start" when it started and went quiet sends them to reinstall a
        // package that is already there.
        let silent = status_line(&Status::AdapterSilent {
            preset: "opencode".into(),
            command: "opencode acp".into(),
        });
        assert!(silent.contains("opencode"), "must name the agent: {silent}");
        assert!(silent.contains("never answered"), "{silent}");
        assert!(silent.contains("terminal mode"), "must name the working fallback: {silent}");
        assert!(!silent.contains("could not start"), "{silent}");
    }
}
