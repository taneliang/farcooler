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

use overnight_agent::acp::conn::AcpConnection;
use overnight_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use overnight_agent::ring::{AgentReplay, AgentRing};
use overnight_agent::session::AgentSession;
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
    AdapterMissing { program: String },
    /// Started, then said nothing. Distinct from missing, because the advice is
    /// different and "it did not start" would be a lie.
    AdapterSilent { program: String },
    Connected { session_id: String },
}

pub fn status_line(status: &Status) -> String {
    match status {
        Status::AdapterMissing { program } => format!(
            "overnight: could not start the ACP adapter `{program}`.\n\
             Install it, or switch this terminal back to terminal mode — \
             terminal mode needs no adapter and is unaffected."
        ),
        Status::AdapterSilent { program } => format!(
            "overnight: the ACP adapter `{program}` started but never answered.\n\
             Nothing is wrong with this terminal — switch it back to terminal mode \
             and it will work as it always has.\n\
             One known cause: the Claude SDK refuses to launch inside another \
             Claude Code session, and neither answers nor exits. Check that the \
             daemon's environment has no CLAUDECODE variable set."
        ),
        Status::Connected { session_id } => {
            format!("overnight: agent session {session_id} connected. Rendering natively.")
        }
    }
}

/// The adapter Overnight uses when preferences name none.
pub fn default_adapter() -> (String, Vec<String>) {
    (
        "npx".to_string(),
        vec!["-y".to_string(), "@zed-industries/claude-code-acp".to_string()],
    )
}

pub async fn run(
    terminal: Uuid,
    socket: PathBuf,
    worktree: PathBuf,
    session: Option<String>,
    adapter: Option<String>,
) -> Fallible {
    let (program, args) = match adapter {
        Some(a) => (a, Vec::new()),
        None => default_adapter(),
    };

    let conn = match AcpConnection::spawn(&program, &args, &worktree).await {
        Ok(c) => c,
        Err(_) => {
            println!("{}", status_line(&Status::AdapterMissing { program }));
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
            println!("{}", status_line(&Status::AdapterSilent { program }));
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
                        let result = match cmd {
                            DaemonMessage::Prompt { text } => running.prompt(&text).await,
                            DaemonMessage::Answer { request_id, option_id } => {
                                let id: serde_json::Value = serde_json::from_str(&request_id)
                                    .unwrap_or(serde_json::Value::String(request_id));
                                running.answer(id, &option_id).await
                            }
                            DaemonMessage::SetMode { agent_mode } => running.set_mode(&agent_mode).await,
                            DaemonMessage::Cancel => running.cancel().await,
                            // The daemon-link loop answers `Subscribe` itself
                            // by reading the ring directly; it never reaches
                            // this channel.
                            DaemonMessage::Subscribe { .. } => Ok(()),
                        };
                        if let Err(e) = result {
                            tracing::warn!(terminal = %terminal, error = %e, "a daemon command could not reach the agent");
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

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                match decode_line::<DaemonMessage>(&line)? {
                    DaemonMessage::Subscribe { from_seq } => {
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
            _ = notify.notified() => {
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

    #[test]
    fn a_status_line_is_readable_by_a_person_looking_at_the_pane() {
        // The pane is a real pane and the user can attach to it. When the
        // adapter cannot start, what is written here is the entire error
        // message they get, so it has to stand alone.
        let line = status_line(&Status::AdapterMissing { program: "claude-code-acp".into() });
        assert!(line.contains("claude-code-acp"));
        assert!(line.contains("terminal mode"), "must name the working fallback: {line}");
    }

    #[test]
    fn the_default_adapter_is_the_zed_package() {
        let (program, args) = default_adapter();
        assert_eq!(program, "npx");
        assert!(args.iter().any(|a| a.contains("@zed-industries/claude-code-acp")));
    }

    #[test]
    fn a_silent_adapter_is_not_reported_as_a_missing_one() {
        // Different cause, different advice. Telling a user it "could not
        // start" when it started and went quiet sends them to reinstall a
        // package that is already there.
        let silent = status_line(&Status::AdapterSilent { program: "npx".into() });
        assert!(silent.contains("never answered"), "{silent}");
        assert!(silent.contains("terminal mode"), "must name the working fallback: {silent}");
        assert!(!silent.contains("could not start"), "{silent}");
    }
}
