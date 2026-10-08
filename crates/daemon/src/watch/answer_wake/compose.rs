//! `terminal.compose` (ov-367): what a person writes in a native view's
//! composer, typed into claude's own box in a terminal pane and submitted,
//! with its line breaks, its images and its slash command, and answered
//! Sent or Queued only once claude has said it took it.
//!
//! **The gate.** The answer wake's, against the pane as it is now, every
//! check failing closed with nothing typed (`compose_into`):
//! 1. the agent in front is proven by its process (`proven_tui`);
//! 2. its box is recognized and empty. A draft in it refuses as `draft`, and
//!    the view offers Bring Here or Show Terminal (R-28); a dialog or a panel
//!    refuses as `prompt`, and the view hands off to the terminal;
//! 3. nobody has typed there lately (`ready`);
//! 4. bracketed paste is known to be on (`proven_tui`).
//!
//! **The paste.** Each image is written to the runner's paste directory and
//! its path pasted on its own, and the box read back until it shows one more
//! `[Image #N]`; then the text is pasted once. The box is read back against
//! what claude draws for the paste (`composer::drawn`): the text as typed, or
//! `[Pasted text #N +K lines]` past claude's threshold. A slash command goes
//! in as its name alone first, and on only when the popup claude opens
//! highlights exactly that command (`/cost` highlights `/usage`: refused);
//! then its arguments. A key someone types meanwhile stops it: no Enter.
//!
//! **The Enter.** Between turns, at once. Mid-turn, as `terminal tell`'s is,
//! through the session's fence (`mid_turn`), so it can't land on a dialog;
//! claude puts the message in its own queue (R-29).
//!
//! **The confirmation.** Queued once claude's transcript has its `enqueue`
//! record, or its `UserPromptSubmit` hook names the prompt in the turn
//! running; Sent once that hook names it as a turn of its own. On claude
//! 2.1.290 a message submitted mid-turn fires the hook at once, naming the
//! running turn's `prompt_id`, so a hook for a turn already named is a
//! queued one (`HookAsks::prompted`), never a Sent. A session no hook was
//! ever heard from is confirmed by the prompt's record in the transcript.
//! None of these within `CONFIRM_SETTLES`: `unconfirmed`.
//!
//! **Not typed.** A text starting with `!`, which turns claude's box into a
//! shell (`command`); a command whose name isn't one (a path); claude's
//! own commands that aren't prompts, which open a panel or act at once and
//! never reach the hook (`/model`, `/usage`, `/config`, `/resume`,
//! `/agents`, `/status`, `/clear`, `/compact`, …): `handoff`, and the view
//! opens the terminal. A command mid-turn is refused as `busy`. codex is
//! typed into between turns only, with its own drawn forms and refusals
//! (`codex`, ov-416); any other agent is refused as `unsupported`. Only a
//! terminal pane; a chat pane has a prompt channel of its own
//! (`terminal.agent_prompt`).
//!
//! Each refusal is a `DomainError::Conflict` naming why with a stable word:
//! `tell`'s (`busy`, `prompt`, `draft`, `typing`, `not_an_agent`,
//! `unfamiliar`, `unproven`, `too_long`, `command`, `not_running`,
//! `paste_left`, `left_at_shell`, `dialog`, `unconfirmed`), and `handoff`,
//! `unsupported` and `unconfirmable`; for codex, `picker` and `too_tall`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use farcooler_core::composer::{self, Composer, drawn::Expected};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::{PaneMode, Terminal};
use uuid::Uuid;

use super::tell::held_word;
use super::{PASTE_POLL, PASTE_SETTLES, TOLD_SPACING_MS, Turn, foreground_agent, may_be_typed_to, mid_turn, queues_mid_turn, registry_turn};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

mod codex;

/// The longest text composed, in characters: far past anything typed, short
/// of what a paste through tmux should carry.
pub(crate) const LONGEST_TEXT: usize = 100_000;

/// The most images in one message.
pub(crate) const MOST_IMAGES: usize = crate::pastes::staged::MOST_IMAGES;

/// How long after the Enter claude has to say it took the message.
const CONFIRM_SETTLES: Duration = Duration::from_secs(5);

/// claude 2.1.290's own commands that aren't prompts: each opens a panel or
/// acts at once, and none reaches `UserPromptSubmit`, so none could be
/// confirmed. Read from the CLI's command table (`type:"local"` and
/// `type:"local-jsx"`); `/clear` and `/compact` were run, and no hook came.
pub(crate) const NOT_PROMPTS: &[&str] = &[
    "add-dir", "advisor", "agents", "artifacts", "auto-mode-setup", "autocompact", "autofix-pr", "background", "branch",
    "brief", "btw", "bug", "cd", "clear", "cloud-plugins", "color", "compact", "config", "context", "copy", "cost",
    "daemon", "design-consent", "design-login", "design-revoke", "desktop", "diff", "doctor", "effort", "exit",
    "export", "extra-usage", "fast", "feedback", "focus", "fork", "goal", "heapdump", "help", "hooks", "ide", "import",
    "install", "install-github-app", "install-slack-app", "keybindings", "list-agents", "login", "logout", "loops",
    "mcp", "memory", "mobile", "model", "output-style", "passes", "pause-memory", "permissions", "plan", "plugin",
    "powerup", "privacy-settings", "pro-trial-expired", "radio", "rate-limit-options", "recap", "release-notes",
    "reload-plugins", "reload-skills", "remote-control", "remote-env", "rename", "restart", "resume", "rewind",
    "scroll-speed", "session", "setup-bedrock", "setup-vertex", "skill-doctor", "skills", "status", "stickers", "stop",
    "subtask", "tasks", "teleport", "terminal-setup", "theme", "tui", "ultraplan", "ultrareview", "upgrade",
    "usage", "usage-credits", "version", "voice", "web-setup", "wellbeing", "workflow-launch-exec", "workflows",
];

/// What a person composed, checked and ready to type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Composition {
    /// The text, its line breaks `\n`, every other control and invisible
    /// character written out, trailing whitespace and leading blank lines
    /// dropped. For a command, its arguments with their leading space.
    pub(crate) text: String,
    /// A slash command's name, `/init`, typed before `text`.
    pub(crate) command: Option<String>,
    /// Each image's bytes and extension.
    pub(crate) images: Vec<(Vec<u8>, &'static str)>,
}

impl Composition {
    /// What claude is sent, as its hook and transcript say it: each image's
    /// placeholder as the box showed it, then the text.
    fn submitted(&self, placeholders: &[String]) -> String {
        let mut parts: Vec<String> = placeholders.to_vec();
        match &self.command {
            Some(name) => parts.push(format!("{name}{}", self.text)),
            None if !self.text.is_empty() => parts.push(self.text.clone()),
            None => {}
        }
        parts.join(" ")
    }
}

/// Check `raw` and `images` (each a claimed MIME type and bytes) as a message
/// to compose, or refuse it: `text` when there's nothing to send, `image`
/// for bytes that aren't an image, `images` for too many or beside a command,
/// `images_too_large` past `MAX_COMPOSE_UPLOAD_BYTES` together (those the
/// request carried are held to `MAX_COMPOSE_IMAGE_BYTES` before this, by
/// `pastes::staged::images`),
/// `too_long`, `command` for a shell escape or
/// a command that isn't one, and `handoff` for one of claude's own that isn't a prompt.
pub(crate) fn composition(raw: &str, images: &[(String, Vec<u8>)]) -> Result<Composition> {
    let text = normalized(raw);
    if text.is_empty() && images.is_empty() {
        return Err(DomainError::InvalidArgument { what: "text" });
    }
    if text.chars().count() > LONGEST_TEXT {
        return Err(DomainError::Conflict { what: "too_long" });
    }
    if images.len() > MOST_IMAGES {
        return Err(DomainError::InvalidArgument { what: "images" });
    }
    if images.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() > farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES {
        return Err(DomainError::Conflict { what: "images_too_large" });
    }
    let mut kept = Vec::new();
    for (_, bytes) in images {
        let ext = match crate::pastes::sniff(bytes) {
            Some(crate::pastes::Kind::Png) => "png",
            Some(crate::pastes::Kind::Jpeg) => "jpg",
            Some(crate::pastes::Kind::Gif) => "gif",
            Some(crate::pastes::Kind::Webp) => "webp",
            None => return Err(DomainError::InvalidArgument { what: "image" }),
        };
        kept.push((bytes.clone(), ext));
    }
    // Read past leading spaces: whether claude trims them before it routes
    // `/` and `!` wasn't measured, so a command behind a space is still one
    // (a panel never typed), and goes in without the space.
    let lead = text.trim_start();
    if lead.starts_with('!') {
        return Err(DomainError::Conflict { what: "command" });
    }
    let Some(rest) = lead.strip_prefix('/') else {
        return Ok(Composition { text, command: None, images: kept });
    };
    let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    let named = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    if !named {
        return Err(DomainError::Conflict { what: "command" });
    }
    if NOT_PROMPTS.contains(&name.as_str()) {
        return Err(DomainError::Conflict { what: "handoff" });
    }
    if !kept.is_empty() {
        return Err(DomainError::InvalidArgument { what: "images" });
    }
    let args = rest[name.len()..].to_string();
    Ok(Composition { text: args, command: Some(format!("/{name}")), images: kept })
}

/// `raw` as it's typed: CR LF and CR as LF, each one break to claude as to
/// `composer::drawn`, which counts LFs; every other control character, and every
/// invisible one, written out (`one_line`'s rule) rather than sent; trailing
/// whitespace and leading blank lines dropped.
pub(crate) fn normalized(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    for c in unified.chars() {
        match c {
            '\n' | '\t' => out.push(c),
            c if c.is_control() || super::invisible(c) => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    let trimmed = out.trim_end();
    let first = trimmed.split('\n').take_while(|line| line.trim().is_empty()).map(|line| line.len() + 1).sum::<usize>();
    trimmed[first.min(trimmed.len())..].to_string()
}

/// How a read-back ended.
enum Shown {
    /// The box shows what was pasted, as this.
    Yes(Composer),
    /// Someone typed since the paste began.
    Typed,
    /// It never did.
    No,
}

impl Watcher {
    /// Type `raw` and `images` into the claude in terminal `id` and submit
    /// it: `Turn::Between` when claude took it as its next prompt (Sent),
    /// `Turn::During` when it's in claude's queue behind the turn running
    /// (Queued). See this module's docs.
    pub(crate) async fn compose_into(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)]) -> Result<Turn> {
        let to = self.service.store.get_terminal(id)?;
        if to.pane_mode == PaneMode::Agent {
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode == PaneMode::Changes || !may_be_typed_to(&to.command_preset, to.role) {
            return Err(DomainError::Conflict { what: "not_an_agent" });
        }
        if !self.service.is_running(&to) {
            return Err(DomainError::Conflict { what: "not_running" });
        }
        let composed = composition(raw, images)?;
        // Nothing else types into this box, an answer, a draft or another
        // send, from the gate's first check through the confirmation
        // (`Watcher::typing`, ov-372).
        let _typing = self.typing(to.id).await;
        self.compose_locked(&to, &composed).await
    }

    /// `compose_into` past its checks of the request, under `to`'s typing
    /// lock: the gate, the pastes, the Enter and the confirmation.
    async fn compose_locked(&self, to: &Terminal, composed: &Composition) -> Result<Turn> {
        // Written before the gate, so nothing slow sits between its checks
        // and the first paste (review finding 8).
        let paths = self.write_images(composed)?;
        // Deleted at once if the send ends before a path is pasted; once one
        // is, claude may read them all, so the sweep takes them within a day.
        let mut unpasted = Unpasted(paths.clone());
        let told = self.told.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if let Some(left) = told.map(|at| TOLD_SPACING_MS - (now_millis() - at)).filter(|left| *left > 0) {
            tokio::time::sleep(Duration::from_millis(left as u64)).await;
        }
        // The watcher's reading can lag a turn: while claude runs a hook, the
        // pane's foreground is the hook's process, and the watcher reads no
        // agent there for a sample. Held as busy for that alone, the send
        // goes on if claude's registry says it's working (below).
        let ready = self.ready(to).await;
        if let Err(held) = ready
            && held != super::Held::Busy
        {
            return Err(DomainError::Conflict { what: held_word(held) });
        }
        let proven = self.proven_tui(to).await.map_err(|held| DomainError::Conflict { what: held_word(held) })?;
        // codex: between turns only, and its own drawn forms (`codex`).
        if proven.preset == "codex" {
            // The watcher can lose codex: under fish, which runs `-c` without
            // job control, the pane's foreground command reads `fish`, and
            // the watcher's word goes to none (measured, ov-416). The fresh
            // capture and the rollout `proven_tui` just read are the word
            // then, as claude's registry is for claude: between turns, and
            // nobody typing, goes on.
            if ready.is_err() && (proven.turn == Turn::During || self.typed_by_hand_lately(to.id, now_millis())) {
                return Err(DomainError::Conflict { what: if proven.turn == Turn::During { "busy" } else { "typing" } });
            }
            return self.compose_codex(to, composed, &proven, &paths, &mut unpasted).await;
        }
        if !queues_mid_turn(proven.preset) {
            return Err(DomainError::Conflict { what: "unsupported" });
        }
        // claude reads a backslash before Enter as a line break, not a send:
        // the text would be left in its box, unconfirmed (ov-393 review 8).
        // codex 0.153.4 sends it as typed.
        if composed.text.ends_with('\\') {
            return Err(DomainError::Conflict { what: "backslash" });
        }
        // Where claude will say it took the message: its session's hooks and
        // transcript, found before anything is typed. Its registry says too
        // whether a turn runs, which the screen can miss: after a long paste
        // claude's footer says `paste again to expand`, working or not, and
        // the screen reads idle (`claude-2.1.290-working-queued-long-paste`).
        let Some((session, transcript, busy)) = session_of(proven.pid).await else {
            return Err(DomainError::Conflict { what: "unconfirmable" });
        };
        if ready.is_err() && (!busy || self.typed_by_hand_lately(to.id, now_millis())) {
            return Err(DomainError::Conflict { what: if busy { "typing" } else { "busy" } });
        }
        let proven = super::Proven { turn: if busy { Turn::During } else { proven.turn }, ..proven };
        if composed.command.is_some() && proven.turn == Turn::During {
            return Err(DomainError::Conflict { what: "busy" });
        }
        // Mid-turn, the witness says it's safe to press Enter at all.
        let witness = match proven.turn {
            Turn::Between => None,
            Turn::During => Some(
                self.witness(&proven, to, &composed.submitted(&[])).await.ok_or(DomainError::Conflict { what: "busy" })?,
            ),
        };
        let from = std::fs::metadata(&transcript).map(|m| m.len()).unwrap_or(0);
        let preset = proven.preset;
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let started = now_millis();
        let mut expected = Expected::default();
        let mut held = Composer::Empty;
        let mut first = true;
        let mut paste = |text: String| {
            let (runtime, tty, was_first) = (&runtime, proven.tty.as_str(), std::mem::replace(&mut first, false));
            async move { self.paste_checked(to, runtime, tty, preset, was_first, &text).await }
        };
        if self.fail_sends_for_tests() {
            return Err(DomainError::OperationFailed);
        }
        for path in &paths {
            paste(quoted(path)).await?;
            unpasted.0.clear();
            expected = expected.then_image();
            held = self.shown(to, preset, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        let placeholders = composer::drawn::images(&held);
        if let Some(name) = &composed.command {
            paste(name.clone()).await?;
            expected = expected.then_paste(name);
            self.shown(to, preset, started, &expected, Some(name)).await.left(&proven.tty, preset).await?;
        }
        if !composed.text.is_empty() {
            let text = if paths.is_empty() { composed.text.clone() } else { format!(" {}", composed.text) };
            paste(text.clone()).await?;
            expected = expected.then_paste(&text);
            self.shown(to, preset, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        let submitted = composed.submitted(&placeholders);
        let since = Instant::now();
        let queued_before = mid_turn::enqueued_before(&transcript, from, &submitted);
        match &witness {
            None => {
                #[cfg(test)]
                if let Some(run) = self.before_enter.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    run();
                }
                self.still_between(to, preset, proven.pid, started, &expected).await?;
                runtime.send_bytes_hex(to.id, "0d").await?
            }
            Some(witness) => {
                let witness = witness.clone().pasted_at(started);
                self.enter_expecting(to, preset, &witness, &expected).await.map_err(|no| match no {
                    mid_turn::NoEnter::Dialog => DomainError::Conflict { what: "dialog" },
                    mid_turn::NoEnter::Moved => DomainError::Conflict { what: "paste_left" },
                    mid_turn::NoEnter::Failed => DomainError::OperationFailed,
                })?
            }
        }
        self.mark_told(to.id);
        let took = Confirm { session, transcript, from, queued_before, since, submitted };
        self.took(&took).await.ok_or(DomainError::Conflict { what: "unconfirmed" })
    }

    /// One paste into `to`, right after proving again what the gate proved
    /// before it: the agent in front (`foreground_agent`) and bracketed paste
    /// on. Between the gate and a paste, claude may have exited and a shell
    /// with no bracketed paste taken the pane, which would run a multi-line
    /// text's lines. Refused before the `first` paste as the gate would be;
    /// after one, the text so far is left at a shell or in the box.
    async fn paste_checked(&self, to: &Terminal, runtime: &Runtime, tty: &str, preset: &str, first: bool, text: &str) -> Result<()> {
        #[cfg(test)]
        if let Some(run) = self.before_paste.lock().unwrap_or_else(|e| e.into_inner()).take() {
            run();
        }
        let agent = foreground_agent(tty).await == Some(preset);
        let held = if agent { self.bracketed(to.id).await.err() } else { Some(super::Held::NotAnAgent) };
        match (held, first) {
            (None, _) => {}
            (Some(held), true) => return Err(DomainError::Conflict { what: held_word(held) }),
            (Some(_), false) => return Err(DomainError::Conflict { what: if agent { "paste_left" } else { "left_at_shell" } }),
        }
        let hex: String = crate::pastes::encode_paste(true, text).iter().map(|b| format!("{b:02x}")).collect();
        runtime.send_bytes_hex(to.id, &hex).await
    }

    /// The last check before a between-turns Enter, which goes in with no
    /// fence: the pastes can take seconds, and claude may have begun a turn
    /// of its own meanwhile (a background task finishing). Nobody typed since
    /// the paste began; a fresh capture reads Idle with the box showing
    /// `expected`; and claude's registry still says idle. Anything else
    /// leaves the text in the box: `paste_left`.
    async fn still_between(&self, to: &Terminal, preset: &str, pid: i32, started: i64, expected: &Expected) -> Result<()> {
        let left = DomainError::Conflict { what: "paste_left" };
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
            return Err(left);
        }
        let (screen, _, _) = self.service.screen(to.id).await?;
        if self.service.registry().classify(preset, &screen) != AgentActivity::Idle
            || !expected.shown_by(&composer::read(preset, &screen))
            || registry_turn::said_of(pid).await != registry_turn::Said::Idle
        {
            return Err(left);
        }
        Ok(())
    }

    /// Write `composed`'s images to the runner's paste directory, which a
    /// sweep empties of these within a day (`pastes::staged::KEEP_COMPOSED`),
    /// each under a name of its own.
    fn write_images(&self, composed: &Composition) -> Result<Vec<PathBuf>> {
        if composed.images.is_empty() {
            return Ok(Vec::new());
        }
        let dir = crate::paths::pastes_dir_in(self.service.root_dir())?;
        let mut paths = Vec::new();
        for (bytes, ext) in &composed.images {
            let path = dir.join(format!("{}{}.{ext}", crate::pastes::staged::COMPOSED_PREFIX, Uuid::now_v7().simple()));
            std::fs::write(&path, bytes).map_err(|e| {
                tracing::warn!(error = %e, "couldn't write a composed image");
                DomainError::OperationFailed
            })?;
            paths.push(path);
        }
        Ok(paths)
    }

    /// Read `to`'s box back, until `PASTE_SETTLES`, for `expected`; with
    /// `command`, until claude's popup highlights exactly it as well.
    async fn shown(&self, to: &Terminal, preset: &str, started: i64, expected: &Expected, command: Option<&str>) -> Shown {
        let deadline = tokio::time::Instant::now() + PASTE_SETTLES;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
                return Shown::Typed;
            }
            let Ok((screen, _, _)) = self.service.screen(to.id).await else { continue };
            if !matches!(self.service.registry().classify(preset, &screen), AgentActivity::Idle | AgentActivity::Working) {
                continue;
            }
            let held = composer::read(preset, &screen);
            let popup = composer::drawn::highlighted_command(&screen);
            if expected.shown_by(&held) && popup.as_deref() == command {
                return Shown::Yes(held);
            }
        }
        Shown::No
    }

    /// Whether claude took `took`, and how, within `CONFIRM_SETTLES`. See
    /// this module's docs, "The confirmation".
    async fn took(&self, took: &Confirm) -> Option<Turn> {
        let asks = self.service.hooks().asks();
        let deadline = tokio::time::Instant::now() + CONFIRM_SETTLES;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            let hooked = asks.hooked(&took.session);
            let recorded = mid_turn::recorded_as(&took.transcript, took.from, &took.submitted, took.queued_before);
            if recorded == Some("enqueue") {
                return Some(Turn::During);
            }
            // A hook for a message claude queued names the running turn
            // (`HookAsks::prompted`): Queued, as its `enqueue` record would
            // say. Answered unconfirmed instead, a person sends it again and
            // claude runs it twice.
            match hooked.then(|| asks.prompted_since(&took.session, took.since, &took.submitted)).flatten() {
                Some(false) => return Some(Turn::Between),
                Some(true) => return Some(Turn::During),
                None => {}
            }
            if recorded.is_some() && !hooked {
                return Some(Turn::Between);
            }
        }
        None
    }
}

impl Shown {
    /// The box as shown, or the refusal for a paste that didn't show: left
    /// at a shell when the agent has gone, else left in the box.
    async fn left(self, tty: &str, preset: &str) -> Result<Composer> {
        match self {
            Shown::Yes(held) => Ok(held),
            Shown::Typed | Shown::No => {
                if foreground_agent(tty).await != Some(preset) {
                    return Err(DomainError::Conflict { what: "left_at_shell" });
                }
                Err(DomainError::Conflict { what: "paste_left" })
            }
        }
    }
}

/// A compose's written images whose paths nothing has pasted yet: deleted
/// when the send ends, so a refused one leaves no copy behind (ov-393).
struct Unpasted(Vec<PathBuf>);

impl Drop for Unpasted {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Where to look for claude saying it took a message.
struct Confirm {
    session: String,
    transcript: PathBuf,
    /// The transcript's length before the paste.
    from: u64,
    /// The same text was queued before the paste.
    queued_before: bool,
    /// When the Enter went: a hook from before is another prompt's.
    since: Instant,
    submitted: String,
}

/// claude's session and transcript, from its process (`mid_turn`), and
/// whether its registry says anything but idle (`registry_turn`): `busy`,
/// `shell`, no status. `None` without a live registry entry, read through
/// `claude_registry` with its `procStart` check, so a stale file is nothing.
async fn session_of(pid: i32) -> Option<(String, PathBuf, bool)> {
    let config = registry_turn::config_of(pid).await?;
    let not_idle = match registry_turn::said(&config, pid) {
        registry_turn::Said::Idle => false,
        registry_turn::Said::NotIdle => true,
        registry_turn::Said::Nothing => return None,
    };
    let (session, transcript) = mid_turn::transcript_in(&config, pid)?;
    Some((session, transcript, not_idle))
}

/// An image's path as pasted: as is, or in single quotes when it has a
/// space in it, which claude reads as one path (measured on 2.1.290).
fn quoted(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    if path.contains(char::is_whitespace) { format!("'{path}'") } else { path }
}
