//! One terminal as the phones read it in the fleet (`Session::fleet`).
//! Its own file so a test can feed it a wire `Terminal` (ov-443).

use super::*;

/// `t` projected for a phone, its draft hold included.
pub(super) fn terminal_json(t: &farcooler_protocol::v1::Terminal) -> serde_json::Value {
    with_draft_hold(t, json!({
            "id": uuid_of(&t.id).to_string(),
            "short": short(&t.id),
            "title": t.title,
            "preset": if t.current_command.is_empty() { t.command_preset.clone() } else { t.current_command.clone() },
            // What was launched, and the agent running now: what
            // a phone offers the conversation view by, never
            // `preset`, which is claude's session title (ov-443).
            "program": t.command_preset,
            "runningAgent": t.running_agent,
            "state": terminal_label(t.state()),
            "activity": activity_label(t.activity),
                    "activitySince": activity_since(t),
            // How it ENDED, which is the difference between a
            // shell you closed and a build that broke.
            "exitCode": t.exit_status.as_ref().and_then(|e| e.code),
            "exitSignal": t.exit_status.as_ref().and_then(|e| e.signal),
            "turnStartedAt": turn_started_at(t),
            "blockedQuestion": t.blocked_question.clone(),
            // The last three things the agent did, already
            // redacted and cut to a row's width by the daemon.
            // The phone is the client this matters most to:
            // it is the one with no screen to scroll back
            // through and the smallest row to say it in.
            "feed": t.feed.clone(),
            // The last of those messages WHOLE and from its
            // start, which is what the phone's `Notifier` puts
            // in a banner. It cannot be recovered from the
            // lines above — they are wrapped rows, so the last
            // of them is the end of the window rather than the
            // beginning of a sentence, which is how a lock
            // screen came to read `batches to avoid N+1
            // shits.` See `farcooler_core::feed::Feed::said`.
            "said": t.said.clone(),
            // The agents it spawned and has not finished with,
            // named, on the same terms. A phone shows these
            // under the row exactly as the Mac does, and their
            // COUNT is already inside `line` for the surfaces
            // with room for only one string.
            "subagents": t.subagents.clone(),
            // The compact ladder, decided on the host. This is
            // the projection a Live Activity will be built
            // from — a lock screen, an Island, a watch face —
            // and each of those has room for a different rung,
            // so all four travel together rather than the
            // phone re-deriving the narrow ones from the wide
            // one and disagreeing with the Mac about the same
            // pane.
            "glyph": t.glyph.clone(),
            "headline": t.headline.clone(),
            "line": t.line.clone(),
            "rank": t.rank,
            // How far the agent is through its OWN task list,
            // as the two numbers `line` may have composed into
            // `3/7`. Carried separately because the phone must
            // not read them back out of that string: `line` is
            // a rung, so a blocked agent's is the question and
            // holds no numbers at all, and scraping it would be
            // a second derivation of a fact the host derives
            // once. Absent — not zero — when the host has
            // nothing to say, which is a pane with no list and
            // every codex and cursor pane; see the fields'
            // comments in `proto/farcooler.proto`.
            "planDone": t.plan_done,
            "planTotal": t.plan_total,
            // Thirteen buckets of what this pane has been
            // doing, base64 because JSON has no bytes. Passed
            // straight across without being unpacked: the
            // widget holds a whole snapshot per timeline entry
            // across thirteen entries in a memory-capped
            // extension, and decoding 66 bytes into three
            // arrays per agent is the cost the bytes encoding
            // exists to avoid. `farcooler_core::trace`
            // documents the layout.
            //
            // ABSENT, not an empty string, when the pane has
            // done nothing the trace can see — a flat zero row
            // and "no history" are different claims and only
            // one of them is true of a pane nobody has used.
            "activityTrace": (!t.activity_trace.is_empty())
                .then(|| farcooler_core::base64::encode(&t.activity_trace)),
            // How the last turn ended, which `activity` cannot
            // say: a turn that died reads as `done` there. The
            // rungs above already carry it, and this is what
            // lets a phone draw its own indicator without
            // having to parse one of them back apart.
            "turnFailed": t.turn_failed,
            "epoch": t.epoch,
            "paneMode": pane_mode_label(t.pane_mode),
            // Without this the phone's terminal/chat switch
            // could never appear: `canSwitchPaneMode` reads it,
            // and a field the runner never sends is a capability
            // the client always denies.
            "chatCapable": t.chat_capable,
            "agentSessionId": t.agent_session_id.clone(),
            "agentMode": t.agent_mode.clone(),
            "availableAgentModes": t.available_agent_modes.clone(),
            // Same reason as `chatCapable` above: a field the
            // runner never sends is a state the phone can
            // never draw, so a chat whose agent refused to
            // start would spin on the phone forever.
            "agentFailure": t.agent_failure.clone(),
            // The board task this pane was opened for, which
            // is what lets a phone's board go from a card to
            // the agent working it. The CLI's two terminal
            // projections carry the same key for the Mac; see
            // `task_of` for why a missing or malformed id is
            // absent rather than the nil uuid.
            "taskId": task_of(t),
    }))
}

#[cfg(test)]
mod tests {
    use farcooler_protocol::v1::Terminal;

    /// A claude typed into a shell that named its session: `preset` is the
    /// title, so a phone that offered the view by it never offered it. Goes
    /// red without `runningAgent` and `program`.
    #[test]
    fn a_phone_is_told_the_agent_running_and_what_was_launched() {
        let t = Terminal {
            command_preset: "shell".into(),
            current_command: "Fix the login bug".into(),
            running_agent: Some("claude".into()),
            ..Default::default()
        };
        let json = super::terminal_json(&t);
        assert_eq!(json["preset"], "Fix the login bug");
        assert_eq!(json["program"], "shell");
        assert_eq!(json["runningAgent"], "claude");
        assert!(super::terminal_json(&Terminal::default())["runningAgent"].is_null());
    }
}
