//! Whether the owner's shell startup files export a Claude credential
//! (ov-651).
//!
//! A member's claude is safe: the `env -u …` sits inside its `-ilc` payload,
//! after the rc files. A member's plain shell pane is not: `env -u … {shell}
//! -il` drops the three variables from the daemon's environment, then the
//! owner's `.zshrc` or `config.fish` runs and may export one again, and a
//! `claude` typed there runs on the owner's account.
//!
//! **Asked, never read.** The daemon starts the owner's login shell as a pane
//! would (`-ilc`, with the three variables dropped from its own environment,
//! so only an rc file can have set one) and runs, per name, `command printenv
//! NAME | command grep -q . && echo NAME`. The value travels down a pipe from
//! `printenv` to `grep`, which only asks whether anything came; it is never an
//! argument of any program (an argument is visible in `/proc/<pid>/cmdline`
//! and in exec logs; zsh and fish run `/bin/test` for `command test`, so a
//! `test -n "$(…)"` would have put it in `/bin/test`'s argv), and never
//! printed. `command` skips an alias, function or builtin an rc file or the
//! shell has for `printenv` or `grep`. What the rc files themselves print (a
//! greeting, an `echo $KEY`) can still reach stdout, which is why only a line
//! that is exactly one of the three names is kept. The payload is simple
//! commands joined by `|`, `&&` and `;`, which fish, zsh and bash read the
//! same.
//!
//! **When.** Once on start and after the members change, in the background:
//! an interactive login shell takes a second or more. The answer is kept, and
//! shown only to the owner and only while the runner has members
//! (`Service::claude_key_exports`).

use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use farcooler_core::claude_keys::NAMES;

use crate::service::Service;

/// How long the login shell may take, rc files and all.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What the login shell runs: for each of `NAMES`, its name when the variable
/// is set and not empty. Ends in `true`, so the shell's exit says nothing.
fn payload() -> String {
    let mut parts: Vec<String> = NAMES.iter().map(|n| format!("command printenv {n} | command grep -q . && echo {n}")).collect();
    parts.push("true".into());
    parts.join("; ")
}

/// The names `shell -ilc` finds set, in `NAMES`' order. Only a line that is
/// exactly a name counts, so nothing else the shell prints (a greeting, an
/// rc file's `echo`) gets further. Fails open to "none": a shell that can't
/// run or doesn't finish in time warns of nothing.
pub(crate) async fn ask(shell: &str) -> Vec<String> {
    ask_within(shell, TIMEOUT).await
}

/// `ask`, with `wait` as the time the shell has. On a timeout the shell's
/// whole process group is killed, not only the shell: an rc file's background
/// job would otherwise outlive it.
pub(crate) async fn ask_within(shell: &str, wait: Duration) -> Vec<String> {
    ask_inheriting(shell, wait, &[]).await
}

/// `ask_within` as a daemon whose own environment holds `inherited` would
/// start it: the three variables are dropped from it all the same.
pub(crate) async fn ask_inheriting(shell: &str, wait: Duration, inherited: &[(&str, &str)]) -> Vec<String> {
    let mut command = tokio::process::Command::new(shell);
    command.envs(inherited.iter().copied());
    command.args(["-ilc", &payload()]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).process_group(0);
    for name in NAMES {
        command.env_remove(name);
    }
    let Ok(mut child) = command.spawn() else {
        tracing::warn!("couldn't ask the login shell about Claude variables");
        return Vec::new();
    };
    let group = child.id().map(|p| p as i32);
    let Some(mut stdout) = child.stdout.take() else { return Vec::new() };
    let read = tokio::time::timeout(wait, async {
        let mut out = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut out).await?;
        child.wait().await?;
        Ok::<_, std::io::Error>(out)
    })
    .await;
    let Ok(Ok(out)) = read else {
        crate::claude_sign_in::kill_group(group);
        tracing::warn!("couldn't ask the login shell about Claude variables");
        return Vec::new();
    };
    let said = String::from_utf8_lossy(&out);
    NAMES.iter().filter(|name| said.lines().any(|line| is_line_for(line, name))).map(|name| name.to_string()).collect()
}

/// Whether `line` is `name` and nothing else, bar one thing: an interactive
/// fish prints a terminal escape (`ESC[6 q`, the cursor shape) with no newline
/// before the first thing the payload prints, so the line may start with a
/// short escape sequence.
fn is_line_for(line: &str, name: &str) -> bool {
    match line.strip_suffix(name) {
        Some("") => true,
        Some(prefix) => prefix.starts_with('\x1b') && prefix.len() <= 32 && prefix.is_ascii(),
        None => false,
    }
}

/// One ask to make, through `shell`.
struct Job {
    shell: String,
}

#[derive(Default)]
struct State {
    names: Vec<String>,
    /// An ask is running; a newer trigger waits in `again`.
    running: bool,
    again: Option<Job>,
}

/// What the last ask found, and which trigger is the latest.
#[derive(Default)]
pub(crate) struct Exports {
    state: Mutex<State>,
    latest: AtomicU64,
    /// Run the next ask through this shell, in place of the login shell. For tests.
    #[cfg(test)]
    pub(crate) shell: Mutex<Option<String>>,
}

impl Exports {
    fn locked(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask again in the background through `shell`, or forget the answer when
    /// `ask_at_all` is false (no members). **Coalesced:** while an ask runs, a
    /// new trigger starts nothing and the task runs it once when it finishes,
    /// so a burst of member changes costs one shell and one more. An ask a
    /// newer trigger has passed never overwrites it.
    pub(crate) fn refresh(self: &std::sync::Arc<Self>, ask_at_all: bool, shell: String) {
        let turn = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let job = Job { shell };
        let mut state = self.locked();
        if !ask_at_all {
            state.names.clear();
            state.again = None;
            return;
        }
        if state.running {
            state.again = Some(job);
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        state.running = true;
        drop(state);
        let this = self.clone();
        runtime.spawn(async move {
            let (mut turn, mut job) = (turn, job);
            loop {
                let found = ask(&job.shell).await;
                let mut state = this.locked();
                if this.latest.load(Ordering::SeqCst) == turn {
                    state.names = found;
                }
                match state.again.take() {
                    Some(next) => {
                        turn = this.latest.load(Ordering::SeqCst);
                        job = next;
                    }
                    None => {
                        state.running = false;
                        break;
                    }
                }
            }
        });
    }

    /// Wait until no ask is running or waiting. For tests.
    #[cfg(test)]
    pub(crate) async fn settled(&self) {
        while self.locked().running {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn get(&self) -> Vec<String> {
        self.locked().names.clone()
    }
}

impl Service {
    /// Ask the owner's login shell again, in the background: on start, and
    /// after a member is added or removed. With no members there is nothing to
    /// warn of, and no shell is run.
    pub fn refresh_claude_key_exports(&self) {
        let has_members = self.store.members().is_ok_and(|m| !m.is_empty());
        #[allow(unused_mut)]
        let mut shell = farcooler_core::shell::login_shell();
        #[cfg(test)]
        if let Some(stand_in) = self.claude_logins.key_exports.shell.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            shell = stand_in;
        }
        self.claude_logins.key_exports.refresh(has_members, shell);
    }

    /// The Claude variables the owner's startup files set: what the last ask
    /// found, and nothing when the runner has no members now.
    pub(crate) fn claude_key_exports(&self) -> Vec<String> {
        if self.store.members().is_ok_and(|m| m.is_empty()) {
            return Vec::new();
        }
        self.claude_logins.key_exports.get()
    }

    /// `claude_key_exports` for `peer`: the owner's own, and nobody else's.
    /// The sentence speaks to the owner, and a member's device has no use
    /// for how the owner's shell is set up.
    pub(crate) fn claude_key_exports_for(&self, peer: &farcooler_transport::Peer) -> Vec<String> {
        match crate::people::person_of(&self.store, peer) {
            Ok(crate::people::Person::Owner) => self.claude_key_exports(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "claude_key_exports_tests.rs"]
mod tests;
