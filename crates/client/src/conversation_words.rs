//! Every word an agent conversation says (ov-1049; ruling R-112). AgentKit
//! (`AgentConversation`, `ConversationItems`), Android (`AgentConversation.kt`)
//! and the web (`chat-copy.ts`) each wrote them; this is the one copy. An app
//! asks once per agent, through the plan view's bridge, for a table: what the
//! composer says for each word the runner refuses a send, a Stop or an answer
//! with, and the words a row says. A count, a time or a name is a `{placeholder}`
//! the app fills, since the app holds the row.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::plan_view::words::Style;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub style: Style,
    /// "Claude" or "Codex", as the conversation names its agent.
    pub agent: String,
}

/// What the composer does with a refusal: show the Handoff row (`handoff`), the
/// panel row (`panel`), the terminal-draft row (`draft`), say `text` (`said`),
/// or nothing (`quiet`).
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub kind: &'static str,
    pub text: Option<String>,
}

fn said(text: impl Into<String>) -> Issue {
    Issue { kind: "said", text: Some(text.into()) }
}

fn row(kind: &'static str) -> Issue {
    Issue { kind, text: None }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Words {
    /// By `terminal.compose`'s refusal word; `images:command` is `images` for a slash command.
    pub send: BTreeMap<&'static str, Issue>,
    /// By `terminal.interrupt`'s refusal word, for Stop and for Send Now.
    pub stop: BTreeMap<&'static str, Issue>,
    pub send_now: BTreeMap<&'static str, Issue>,
    /// By `terminal.agent_answer`'s refusal word.
    pub answer: BTreeMap<&'static str, String>,
    /// Everything else, by name.
    pub phrases: BTreeMap<&'static str, String>,
    /// A run of tool calls' line: the first group naming every call's tool, the last naming none.
    pub groups: Vec<Group>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Group {
    pub names: Vec<&'static str>,
    pub running: String,
    pub done: String,
}

pub const LONGEST: u32 = 500;
pub const MOST_IMAGES: u32 = 10;

pub fn words(style: Style, agent: &str) -> Words {
    let send_now = style.label("send now");
    let send = BTreeMap::from([
        ("prompt", row("handoff")),
        ("dialog", row("handoff")),
        ("handoff", row("panel")),
        ("draft", row("draft")),
        ("typing", said("Someone typed in the terminal in the last 3 seconds, so the message wasn’t sent. Try again once they stop.")),
        ("busy", said(format!("{agent} takes messages only between turns, so this one wasn’t sent. Send it again when the turn ends."))),
        ("too_long", said(format!("That message is over {LONGEST} characters. Shorten it, or paste it in the terminal."))),
        ("command", said(format!("{agent} would run that as a shell command or doesn’t have that command, so it wasn’t sent. Use the terminal for it."))),
        ("paste_left", said("The message didn’t land in the box as typed, so it was left there and not sent.")),
        ("left_at_shell", said(format!("{agent} quit as the message was typed. It wasn’t run."))),
        ("unconfirmed", said(format!("{agent} didn’t confirm it took the message. Check the terminal before sending it again."))),
        ("not_running", said(format!("{agent} isn’t running in this pane."))),
        ("not_an_agent", said(format!("{agent} isn’t running in this pane."))),
        ("unfamiliar", said("Far Cooler can’t read this terminal’s box, so nothing was typed.")),
        ("unproven", said("Far Cooler can’t read this terminal’s box, so nothing was typed.")),
        ("unconfirmable", said(format!("Far Cooler can’t find {agent}’s session to confirm a send, so nothing was typed."))),
        ("unsupported", said(format!("{agent} can’t take a message from here. Use the terminal."))),
        ("picker", said(format!("{agent} would open a picker for a last word that starts with @ or $, so the message wasn’t sent. Add a word after it, or use the terminal."))),
        ("too_tall", said(format!("That message is too tall for {agent}’s box to show whole, so it wasn’t sent. Shorten it, or paste it in the terminal."))),
        ("images_too_large", said("These images are too large to send together. Send fewer or smaller ones.")),
        ("images", said(format!("A message takes at most {MOST_IMAGES} images."))),
        ("images:command", said("A slash command can’t carry images. Send it without them.")),
        ("image_too_large", said("That image is too large to send. Use a smaller one.")),
        ("backslash", said("Claude reads a backslash at the end as a new line, so the message wasn’t sent. Remove it, or add a word after it.")),
        ("image", said("One of the images couldn’t be read, so nothing was sent.")),
        ("files", said(format!("A message takes at most {MOST_IMAGES} files."))),
        ("files:command", said("A slash command can’t carry files. Send it without them.")),
        ("file", said("One of the files didn’t reach the runner, so nothing was sent.")),
        ("file_too_large", said("That file is too large to send. Files up to 16 MB work.")),
        ("files_too_large", said("These files are too large to send together. Send fewer or smaller ones.")),
        // An agent's words (ov-620): the runner read this send as an agent's.
        ("typing_off", said("This workspace doesn’t let Far Cooler type into its agents’ panes. Turn on waking agents in its settings.")),
        ("in_pane", said("Far Cooler couldn’t tell whether a person or an agent sent this, so it wasn’t sent. Send it from the app.")),
        ("word:scope-denied", said("This device can’t send messages to this runner.")),
        ("", said("The message wasn’t sent.")),
    ]);
    let key = |stop: bool| {
        let pick = |a: &str, b: String| said(if stop { a.to_string() } else { b });
        BTreeMap::from([
            ("idle", row("quiet")),
            ("too_soon", row("quiet")),
            ("prompt", row("handoff")),
            ("draft", row("draft")),
            ("typing", said("Someone is typing in the terminal. Try again in a moment.")),
            ("sending", said("A message is still going in. Try again in a moment.")),
            ("nothing_queued", said("Nothing is waiting in Claude’s queue.")),
            ("settling", pick("Claude is starting a step. Try Stop again in a moment.", format!("Claude is starting a step. Try {send_now} again in a moment."))),
            ("unconfirmed", pick("Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again.", "Claude didn’t confirm it sent the queued messages. Check the terminal.".into())),
            ("word:scope-denied", said("This device can’t control this runner.")),
            ("", pick("Far Cooler can’t stop Claude safely from here. Use the terminal.", "Far Cooler can’t send the queue safely from here. Use the terminal.".into())),
        ])
    };
    let answer = BTreeMap::from([
        ("not_held", "This isn’t waiting here anymore. It was answered, or only the terminal can answer it now.".to_string()),
        ("not_delivered", format!("The answer didn’t reach {agent}. Answer in the terminal.")),
        ("answers", "Answer every question first.".into()),
        ("timed_out", "The runner didn’t answer in time. Check the terminal before answering again.".into()),
        ("", "The answer wasn’t sent. Answer in the terminal.".into()),
    ]);
    let phrases = [
        // The composer.
        ("may_have_been_sent", "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again.".to_string()),
        ("not_connected", "The runner isn’t connected, so the message wasn’t sent.".into()),
        ("key_timed_out", "The runner didn’t answer in time. Check the terminal.".into()),
        ("key_not_connected", "The runner isn’t connected. Use the terminal.".into()),
        ("too_long_composed", "That message is over 100,000 characters. Shorten it, or paste it in the terminal.".into()),
        ("command", "A message can’t start with a symbol Claude reads as a command, such as / or !. Use the terminal for commands.".into()),
        ("draft_in_terminal", "The terminal’s box already holds a draft. Send or clear it there first.".into()),
        ("draft_in_terminal_bring", "The terminal’s box already holds a draft of its own.".into()),
        ("handoff", format!("{agent} is showing something only the terminal can.")),
        ("panel", format!("This opens a panel in {agent}, so it’s for the terminal.")),
        // A turn's ending.
        ("outcome_done", "Done".into()),
        ("outcome_took", "Took {time}".into()),
        ("outcome_interrupted", "Interrupted".into()),
        ("outcome_unrecorded", "Not recorded".into()),
        ("outcome_failed", "Failed".into()),
        ("outcome_failed_detail", "Failed: {detail}".into()),
        ("outcome_ended", "Ended".into()),
        ("origin_queued", "Queued during the last turn".into()),
        ("scheduled_task", "Scheduled task".into()),
        // A queued message, by its state.
        ("queued_Withdrawn", "Withdrawn".into()),
        ("queued_InBox", "Waiting to send".into()),
        ("queued_Sent", "Sent mid-turn".into()),
        ("queued", "Queued".into()),
        // An ask's heading.
        ("ask_Permission", format!("{agent} is asking for permission")),
        ("ask_PlanExit", format!("{agent} has a plan for you to review")),
        ("ask", format!("{agent} is asking a question")),
        ("ask_answered", "Answered".into()),
        ("ask_answered_on", "Answered on {by}".into()),
        // Rows.
        ("tools_one", "1 tool".into()),
        ("tools", "{n} tools".into()),
        ("tasks_progress", "{done} of {n} done".into()),
        ("task_Completed", "Done".into()),
        ("task_InProgress", "In progress".into()),
        ("task", "Not started".into()),
        ("harness_Report", "Report from {name}".into()),
        ("harness_Message", "Message from {name}".into()),
        ("harness", "System note".into()),
        ("harness_a_subagent", "a subagent".into()),
        ("harness_agent", "agent {id}".into()),
        ("gap_one", "A line of this session couldn’t be read.".into()),
        ("gap", "Some of this session couldn’t be read.".into()),
    ];
    let group = |names: &[&'static str], running: &str, done: &str| Group { names: names.to_vec(), running: running.into(), done: done.into() };
    let groups = vec![
        group(&["Bash", "bash"], "Running {n} commands", "Ran {n} commands"),
        group(&["Read"], "Reading {n} files", "Read {n} files"),
        group(&["Edit", "MultiEdit", "Write", "NotebookEdit"], "Editing {n} files", "Edited {n} files"),
        group(&["Grep", "Glob"], "Running {n} searches", "Ran {n} searches"),
        group(&[], "Using {n} tools", "Used {n} tools"),
    ];
    Words { send, stop: key(true), send_now: key(false), answer, phrases: BTreeMap::from(phrases), groups }
}

/// What an app asks of one conversation item (ov-1054): any of the three,
/// `{"agent_type":"general-purpose"}`, `{"ms":65000}`, `{"origin":"System"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRequest {
    pub agent_type: Option<String>,
    pub ms: Option<i64>,
    pub origin: Option<String>,
}

/// The answers under the names they were asked by, each present only when asked:
/// `agent_type` and `ms` come back as words ("General purpose", "1:05"), `origin` as whether it is a notice.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub agent_type: Option<String>,
    pub ms: Option<String>,
    pub origin: Option<bool>,
}

pub fn item(asked: &ItemRequest) -> Item {
    Item {
        agent_type: asked.agent_type.as_deref().map(agent_type),
        ms: asked.ms.map(short),
        origin: asked.origin.as_deref().map(is_notice),
    }
}

/// A subagent's type as words: `general-purpose` reads "General purpose".
pub fn agent_type(raw: &str) -> String {
    let words = raw.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(|| "Agent".into(), |first| first.to_uppercase().chain(chars).collect())
}

/// "0:04", "1:12", "2:03:09": the format a running timer counts in, so a
/// finished time reads like the running one did.
pub fn short(ms: i64) -> String {
    let seconds = (ms / 1000).max(0);
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A turn nobody typed: a background task finishing, or claude waking itself
/// (`TurnOrigin::Notification`, `System`). Drawn as a notice, never as the
/// person's message.
pub fn is_notice(origin: &str) -> bool {
    matches!(origin, "Notification" | "System")
}

#[cfg(test)]
#[path = "conversation_words/tests.rs"]
mod tests;
