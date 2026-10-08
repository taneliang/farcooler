//! Stop and Send Now from the native view (ov-368): `terminal.interrupt`
//! presses one Esc in a working claude's terminal pane, and
//! `terminal.send_now` presses claude's ctrl+x ctrl+s, which sends what
//! waits in its queue now (R-29). Each is one key, pressed once, only past
//! the gate below, and answered only once claude's transcript or registry
//! says it took.
//!
//! **What the keys do**, measured on claude 2.1.290 against a stand-in API:
//! - One Esc while a reply streams stops it: the reply so far is recorded
//!   with `isAbortedMidStream`, then a `user` record `[Request interrupted
//!   by user]`, within 70 ms, and the registry reads `idle`. No hook fires,
//!   not even `Stop`. Pressed before any reply, the prompt goes back into the
//!   box instead, and nothing is recorded but the registry turning idle.
//! - One Esc while a tool runs kills it: an error `tool_result` ("User
//!   rejected tool use") and `[Request interrupted by user for tool use]`. No
//!   `PostToolUse` fires, so the session's hooks keep the call in flight
//!   until the next turn boundary, which fails closed.
//! - One Esc on a permission dialog answers it No and ends the turn: never
//!   pressed while one may be up. While a dialog is up the registry says
//!   `waiting`, not `busy`.
//! - With a message queued, Esc stops the turn and the message runs at once.
//!   A draft in the box survives one Esc; a second within about 0.6 s clears
//!   it (and on an empty box opens the rewind picker), hence the lockout.
//! - ctrl+x ctrl+s with messages queued, as a reply streams, stops the reply
//!   and sends every queued message together as the next turn: a
//!   `queue-operation` `dequeue` for each, then each as a `user` record with
//!   `promptSource: "queued"`. No `UserPromptSubmit` fires for them then;
//!   that fired when each was queued. While a tool runs, it moves the command
//!   to the background and hands the messages to the turn running
//!   (`remove`, `reason: "absorbed_mid_turn"`). With nothing queued it does
//!   nothing; but between turns it submits whatever draft is in the box, so
//!   it's never pressed over a draft.
//!
//! **The gate.** Every check fails closed, with nothing pressed; each
//! refusal is a `DomainError::Conflict` naming why with a stable word:
//! 1. a terminal-mode pane launched as an agent, running (`not_an_agent`,
//!    `not_running`; a chat pane has `terminal.agent_cancel`);
//! 2. nothing else typing into its box (`Watcher::typing`, waited for up to
//!    `TYPING_WAIT`: `sending`);
//! 3. no key pressed there in the last `LOCKOUT` (`too_soon`), and nobody
//!    typed there in the last `TYPED_WITHIN_MS` (`typing`);
//! 4. claude in front, proven by its process (`not_an_agent`; codex and
//!    cursor are `unsupported`);
//! 5. its registry says `busy` (`idle`; `waiting` or no status is
//!    `prompt`; no live entry is `unconfirmable`);
//! 6. the screen shows its box, not a dialog or a panel (`prompt`,
//!    `unfamiliar`); for Send Now an empty box (`draft`);
//! 7. its transcript found, and its hooks heard by this daemon, whose fence
//!    the key is pressed under (`unconfirmable`);
//! 8. no ask held on the pane (`prompt`), and for Send Now a message waiting
//!    in claude's queue (`nothing_queued`);
//! 9. under the fence, a dialog that may be coming waited out in time
//!    (`settling`), and checks 3 to 8 again.
//!
//! **The key.** Under the session's fence, so no `PreToolUse` is answered and
//! no new dialog drawn while it lands (`mid_turn`). The fence is taken first;
//! then a gate begun in the last two seconds, or a call begun before the
//! fence and in the last `CALL_SETTLES`, is waited out under it
//! (`HookAsks::settles_in`), as its dialog may be coming; then every check
//! runs again. A call marked once the fence is held waits on it, so the
//! wait converges. That wait and the checks get `UNDER_FENCE`, else
//! `settling`; then the key is sent once and the fence held `KEY_LANDS` past
//! it. A confirmed Stop ends the main thread's calls begun before the key,
//! which no hook ends (`HookAsks::calls_ended_before`).
//!
//! **The confirmation.** Stop: an interrupted record past the transcript's
//! length before the key, or the registry turning idle. Send Now: a
//! `dequeue`, an absorbed `remove` or a queued prompt past that length.
//! Neither within `CONFIRM_SETTLES`: `unconfirmed`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_core::composer::Composer;
use farcooler_core::{DomainError, Result};
use farcooler_store::models::{PaneMode, Terminal};
use uuid::Uuid;

use super::mid_turn::{self, KEY_LANDS, LATE_KEY};
use super::registry_turn::{self, Status};
use super::tell::held_word;
use super::{Held, PASTE_POLL, may_be_typed_to};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// The least time between two keys pressed in one terminal: past the 0.6 s
/// in which claude reads a second Esc as clearing the box.
pub(crate) const LOCKOUT: Duration = Duration::from_millis(1_500);

/// How recently a person's key in the pane stops one of these.
pub(crate) const TYPED_WITHIN_MS: i64 = 3_000;

/// The longest the key waits for another send to let go of the box.
const TYPING_WAIT: Duration = Duration::from_secs(10);

/// How long after the key claude has to say it took.
const CONFIRM_SETTLES: Duration = Duration::from_secs(3);

/// The longest the gate's wait for a dialog that may be coming, and its
/// checks again, take under the fence. Past it: `settling`, nothing pressed.
const UNDER_FENCE: Duration = Duration::from_millis(3_000);

/// The longest the key's `tmux send-keys` may take before it's killed.
const KEY_DEADLINE: Duration = Duration::from_millis(1_500);

// The fence is held at most `UNDER_FENCE`, then the key's send, then
// `LATE_KEY` for a send that went wrong and `KEY_LANDS`: all inside the
// longest a `PreToolUse` waits on it, so a late key never outlives the fence.
const _: () = assert!(
    UNDER_FENCE.as_millis() + KEY_DEADLINE.as_millis() + LATE_KEY.as_millis() + KEY_LANDS.as_millis()
        < crate::hook_asks::FENCE_HOLD.as_millis()
);

/// Which key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    /// One Esc: stop the turn.
    Interrupt,
    /// ctrl+x ctrl+s: send what's queued now.
    SendNow,
}

impl Key {
    fn hex(self) -> &'static str {
        match self {
            Key::Interrupt => "1b",
            Key::SendNow => "1813",
        }
    }
}

/// The claude a key goes to, as the gate found it.
struct Target {
    session: String,
    transcript: PathBuf,
    config: PathBuf,
    pid: i32,
}

impl Watcher {
    /// Press `key` in the claude in terminal `id`, past this module's gate,
    /// and answer once claude says it took. On a task of its own, so a caller
    /// that goes away can't let go of the fence while the key may still land.
    pub(crate) async fn press(&self, id: Uuid, key: Key) -> Result<()> {
        let Some(me) = self.me.upgrade() else { return Err(DomainError::OperationFailed) };
        tokio::spawn(async move { me.press_here(id, key).await }).await.unwrap_or(Err(DomainError::OperationFailed))
    }

    async fn press_here(&self, id: Uuid, key: Key) -> Result<()> {
        let to = self.service.store.get_terminal(id)?;
        if to.pane_mode == PaneMode::Agent {
            // A chat pane stops through its own channel (`terminal.agent_cancel`).
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode.is_client_drawn() || !may_be_typed_to(&to.command_preset, to.role) {
            return Err(conflict("not_an_agent"));
        }
        if !self.service.is_running(&to) {
            return Err(conflict("not_running"));
        }
        let Ok(_typing) = tokio::time::timeout(TYPING_WAIT, self.typing(to.id)).await else {
            return Err(conflict("sending"));
        };
        let asks = self.service.hooks().asks().clone();
        let target = self.pressable(&to, key).await?;
        self.before_key_for_tests();
        let Some(fence) = asks.fence(&target.session) else { return Err(conflict("unconfirmable")) };
        let fenced = fence.lock_owned().await;
        // From here no `PreToolUse` is answered, so no call begun now can
        // raise a dialog. One begun before may still: its gate, or its first
        // `CALL_SETTLES`, is waited out here, and every check runs again.
        // Both inside `UNDER_FENCE`, failing closed (`settling`), so the key
        // and a late one's hold stay inside `FENCE_HOLD`.
        let fenced_at = Instant::now();
        let checked = tokio::time::timeout(UNDER_FENCE, async {
            while let Some(left) = asks.settles_in(&target.session, fenced_at) {
                if fenced_at.elapsed() + left >= UNDER_FENCE {
                    return Err(conflict("settling"));
                }
                tokio::time::sleep(left).await;
            }
            #[cfg(test)]
            tokio::time::sleep(Duration::from_millis(self.slow_recheck_ms.load(std::sync::atomic::Ordering::SeqCst))).await;
            self.pressable(&to, key).await
        })
        .await;
        let again = match checked {
            Ok(Ok(again)) if again.session == target.session => again,
            Ok(Ok(_)) => return Err(conflict("unconfirmable")),
            Ok(Err(refused)) => return Err(refused),
            Err(_) => return Err(conflict("settling")),
        };
        let from = std::fs::metadata(&again.transcript).map(|m| m.len()).unwrap_or(0);
        let pressed_at = Instant::now();
        self.keys_pressed.lock().unwrap_or_else(|e| e.into_inner()).insert(to.id, pressed_at);
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        match tokio::time::timeout(KEY_DEADLINE, runtime.send_bytes_hex(to.id, key.hex())).await {
            Ok(Ok(())) => tokio::time::sleep(KEY_LANDS).await,
            Ok(Err(_)) | Err(_) => {
                tracing::warn!(terminal = %to.id, "a key didn't go cleanly; holding the fence while it may still land");
                tokio::time::sleep(LATE_KEY).await;
                return Err(DomainError::OperationFailed);
            }
        }
        drop(fenced);
        if !confirmed(key, &again, from).await {
            return Err(conflict("unconfirmed"));
        }
        // Every main-thread call begun before the Esc ended with it: killed,
        // or never run. No hook says so (measured), so they'd keep the
        // session busy for mid-turn sends until the next boundary.
        if key == Key::Interrupt {
            asks.calls_ended_before(&again.session, pressed_at);
        }
        Ok(())
    }

    /// The gate's checks 3 to 8, against the pane as it is now.
    async fn pressable(&self, to: &Terminal, key: Key) -> Result<Target> {
        let last = self.keys_pressed.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if last.is_some_and(|at| at.elapsed() < LOCKOUT) {
            return Err(conflict("too_soon"));
        }
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| now_millis() - at < TYPED_WITHIN_MS) {
            return Err(conflict("typing"));
        }
        let (preset, _, pid) = self.proven_agent(to).await.map_err(|held| conflict(held_word(held)))?;
        if preset != "claude" {
            return Err(conflict("unsupported"));
        }
        let config = registry_turn::config_of(pid).await.ok_or(conflict("unconfirmable"))?;
        match registry_turn::status(&config, pid) {
            Status::Busy => {}
            Status::Idle => return Err(conflict("idle")),
            Status::Waiting => return Err(conflict("prompt")),
            Status::Nothing => return Err(conflict("unconfirmable")),
        }
        match self.box_of(to, preset).await {
            Ok(Ok((Composer::Empty, _))) => {}
            Ok(Ok((Composer::Holds(_), _))) if key == Key::Interrupt => {}
            Ok(Ok((Composer::Holds(_), _))) => return Err(conflict("draft")),
            Ok(Err(Held::Prompt)) => return Err(conflict("prompt")),
            Ok(Ok((Composer::Unrecognized, _)) | Err(_)) | Err(_) => return Err(conflict("unfamiliar")),
        }
        let (session, transcript) = mid_turn::transcript_in(&config, pid).ok_or(conflict("unconfirmable"))?;
        let asks = self.service.hooks().asks();
        if !asks.hooked(&session) {
            return Err(conflict("unconfirmable"));
        }
        if asks.is_holding(to.id) {
            return Err(conflict("prompt"));
        }
        if key == Key::SendNow && !waiting_in_queue(&transcript) {
            return Err(conflict("nothing_queued"));
        }
        Ok(Target { session, transcript, config, pid })
    }

    /// Run the hook a test set to act after the first checks, as the fence is taken.
    fn before_key_for_tests(&self) {
        #[cfg(test)]
        if let Some(run) = self.before_key.lock().unwrap_or_else(|e| e.into_inner()).take() {
            run();
        }
    }
}

fn conflict(what: &'static str) -> DomainError {
    DomainError::Conflict { what }
}

/// Whether claude said it took `key`, within `CONFIRM_SETTLES`. See this
/// module's docs, "The confirmation".
async fn confirmed(key: Key, target: &Target, from: u64) -> bool {
    let deadline = tokio::time::Instant::now() + CONFIRM_SETTLES;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(PASTE_POLL).await;
        let took = match key {
            Key::Interrupt => {
                interrupted_since(&target.transcript, from)
                    || registry_turn::status(&target.config, target.pid) == Status::Idle
            }
            Key::SendNow => sent_from_queue_since(&target.transcript, from),
        };
        if took {
            return true;
        }
    }
    false
}

/// The transcript's records past byte `from`.
fn since(path: &Path, from: u64) -> Vec<serde_json::Value> {
    mid_turn::records_between(path, from, from + mid_turn::LONGEST_READ)
}

fn text_of(record: &serde_json::Value, key: &str) -> Option<String> {
    record.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// Whether claude recorded a turn interrupted past byte `from`: a `user`
/// record whose text begins `[Request interrupted by user`, for a reply or
/// for a tool (measured on 2.1.290).
pub(crate) fn interrupted_since(path: &Path, from: u64) -> bool {
    since(path, from).iter().any(|record| {
        if text_of(record, "type").as_deref() != Some("user") {
            return false;
        }
        let texts: Vec<String> = match record.get("message").and_then(|m| m.get("content")) {
            Some(serde_json::Value::String(text)) => vec![text.clone()],
            Some(serde_json::Value::Array(blocks)) => blocks.iter().filter_map(|b| text_of(b, "text")).collect(),
            _ => Vec::new(),
        };
        texts.iter().any(|t| t.starts_with("[Request interrupted by user"))
    })
}

/// Whether claude sent from its queue past byte `from`: a `dequeue`, a
/// `remove` that handed the message to the turn running, or a prompt that
/// came from the queue (measured on 2.1.290).
pub(crate) fn sent_from_queue_since(path: &Path, from: u64) -> bool {
    since(path, from).iter().any(|record| match text_of(record, "type").as_deref() {
        Some("queue-operation") => match text_of(record, "operation").as_deref() {
            Some("dequeue") => true,
            Some("remove") => matches!(text_of(record, "reason").as_deref(), Some("absorbed_mid_turn" | "delivered_to_agent")),
            _ => false,
        },
        Some("user") => text_of(record, "promptSource").as_deref() == Some("queued"),
        _ => false,
    })
}

/// Whether a person's message waits in claude's queue, as the transcript's
/// last stretch says: each `enqueue` until a `dequeue` takes the oldest, a
/// `remove` takes it by its text, or a `popAll` takes them all back to the
/// box. What claude queues for itself (a background task's notice) isn't a
/// person's, and draws no Queued row (`machine_queued`).
pub(crate) fn waiting_in_queue(path: &Path) -> bool {
    let end = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut queue: std::collections::VecDeque<String> = Default::default();
    for record in mid_turn::records_between(path, end.saturating_sub(mid_turn::LONGEST_READ), end) {
        if text_of(&record, "type").as_deref() != Some("queue-operation") {
            continue;
        }
        let content = text_of(&record, "content").unwrap_or_default();
        match text_of(&record, "operation").as_deref() {
            Some("enqueue") if !content.trim().is_empty() => queue.push_back(content),
            Some("dequeue") => {
                queue.pop_front();
            }
            Some("remove") => {
                if let Some(n) = queue.iter().position(|q| *q == content) {
                    queue.remove(n);
                }
            }
            Some("popAll") => queue.clear(),
            _ => {}
        }
    }
    queue.iter().any(|q| !farcooler_core::session_log::projector::fold::machine_queued(q))
}
