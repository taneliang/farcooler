//! Terminal mutation replies and watcher-derived activity share one conversion.

use super::{DomainError, Rpc, Result, Uuid, result, wire};

impl Rpc {
    /// Re-read a terminal so the reply carries its DERIVED state rather than
    /// the intent that was just written.
    pub(super) async fn terminal_result(&self, id: Uuid) -> Result<result::Value> {
        for view in self.service.fleet().await? {
            if let Some(t) = view.terminals.iter().find(|t| t.terminal.id == id) {
                let person = self.watcher.viewer_for([id], || self.caller());
                return Ok(result::Value::Terminal(self.with_activity(t, crate::claude_login_wire::Logins::One, &person).await));
            }
        }
        Err(DomainError::NotFound)
    }

    /// Attach what the watcher decided the agent is doing.
    ///
    /// One place, so a terminal in a list and the same terminal in a mutation
    /// reply cannot disagree about whether its agent is waiting for you.
    pub(super) async fn with_activity(
        &self,
        view: &crate::service::TerminalView, logins: crate::claude_login_wire::Logins<'_>, person: &crate::people::Person,
    ) -> farcooler_protocol::v1::Terminal {
        let mut message = wire::terminal_with_agent_state(view, self.service.agents());
        // The runner's answers, as `announce` makes them, so a list and an event agree.
        crate::task_link::stamp_notice_task(&self.service.store, &view.terminal, &mut message);
        self.service.stamp_claude_login(logins, &view.terminal, &mut message);
        // As `person` reads it: a member's own look ends their `Done` (ov-740).
        let (activity, state_since, turn_started_at) = self.watcher.activity_for(person, view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = state_since.map(wire::timestamp);
        message.turn_started_at = turn_started_at.map(wire::timestamp);
        message.blocked_question = self.watcher.blocked_question(view.terminal.id).await;
        if let Some(command) = self.watcher.command(view.terminal.id).await {
            message.current_command = command;
        }
        message.ports = self.watcher.ports(view.terminal.id).await;
        message.running_agent = self.watcher.running_agent(view.terminal.id).await;
        message.draft_hold = self.watcher.draft_hold(view.terminal.id);
        // The same lines the broadcast path sends, off the same `Observed`. A
        // client that reads a list and then watches events must not see the
        // feed appear, vanish, and come back.
        message.feed = self.watcher.feed(view.terminal.id).await;
        // The same message those lines were cut from, cut from its opening
        // instead, off the same `Observed` for the same reason: this is what
        // both apps' notifications quote, and a client that read it from a
        // list and then watched events must not see the sentence a banner is
        // about appear only on one of the two paths.
        message.said = self.watcher.said(view.terminal.id).await;
        // Off the same `Observed` for the same reason the feed is: a client
        // that lists terminals and then watches events must not be told a turn
        // failed by one path and that it finished cleanly by the other.
        message.turn_failed = self.watcher.turn_failed(view.terminal.id).await;
        // The agents this one spawned and has not finished with, off the same
        // `Observed` for the same reason again: a client that lists terminals
        // and then watches events must not see two subagents in the list and
        // none in the push.
        self.watcher.stamp_subagents(&mut message, view.terminal.id).await;
        // The thirteen buckets, off the watcher's ring for the same reason as
        // everything above it: the ring is the only copy, and a client that
        // lists terminals and then watches events must not be handed two
        // different histories of one pane. See `Watcher::trace`.
        self.watcher.stamp_trace(&mut message, view.terminal.id);
        // The compact ladder, computed from everything just set above — see
        // `wire::apply_rungs` for why it has to run last, and why the signal
        // line is handed to it rather than read off the message.
        // The line and the counts behind its task-list rung go in together, so
        // a list reply cannot state a position in prose and a different one in
        // numbers. Two reads of the watcher's state, a moment apart, is the
        // most this path can do — the alternative is a lock held across the
        // whole conversion — and handing both to one function is what keeps
        // the pair from also being set in two places. See `wire::apply_rungs`.
        wire::apply_rungs(
            &mut message,
            self.watcher.signal(view.terminal.id).await.as_deref(),
            self.watcher.plan(view.terminal.id).await,
        );
        message
    }
}
