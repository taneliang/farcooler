//! A person's plain text by a harness's own queue, where typing would be
//! refused (ov-627, the owner's decision for codex, the coordinator's for
//! cursor on the same evidence). R-78 holds: a person's words reach the model
//! as their own role:user turn. Typing does that, and, measured, so do these:
//!
//! - **codex** (codex-cli 0.153.4): `codex queue` gives the model the same
//!   plain role:user `input_text` as typing, no wrapper, and the rollout the
//!   same `UserMessage`. So a person's plain text (no image, no slash command)
//!   goes by the queue when compose would refuse it as `busy` (mid-turn) or
//!   `draft` (a draft in the box); otherwise it's typed. Images and slash
//!   commands are always typed (the queue refuses images and sends a command
//!   as text). Live (`queue_prompt`) and queued (`queue_prompt_wake`) alike.
//! - **cursor** (cursor-agent 2026.09.26): the stop hook's `followup_message`
//!   is the same role:user `<user_query>` as typing. So a person's message to
//!   a working cursor waits for the turn's end and goes by the followup
//!   (`followup`), saying so (`Held::Followup`). An idle cursor, and every
//!   answer, is typed, as on main (`cursor_waits`). cursor stops calling the
//!   hook after about 5 followups in a row with nothing typed; past that, a
//!   person's message is held and says why (`Held::FollowupLimit`), never
//!   dropped.
//!
//! **Shown as queued.** A message the codex queue took shows on the pane's
//! rows as Queued, as a held compose does (`Projection::runner_held`), until
//! the rollout records it (`watch_queued`). A message waiting for cursor's
//! followup says so on its card (`say_waiting`).

use std::path::PathBuf;
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::{PendingWake, WakeKind};
use farcooler_store::models::{Task, Terminal};
use uuid::Uuid;

use super::compose::codex::{Sent, records};
use super::route::Delivered;
use super::{Held, Pass, Turn, codex_turn, told};
use crate::runtime::last_input;
use crate::watch::{Watcher, now_millis};

/// How long a queued message is watched for in codex's rollout.
pub(crate) const WATCH_FOR: Duration = Duration::from_secs(4 * 60 * 60);
/// How often the rollout is read, and its process asked again which one it
/// holds.
const WATCH_EVERY: Duration = Duration::from_secs(2);
const REJOIN_EVERY: u32 = 15;

/// About how many followups in a row cursor takes, with nothing typed, before
/// it stops calling its stop hook (measured, ov-627).
pub(crate) const FOLLOWUP_LIMIT: u32 = 5;

/// Where a message `codex queue` took will show as run.
pub(crate) struct Queued {
    pub(crate) pid: i32,
    pub(crate) rollout: PathBuf,
    pub(crate) from: u64,
    pub(crate) at: i64,
}

/// Each cursor pane's followups in a row: how many, and when the last went.
static FOLLOWUPS: std::sync::Mutex<Vec<(Uuid, u32, i64)>> = std::sync::Mutex::new(Vec::new());

/// Whether `text` is plain: something to send, and no slash command.
fn plain(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && !text.starts_with('/')
}

impl Watcher {
    /// A person's live plain `text` for `pane`, which compose refused as
    /// `busy` or `draft`: by the codex queue when `pane` runs codex, shown as
    /// Queued. `None` when it isn't codex, isn't plain, or the queue didn't
    /// take it: the refusal stands.
    pub(crate) async fn queue_prompt(&self, pane: Uuid, text: &str) -> Option<Result<Delivered>> {
        let text = super::compose::normalized(text);
        if !plain(&text) {
            return None;
        }
        let to = self.service.store.get_terminal(pane).ok()?;
        if !super::deliver::codex_queue_on(to.id) {
            return None;
        }
        let queued = self.run_codex_queue(&to, &text).await?;
        self.mark_told(to.id);
        self.watch_queued(to.id, Uuid::now_v7(), &text, queued, None);
        Some(Ok(Delivered::Queued))
    }

    /// A person's queued plain `text` (an answer, a message) that typing into
    /// codex `to` held as busy or a draft: by the codex queue, settled as told
    /// and shown as Queued. `None` when the queue didn't take it: it waits.
    pub(super) async fn queue_prompt_wake(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Option<Pass> {
        if wake.claimed_at.is_some() || !plain(text) || !super::deliver::codex_queue_on(to.id) {
            return None;
        }
        let queued = self.run_codex_queue(to, text).await?;
        self.mark_told(to.id);
        self.watch_queued(to.id, wake.note, text, queued, Some(task));
        Some(self.settle(wake, Some(task), told(wake.kind, to, Turn::During)))
    }

    /// Show `text` on `pane`'s rows as Queued, as `id`, until codex's rollout
    /// records it run, or `WATCH_FOR` passes; then the row is taken back, and
    /// codex's own shows it. One never seen run is withdrawn with a notice,
    /// and when it was told for `task` (a queued answer or message, whose
    /// card says it was told), the card says so too, in the same words.
    pub(super) fn watch_queued(&self, pane: Uuid, id: Uuid, text: &str, queued: Queued, task: Option<&Task>) {
        let (row, text) = (id.to_string(), text.to_string());
        let (me, task, watch) = (self.me.clone(), task.cloned(), self.timing().queue_watch);
        crate::session_projectors::global().runner_hold(pane, |p| p.runner_held(&row, &text, queued.at));
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        runtime.spawn(async move {
            let sent = Sent { text, images: 0, before: Some(queued.rollout.clone()), from: queued.from, entered: queued.at };
            let deadline = tokio::time::Instant::now() + watch;
            let mut now: Option<PathBuf> = None;
            let mut polls = 0u32;
            let mut ran = false;
            while tokio::time::Instant::now() < deadline {
                if records(&queued.rollout, queued.from, &sent)
                    || now.as_deref().filter(|p| *p != queued.rollout).is_some_and(|p| records(p, 0, &sent))
                {
                    ran = true;
                    break;
                }
                if polls % REJOIN_EVERY == REJOIN_EVERY - 1 {
                    now = codex_turn::rollout_now(queued.pid).await;
                }
                polls += 1;
                tokio::time::sleep(WATCH_EVERY).await;
            }
            // Never seen run (codex moved threads, or didn't pick it up): said
            // so, as a held message given up is, never shown as told.
            let why = (!ran).then(|| super::compose::held::why_not_sent("Codex", &sent.text, "queued_unseen"));
            crate::session_projectors::global().runner_hold(pane, |p| match &why {
                None => p.runner_told(&row),
                Some(why) => p.runner_dropped(&row, why, now_millis()),
            });
            if let (Some(why), Some(task), Some(me)) = (why, task, me.upgrade()) {
                match me.service.store.add_note(task.id, farcooler_store::models::NoteKind::Progress, farcooler_store::models::Actor::Runner, &why, serde_json::json!({})) {
                    Ok(_) => me.announce_task_changed(&task, None, farcooler_store::models::Actor::Runner),
                    Err(e) => tracing::warn!(terminal = %pane, error = %e, "couldn't say a queued message never ran"),
                }
            }
        });
    }

    /// Why a person's message to the cursor pane `to` waits, rather than be
    /// typed: for the followup at its working turn's end, or held past
    /// cursor's loop limit. `None` for an idle pane and for any answer, which
    /// are typed, as on main: the followup fires only at a turn's end, and an
    /// idle cursor, one waiting on an answer above all, may never end another
    /// (the coordinator's ruling on ov-620 re-review 4). Typing into cursor
    /// 2026.09.26 is fixed by ov-632; until it lands, a typed one is left in
    /// the box and said so (`paste_left`), as on main.
    pub(super) async fn cursor_waits(&self, wake: &PendingWake, to: &Terminal) -> Option<Held> {
        if wake.kind != WakeKind::Message || super::deliver::Way::of(&to.command_preset, to.id) != super::deliver::Way::CursorFollowup {
            return None;
        }
        if self.activity(to.id).await.0 != AgentActivity::Working {
            return None;
        }
        Some(if self.followup_limited(to.id) { Held::FollowupLimit } else { Held::Followup })
    }

    /// A followup went to `terminal`'s cursor: one more in a row.
    pub(super) fn followup_taken(&self, terminal: Uuid) {
        let typed = last_input(self.service.root_dir(), terminal);
        let mut all = FOLLOWUPS.lock().unwrap_or_else(|e| e.into_inner());
        match all.iter_mut().find(|(id, ..)| *id == terminal) {
            // Typed since the last: the row starts again.
            Some(row) if typed.is_some_and(|at| at > row.2) => *row = (terminal, 1, now_millis()),
            Some(row) => *row = (terminal, row.1 + 1, now_millis()),
            None => all.push((terminal, 1, now_millis())),
        }
    }

    /// Whether `terminal`'s cursor has taken `FOLLOWUP_LIMIT` followups in a
    /// row with nothing typed since, so it calls its hook no more. Counted in
    /// memory: after a restart the count starts again at 0, while cursor may
    /// already have stopped calling its hook, so a card can say Followup
    /// where the limit is true until a person types in the pane.
    pub(super) fn followup_limited(&self, terminal: Uuid) -> bool {
        let typed = last_input(self.service.root_dir(), terminal);
        let all = FOLLOWUPS.lock().unwrap_or_else(|e| e.into_inner());
        all.iter().any(|&(id, n, last)| id == terminal && n >= FOLLOWUP_LIMIT && typed.is_none_or(|at| at <= last))
    }
}

/// The refusals of a person's plain text that the codex queue may take
/// instead.
pub(crate) fn queueable(e: &DomainError) -> bool {
    matches!(e, DomainError::Conflict { what: "busy" | "draft" })
}
