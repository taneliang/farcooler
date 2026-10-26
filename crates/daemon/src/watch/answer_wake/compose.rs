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
//! **The paste.** Last of all, right before the first paste, the box is
//! captured once more and must still read empty (`box_still_empty`): a draft
//! typed straight into the terminal while the gate did its slow reads leaves
//! no input mark. A draft then is `draft`, which keeps a held message held
//! and refuses any other send (ov-556).
//!
//! Each image is written to the runner's paste directory and its path pasted
//! on its own, and the box read back until it shows one more
//! `[Image #N]`; then the text is pasted once. The box is read back against
//! what claude draws for the paste (`composer::drawn`): the text as typed, or
//! `[Pasted text #N +K lines]` past claude's threshold, or, in a box claude
//! has grown to its tallest and scrolled, the text's last rows
//! (`Expected::shown_on`, ov-618). A slash command goes
//! in as its name alone first, and on only when the popup claude opens
//! highlights exactly that command (`/cost` highlights `/usage`: refused);
//! then its arguments. A key someone types meanwhile stops it: no Enter.
//!
//! **The Enter.** Between turns, at once. Mid-turn, as `terminal tell`'s is,
//! through the session's fence (`mid_turn`), so it can't land on a dialog;
//! claude puts the message in its own queue (R-29). A claude whose turn has
//! ended while background agents run is between turns (ov-468). One that
//! can't be typed to safely yet, or a command mid-turn, is answered Queued
//! and held until it can (`held`, ov-468). A turn claude begins between the
//! paste and a between-turns Enter takes the message as a mid-turn send
//! would, under the fence, or, with no fence to be had, is answered Queued
//! while its Enter waits out the turn off the lock (`pending`, ov-808).
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
//! opens the terminal. A command mid-turn is held for the turn's end. codex
//! is typed into between turns only, and refused as `busy` mid-turn, with
//! its own drawn forms and refusals (`codex`, ov-416); any other agent is
//! refused as `unsupported`. Only a terminal pane.
//!
//! Each refusal is a `DomainError::Conflict` naming why with a stable word:
//! `tell`'s (`busy`, `prompt`, `draft`, `typing`, `not_an_agent`,
//! `unfamiliar`, `unproven`, `too_long`, `command`, `not_running`,
//! `paste_left`, `left_at_shell`, `dialog`, `unconfirmed`), and `handoff`,
//! `unsupported` and `unconfirmable`; for codex, `picker` and `too_tall`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use farcooler_core::activity::Background;
use farcooler_core::composer::{self, Composer, drawn::Expected};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::{PaneMode, Terminal};
use uuid::Uuid;

use super::tell::held_word;
use super::{PASTE_POLL, Turn, foreground_agent, may_be_typed_to, mid_turn, queues_mid_turn, registry_turn};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

pub(super) mod codex;
pub(crate) mod held;
mod pending;

/// The longest text composed, in characters: far past anything typed, short
/// of what a paste through tmux should carry.
pub(crate) const LONGEST_TEXT: usize = 100_000;

/// The most images in one message.
pub(crate) const MOST_IMAGES: usize = crate::pastes::staged::MOST_IMAGES;

/// How long after the Enter claude has to say it took the message.
pub(crate) const CONFIRM_SETTLES: Duration = Duration::from_secs(5);

/// How long a composed message whose claude began a turn between the paste
/// and the Enter waits for a fenced Enter or the turn's end before it's left
/// in the box (`pending`, ov-808): a short turn's worth, as a background
/// shell finishing starts one. Nothing waits on it: the send was answered
/// Queued, and the box is taken only for each try.
pub(crate) const TURN_BEGUN_WAIT: Duration = Duration::from_secs(30);

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
#[cfg(test)]
pub(crate) fn composition(raw: &str, images: &[(String, Vec<u8>)]) -> Result<Composition> {
    composition_with(raw, images, &[])
}

/// `composition`, with `files` written on the runner (ov-454): their paths
/// go before the text once it's checked, so a path's leading `/` is never
/// read as a command; beside a command, refused as `files`.
pub(crate) fn composition_with(raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<Composition> {
    let text = normalized(raw);
    if text.is_empty() && images.is_empty() && files.is_empty() {
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
        let text = if files.is_empty() { text } else { crate::pastes::staged::with_files(files, &text) };
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
    if !files.is_empty() {
        return Err(DomainError::InvalidArgument { what: "files" });
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
    #[cfg(test)]
    pub(crate) async fn compose_into(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)]) -> Result<Turn> {
        self.compose_files_into(id, raw, images, &[]).await
    }

    /// `compose_into`, with `files` already written on the runner, their
    /// paths typed before the text (ov-454). Refused before anything was
    /// typed, the files go at once; once it may have been, the sweep takes
    /// them with the composed images.
    pub(crate) async fn compose_files_into(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<Turn> {
        let composed = self.compose_or_hold(id, raw, images, files).await;
        if let Err(e) = &composed
            && !matches!(e, DomainError::Conflict { what: "unconfirmed" })
        {
            for file in files {
                let _ = std::fs::remove_file(file);
            }
        }
        composed
    }

    /// `compose_files_into` past what becomes of its files and whether it's
    /// held (`held`): `held::HOLD` for a send to wait.
    async fn compose_checked(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<Turn> {
        let (to, composed) = self.compose_request(id, raw, images, files)?;
        // Nothing else types into this box, an answer, a draft or another
        // send, from the gate's first check through the confirmation
        // (`Watcher::typing`, ov-372).
        let _typing = self.typing(to.id).await;
        self.compose_locked(&to, &composed, raw).await
    }

    /// The request's own checks: the pane, and the message (`composition_with`).
    fn compose_request(&self, id: Uuid, raw: &str, images: &[(String, Vec<u8>)], files: &[PathBuf]) -> Result<(Terminal, Composition)> {
        let to = self.service.store.get_terminal(id)?;
        if to.pane_mode == PaneMode::Agent {
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode.is_client_drawn() || !may_be_typed_to(&to.command_preset, to.role) {
            return Err(DomainError::Conflict { what: "not_an_agent" });
        }
        if !self.service.is_running(&to) {
            return Err(DomainError::Conflict { what: "not_running" });
        }
        let composed = composition_with(raw, images, files)?;
        Ok((to, composed))
    }

    /// `compose_into` past its checks of the request, under `to`'s typing
    /// lock: the gate, the pastes, the Enter and the confirmation. `raw` is
    /// what the person wrote, for a row while its Enter waits (`pending`).
    async fn compose_locked(&self, to: &Terminal, composed: &Composition, raw: &str) -> Result<Turn> {
        // Written before the gate, so nothing slow sits between its checks
        // and the first paste (review finding 8).
        let paths = self.write_images(composed)?;
        // Deleted at once if the send ends before a path is pasted; once one
        // is, claude may read them all, so the sweep takes them within a day.
        let mut unpasted = Unpasted(paths.clone());
        let told = self.told.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if let Some(left) = told.map(|at| self.told_spacing_ms() - (now_millis() - at)).filter(|left| *left > 0) {
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
        let (session, transcript, busy) = match session_of(proven.pid, proven.background).await {
            Ok(found) => found,
            // Nothing is typed yet: a held message tries again (`held::TORN`).
            Err(Gone::Torn) => return Err(DomainError::Conflict { what: held::TORN }),
            Err(Gone::Nothing) => return Err(DomainError::Conflict { what: "unconfirmable" }),
        };
        // From here, what can't be typed now is held, not refused (`held`).
        if ready.is_err() && (!busy || self.typed_by_hand_lately(to.id, now_millis())) {
            return Err(DomainError::Conflict { what: if busy { "typing" } else { held::HOLD } });
        }
        let proven = super::Proven { turn: if busy { Turn::During } else { proven.turn }, ..proven };
        if composed.command.is_some() && proven.turn == Turn::During {
            return Err(DomainError::Conflict { what: held::HOLD });
        }
        // Mid-turn, the witness says it's safe to press Enter at all.
        let witness = match proven.turn {
            Turn::Between => None,
            Turn::During => Some(
                self.witness(&proven, to, &composed.submitted(&[])).await.ok_or(DomainError::Conflict { what: held::HOLD })?,
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
            held = self.shown(to, &proven, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        let placeholders = composer::drawn::images(&held);
        if let Some(name) = &composed.command {
            paste(name.clone()).await?;
            expected = expected.then_paste(name);
            self.shown(to, &proven, started, &expected, Some(name)).await.left(&proven.tty, preset).await?;
        }
        if !composed.text.is_empty() {
            let text = if paths.is_empty() { composed.text.clone() } else { format!(" {}", composed.text) };
            paste(text.clone()).await?;
            expected = expected.then_paste(&text);
            self.shown(to, &proven, started, &expected, None).await.left(&proven.tty, preset).await?;
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
                if let Entered::Pending(left) = self.enter_between(to, &proven, started, &expected, &submitted).await? {
                    let confirm = Confirm { session, transcript, from, queued_before, since, submitted };
                    let echo = pending::echo(placeholders.len(), raw);
                    self.pend_enter(pending::Pending { to: to.clone(), proven, started, expected, confirm, echo, left });
                    return Ok(Turn::During);
                }
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
        if first {
            self.proven_again(to, tty, preset).await?;
        } else {
            let agent = foreground_agent(tty).await == Some(preset);
            if !agent || self.bracketed(to.id).await.is_err() {
                return Err(DomainError::Conflict { what: if agent { "paste_left" } else { "left_at_shell" } });
            }
        }
        runtime.paste_text(to.id, preset, text).await
    }

    /// The gate's proofs again, right before a first paste, refused as the
    /// gate would refuse them: the agent in front (`not_an_agent`),
    /// bracketed paste on, and last, the box still empty. That last look is
    /// taken after every slow read in the gate (claude's registry, a
    /// witness, the process table): typing straight into the terminal leaves
    /// no input mark, so a draft begun since the gate's capture shows only
    /// here. Held as `draft`, which keeps a held message held
    /// (`held::STILL_HELD`). A queued Prompt makes the same checks
    /// (`type_into`, ov-620).
    pub(super) async fn proven_again(&self, to: &Terminal, tty: &str, preset: &str) -> Result<()> {
        if foreground_agent(tty).await != Some(preset) {
            return Err(DomainError::Conflict { what: held_word(super::Held::NotAnAgent) });
        }
        self.bracketed(to.id).await.map_err(|held| DomainError::Conflict { what: held_word(held) })?;
        self.box_still_empty(to, preset).await
    }

    /// A fresh capture of `to`'s composer box, which must read empty:
    /// `draft` for text in it, `prompt` for a box that no longer reads as one
    /// (a dialog opened). Both leave a held message held. Nothing is typed.
    pub(super) async fn box_still_empty(&self, to: &Terminal, preset: &str) -> Result<()> {
        let (screen, _, _) = self.service.screen(to.id).await?;
        match composer::read(preset, &screen) {
            Composer::Empty => Ok(()),
            Composer::Holds(_) => Err(DomainError::Conflict { what: "draft" }),
            Composer::Unrecognized => Err(DomainError::Conflict { what: "prompt" }),
        }
    }

    /// The last check before a between-turns Enter, which goes in with no
    /// fence: the pastes can take seconds, and claude may have begun a turn
    /// of its own meanwhile (a background task finishing). Nobody typed since
    /// the paste began; a fresh capture reads Idle with the box showing
    /// `expected`; and claude's registry still says idle, or with `unlisted`,
    /// says nothing at all (an answer's claude that has no registry, as the
    /// gate allows). Anything else leaves the text in the box: `paste_left`,
    /// logged with which check it was (`left_in_box`). `proven` is the
    /// gate's: the agent, and the background work it saw (`between_or_why`).
    pub(super) async fn still_between(&self, to: &Terminal, proven: &super::Proven, started: i64, expected: &Expected, unlisted: bool) -> Result<()> {
        match self.between_or_why(to, proven, started, expected, unlisted).await? {
            None => Ok(()),
            Some(left) => Err(left.logged(&self.service.registry(), to, proven)),
        }
    }

    /// `still_between`'s checks, unlogged: `None` between turns, else the
    /// one that refused.
    ///
    /// Background agents keep claude's registry busy between turns, so
    /// beside them `busy` is no turn (ov-468); beside shells alone it is one
    /// (ov-795). Which work runs is read off the screen, and the agents are
    /// told by their rows under the box, which claude 2.1.309 drops once a
    /// tall box leaves them no room: three agents and two shells at 59x23,
    /// a six-row message pasted in, leave only the mode line's `· 2 shells`,
    /// the registry `busy` for the agents throughout (ov-808,
    /// `captures/claude-2.1.309-agents-shells-*-59x23-e.txt`). So agents the
    /// gate saw (`proven.background`) still count when the screen no longer
    /// shows them. Gone, they leave the registry `shell` or `idle`, never
    /// `busy`, but for a turn begun the moment they ended, which the screen
    /// reads Idle only until its spinner is drawn: the same window a registry
    /// read `idle` has before any between-turns Enter. A registry `waiting`
    /// (a dialog it says is up before the screen draws it) reads as a turn
    /// too, and is waited out rather than left at once: neither Enter can
    /// take a dialog, as the plain one needs the registry idle and the
    /// fenced one's box check reads the dialog (`NoEnter::Dialog`).
    async fn between_or_why(&self, to: &Terminal, proven: &super::Proven, started: i64, expected: &Expected, unlisted: bool) -> Result<Option<Left>> {
        let (preset, pid, seen) = (proven.preset, proven.pid, proven.background);
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
            return Ok(Some(Left { check: "typed", screen: None, turn: false }));
        }
        let (screen, _, _) = self.service.screen(to.id).await?;
        let registry = self.service.registry();
        // Background agents keep claude's registry busy between turns: the
        // screen still showing them, or the gate having seen them, is what
        // says that's all it is (ov-468, ov-808).
        let main = registry.classify_main(preset, &screen);
        let background = match main {
            AgentActivity::Idle if seen == Background::Agents => Background::Agents,
            AgentActivity::Idle => registry.background_work(preset, &screen),
            _ => Background::None,
        };
        let (check, turn) = if main != AgentActivity::Idle {
            ("not_idle", main == AgentActivity::Working)
        } else if !expected.shown_on(preset, &screen) {
            ("not_shown", false)
        } else {
            match registry_turn::said_beside_of(pid, background).await {
                registry_turn::Said::Idle => return Ok(None),
                registry_turn::Said::Nothing if unlisted => return Ok(None),
                registry_turn::Said::Nothing => ("registry_none", false),
                registry_turn::Said::NotIdle if background != Background::None => ("registry_busy", true),
                registry_turn::Said::NotIdle => ("registry_busy_no_background", true),
                registry_turn::Said::Torn => ("registry_torn", false),
            }
        };
        Ok(Some(Left { check, screen: Some(screen), turn }))
    }

    /// The between-turns Enter for a compose, past `between_or_why`, tried
    /// once under the typing lock. Between turns, the Enter. A turn claude
    /// began since the gate (its registry `busy` or `waiting` with no agents
    /// seen, or its spinner drawn) no longer leaves the message in the box
    /// (ov-808): claude queues a message entered mid-turn, so it goes in as a
    /// mid-turn send does, under the fence (`mid_turn`), when the session's
    /// hooks give a witness now. When they don't (a call in flight, or a
    /// claude no hook is heard from, as the owner's orchestrator, ov-711),
    /// `Entered::Pending`: the send is answered Queued and the Enter waits
    /// out the turn off the lock (`pending`). Never with no fence into a turn
    /// running: a dialog drawn as the key lands would take it as Yes.
    async fn enter_between(&self, to: &Terminal, proven: &super::Proven, started: i64, expected: &Expected, submitted: &str) -> Result<Entered> {
        let Some(left) = self.between_or_why(to, proven, started, expected, false).await? else {
            let runtime = Runtime { marks: None, ..self.service.runtime() };
            runtime.send_bytes_hex(to.id, "0d").await?;
            return Ok(Entered::Now);
        };
        if !left.turn {
            return Err(left.logged(&self.service.registry(), to, proven));
        }
        match self.witness(proven, to, submitted).await {
            Some(witness) => {
                self.enter_fenced_for(to, proven, started, &witness, expected).await.map_err(|what| match what {
                    "failed" => DomainError::OperationFailed,
                    what => DomainError::Conflict { what },
                })?;
                Ok(Entered::Now)
            }
            None => Ok(Entered::Pending(left)),
        }
    }

    /// The fenced Enter for a compose a turn began under, with `witness`
    /// taken now (`mid_turn::Witness::pasted_at`: a key typed since the paste
    /// still stops it; its call and gate windows start at the witness, which
    /// the hooks' own in-flight and quiet checks cover, N1 of the ov-808
    /// review). Refused with the word a send is refused with.
    async fn enter_fenced_for(&self, to: &Terminal, proven: &super::Proven, started: i64, witness: &mid_turn::Witness, expected: &Expected) -> std::result::Result<(), &'static str> {
        let witness = witness.clone().pasted_at(started);
        self.enter_expecting(to, proven.preset, &witness, expected).await.map_err(|no| match no {
            mid_turn::NoEnter::Dialog => "dialog",
            mid_turn::NoEnter::Moved => "paste_left",
            mid_turn::NoEnter::Failed => "failed",
        })
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
    async fn shown(&self, to: &Terminal, proven: &super::Proven, started: i64, expected: &Expected, command: Option<&str>) -> Shown {
        let (preset, gate) = (proven.preset, proven.background);
        let deadline = tokio::time::Instant::now() + self.timing().paste_settles;
        let mut last = None;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
                left_in_box(&self.service.registry(), to, "typed", gate, None);
                return Shown::Typed;
            }
            let Ok((screen, _, _)) = self.service.screen(to.id).await else { continue };
            if !matches!(self.service.registry().classify(preset, &screen), AgentActivity::Idle | AgentActivity::Working) {
                last = Some(screen);
                continue;
            }
            let held = composer::read(preset, &screen);
            let popup = composer::drawn::highlighted_command(&screen);
            if expected.shown_on(preset, &screen) && popup.as_deref() == command {
                return Shown::Yes(held);
            }
            last = Some(screen);
        }
        left_in_box(&self.service.registry(), to, "never_shown", gate, last.as_deref().map(|screen| (preset, screen)));
        Shown::No
    }

    /// Whether claude took `took`, and how, within `CONFIRM_SETTLES`. See
    /// this module's docs, "The confirmation".
    async fn took(&self, took: &Confirm) -> Option<Turn> {
        let asks = self.service.hooks().asks();
        let deadline = tokio::time::Instant::now() + self.timing().compose_confirm;
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

/// Say why a paste was left in `to`'s box (ov-795): `check`, the read-back
/// that refused it, and on `screen` its shape, never its text: what the
/// agent reads as, whether background work shows, what the box holds as a
/// kind and a length, and the pane's rows; and `gate`, the background work
/// the gate saw (ov-808), which the screen can stop showing. The owner's
/// report of one had nothing to go on, as nothing was written.
fn left_in_box(registry: &farcooler_core::activity::Registry, to: &Terminal, check: &'static str, gate: Background, screen: Option<(&str, &str)>) {
    let Some((preset, screen)) = screen else {
        tracing::info!(terminal = %to.id, check, ?gate, "a paste was left in the box");
        return;
    };
    let (held, chars) = match composer::read(preset, screen) {
        Composer::Empty => ("empty", 0),
        Composer::Holds(text) => ("holds", text.chars().count()),
        Composer::Unrecognized => ("unrecognized", 0),
    };
    tracing::info!(
        terminal = %to.id,
        check,
        main = ?registry.classify_main(preset, screen),
        background = ?registry.background_work(preset, screen),
        ?gate,
        held,
        chars,
        rows = screen.lines().count(),
        "a paste was left in the box"
    );
}

/// What `enter_between`'s one try came to.
enum Entered {
    /// The Enter went.
    Now,
    /// A turn runs and no fence can be had yet: the Enter waits (`pending`).
    Pending(Left),
}

/// Why `between_or_why` would leave a paste in the box.
pub(super) struct Left {
    /// The check that refused it (`left_in_box`).
    check: &'static str,
    /// The screen it read, if it got that far.
    screen: Option<String>,
    /// A turn may have begun since the gate: claude would queue the message
    /// (`enter_between`).
    turn: bool,
}

impl Left {
    /// Log it (`left_in_box`) and refuse as `paste_left`.
    fn logged(&self, registry: &farcooler_core::activity::Registry, to: &Terminal, proven: &super::Proven) -> DomainError {
        left_in_box(registry, to, self.check, proven.background, self.screen.as_deref().map(|screen| (proven.preset, screen)));
        DomainError::Conflict { what: "paste_left" }
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
#[derive(Clone)]
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
/// `shell`, no status; with `background` work beside a main loop between
/// turns, `busy` and `shell` are that work's (`said_beside`). `Gone` without
/// a live registry entry, read through `claude_registry` with its
/// `procStart` check, so a stale file is nothing; `Torn` for a file that is
/// there but doesn't parse, which is claude mid-write more often than gone
/// (ov-645).
async fn session_of(pid: i32, background: Background) -> std::result::Result<(String, PathBuf, bool), Gone> {
    let config = registry_turn::config_of(pid).await.ok_or(Gone::Nothing)?;
    let not_idle = match registry_turn::said_beside(&config, pid, background) {
        registry_turn::Said::Idle => false,
        registry_turn::Said::NotIdle => true,
        registry_turn::Said::Torn => return Err(Gone::Torn),
        registry_turn::Said::Nothing => return Err(Gone::Nothing),
    };
    let (session, transcript) = mid_turn::transcript_in(&config, pid).ok_or(Gone::Nothing)?;
    Ok((session, transcript, not_idle))
}

/// Why `session_of` found no session.
enum Gone {
    /// No live claude registry entry, or no transcript.
    Nothing,
    /// A registry file that doesn't parse.
    Torn,
}

/// An image's path as pasted: as is, or in single quotes when it has a
/// space in it, which claude reads as one path (measured on 2.1.290).
fn quoted(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    if path.contains(char::is_whitespace) { format!("'{path}'") } else { path }
}
