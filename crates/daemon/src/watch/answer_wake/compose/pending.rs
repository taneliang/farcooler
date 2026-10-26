//! A composed message pasted into claude's box whose Enter waits out a turn
//! claude began after the paste, with no fence to be had yet (ov-808).
//!
//! **Off the lock.** The send is answered Queued at once and the Enter left
//! to a task of its own, which takes the pane's typing lock only for each
//! try (`try_typing`, never waiting on it) and lets it go between tries. So
//! Stop, Bring Here and a phone's keys, which queue behind a pane's call on
//! its connection and behind the typing lock on the runner, never wait on a
//! turn (ov-808 review, S1). The pane's rows carry the message as Queued
//! while it waits, as a held message's do (`held`).
//!
//! **Each try**, every `PASTE_POLL` until `turn_begun_wait`: the read-back
//! of `still_between` again (`between_or_why`), and then
//! - a turn still running: the fenced Enter once the hooks give a witness;
//! - between turns twice running, `IDLE_TWICE` apart, with nothing read
//!   between but that: the plain Enter. A turn has been seen, and one stray
//!   frame read Idle mid-turn must not send a key with no fence (S2);
//! - anything else (a key typed, the box changed, the agent gone): left.
//!
//! **Given up.** A Stop or Send Now pressed since the message was queued
//! leaves it, as the person stopped the turn it waited on; so does the wait
//! running out. Either is logged as a paste left in the box, with its check,
//! and the pane's rows say it wasn't sent.
//!
//! **Not kept.** The wait lives in memory: a runner restarted during it
//! leaves the text typed in the box, unlogged, and the next send's gate
//! reads it as a draft and holds (N3).

use std::time::{Duration, Instant};

use farcooler_store::models::Terminal;
use uuid::Uuid;

use super::super::{PASTE_POLL, Proven};
use super::{Confirm, Expected, Left, held, left_in_box};
use crate::runtime::Runtime;
use crate::watch::{Watcher, now_millis};

/// How far apart two reads between turns must be, with only such reads
/// between them, before a waiting message's plain Enter (S2).
pub(crate) const IDLE_TWICE: Duration = Duration::from_millis(300);

/// A message in claude's box, its Enter waiting out a turn.
pub(super) struct Pending {
    pub(super) to: Terminal,
    pub(super) proven: Proven,
    /// When the paste began (`last_input`'s clock).
    pub(super) started: i64,
    pub(super) expected: Expected,
    pub(super) confirm: Confirm,
    /// The message as the pane's rows show it.
    pub(super) echo: String,
    /// The check that found the turn.
    pub(super) left: Left,
}

/// The message's words as a view's echo draws them: each image as
/// `[Image]`, then the text (`held`'s echo).
pub(super) fn echo(images: usize, raw: &str) -> String {
    let mut parts = vec!["[Image]".to_string(); images];
    if !raw.trim().is_empty() {
        parts.push(raw.trim().to_string());
    }
    parts.join(" ")
}

impl Watcher {
    /// Show `pending` on its pane's rows as Queued, and wait out its turn on
    /// a task of its own.
    pub(super) fn pend_enter(&self, pending: Pending) {
        let row = format!("pending:{}", Uuid::now_v7());
        let terminal = pending.to.id;
        crate::session_projectors::global().runner_hold(terminal, |p| p.runner_held(&row, &pending.echo, now_millis()));
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.insert(terminal);
        if let Some(me) = self.me.upgrade() {
            tokio::spawn(async move {
                let entered = me.enter_when_it_can(&pending).await;
                me.pended(&pending, &row, entered).await;
                me.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.remove(&terminal);
            });
        }
    }

    /// Whether a message pasted into `terminal` waits for its Enter: a send
    /// meanwhile is held behind it (`compose_or_hold`).
    pub(super) fn enter_pending(&self, terminal: Uuid) -> bool {
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.contains(&terminal)
    }

    /// Try `p`'s Enter until it goes or must be given up (this module's
    /// docs): the word it's given up with.
    async fn enter_when_it_can(&self, p: &Pending) -> Result<(), &'static str> {
        let queued = Instant::now();
        let deadline = tokio::time::Instant::now() + self.timing().turn_begun_wait;
        let registry = self.service.registry();
        let mut idle_since: Option<Instant> = None;
        let mut last = Left { check: p.left.check, screen: p.left.screen.clone(), turn: true };
        loop {
            tokio::time::sleep(PASTE_POLL).await;
            if self.key_pressed_since(p.to.id, queued) {
                left_in_box(&registry, &p.to, "stopped", p.proven.background, None);
                return Err("stopped");
            }
            if tokio::time::Instant::now() >= deadline {
                last.logged(&registry, &p.to, &p.proven);
                return Err("paste_left");
            }
            // Never wait on the box: Stop and Bring Here take it.
            let Some(_typing) = self.try_typing(p.to.id) else {
                idle_since = None;
                continue;
            };
            let read = self.between_or_why(&p.to, &p.proven, p.started, &p.expected, false).await;
            match read {
                Err(_) => return Err("not_running"),
                Ok(None) => {
                    let first = *idle_since.get_or_insert_with(Instant::now);
                    if first.elapsed() >= IDLE_TWICE {
                        let runtime = Runtime { marks: None, ..self.service.runtime() };
                        return runtime.send_bytes_hex(p.to.id, "0d").await.map_err(|_| "failed");
                    }
                }
                Ok(Some(left)) if !left.turn => {
                    left.logged(&registry, &p.to, &p.proven);
                    return Err("paste_left");
                }
                Ok(Some(left)) => {
                    idle_since = None;
                    if let Some(witness) = self.witness(&p.proven, &p.to, &p.confirm.submitted).await {
                        return self.enter_fenced_for(&p.to, &p.proven, p.started, &witness, &p.expected).await;
                    }
                    last = left;
                }
            }
        }
    }

    /// Whether Stop or Send Now was pressed in `terminal` since `since`.
    fn key_pressed_since(&self, terminal: Uuid, since: Instant) -> bool {
        self.keys_pressed.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).is_some_and(|at| *at >= since)
    }

    /// What became of `p`'s Enter, on the pane's rows: taken back once claude
    /// says it took it, its own rows showing it; else why it wasn't sent.
    async fn pended(&self, p: &Pending, row: &str, entered: Result<(), &'static str>) {
        let agent = held::agent_name(&p.to.command_preset);
        let why = match entered {
            Ok(()) => {
                self.mark_told(p.to.id);
                let confirm = Confirm { since: Instant::now(), ..p.confirm.clone() };
                let took = self.took(&confirm).await;
                crate::session_projectors::global().runner_hold(p.to.id, |r| r.runner_told(row));
                if took.is_some() {
                    return;
                }
                held::why_not_sent(agent, &p.echo, "unconfirmed")
            }
            Err(what) => held::why_not_sent(agent, &p.echo, what),
        };
        crate::session_projectors::global().runner_hold(p.to.id, |r| r.runner_dropped(row, &why, now_millis()));
    }
}
