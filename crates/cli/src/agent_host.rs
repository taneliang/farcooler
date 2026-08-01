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

use overnight_agent::acp::conn::AcpConnection;
use overnight_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use overnight_agent::ring::{AgentReplay, AgentRing};
use overnight_agent::session::AgentSession;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use uuid::Uuid;

/// The error type every command in this binary already uses: a boxed
/// `std::error::Error`, chosen because `crates/cli` has no `anyhow`
/// dependency and this shim is one more command dispatched from `main.rs`,
/// not a reason to add one.
type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub enum Status {
    AdapterMissing { program: String },
    Connected { session_id: String },
}

pub fn status_line(status: &Status) -> String {
    match status {
        Status::AdapterMissing { program } => format!(
            "overnight: could not start the ACP adapter `{program}`.\n\
             Install it, or switch this terminal back to terminal mode — \
             terminal mode needs no adapter and is unaffected."
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

    let (mut agent, prelude) = AgentSession::start(conn, session).await?;
    println!("{}", status_line(&Status::Connected { session_id: agent.session_id.clone() }));

    let mut ring = AgentRing::new();
    for event in prelude {
        ring.push(event);
    }

    // The daemon may restart under us. Reconnect forever; the ring is what
    // makes that free.
    loop {
        let Ok(stream) = UnixStream::connect(&socket).await else {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        };
        if let Err(e) = serve(&mut agent, &mut ring, stream, terminal).await {
            tracing::warn!(terminal = %terminal, error = %e, "daemon link dropped; will reconnect");
        }
    }
}

async fn serve(
    agent: &mut AgentSession,
    ring: &mut AgentRing,
    stream: UnixStream,
    terminal: Uuid,
) -> Fallible {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    write_half
        .write_all(
            encode_line(&ShimMessage::Established {
                session_id: agent.session_id.clone(),
                available_modes: agent.available_modes.clone(),
            })?
            .as_bytes(),
        )
        .await?;

    let mut cursor = 0u64;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                match decode_line::<DaemonMessage>(&line)? {
                    DaemonMessage::Subscribe { from_seq } => {
                        cursor = from_seq;
                        let message = match ring.since(from_seq) {
                            AgentReplay::At { events } => ShimMessage::Events { events },
                            AgentReplay::Gap { resumed_at, dropped, events } =>
                                ShimMessage::Trimmed { resumed_at, dropped, events },
                        };
                        cursor = ring.next_seq();
                        write_half.write_all(encode_line(&message)?.as_bytes()).await?;
                    }
                    DaemonMessage::Prompt { text } => agent.prompt(&text).await?,
                    DaemonMessage::Answer { request_id, option_id } => {
                        let id: serde_json::Value = serde_json::from_str(&request_id)
                            .unwrap_or(serde_json::Value::String(request_id.clone()));
                        agent.answer(id, &option_id).await?;
                    }
                    DaemonMessage::SetMode { agent_mode } => agent.set_mode(&agent_mode).await?,
                    DaemonMessage::Cancel => agent.cancel().await?,
                }
            }
            pumped = agent.pump() => {
                let events = pumped?;
                if events.is_empty() { continue }
                let mut batch = Vec::with_capacity(events.len());
                for event in events {
                    let seq = ring.push(event.clone());
                    batch.push(overnight_agent::event::Sequenced { seq, event });
                }
                cursor = ring.next_seq();
                let _ = terminal;
                write_half
                    .write_all(encode_line(&ShimMessage::Events { events: batch })?.as_bytes())
                    .await?;
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
}
