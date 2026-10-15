//! Claude's mailbox as a way to deliver a message (ov-563): the hook entries
//! that arm it, and where its directories are. What a mailbox is, and the
//! waiter that empties it, are `farcooler_agent_hooks::mailbox` and
//! `farcooler hook wait`; whether a message goes this way is
//! `watch::answer_wake::deliver`.
//!
//! claude's settings file is one for every claude pane, so the entries can't
//! name a pane. They name the runner's mailbox directory, and the waiter finds
//! its session's own under it from the session id in the hook payload.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// How long claude may let the waiter run: a day, and Stop starts the next.
const WAITER_TIMEOUT_S: u64 = 86_400;

/// What claude prints before a message (`rewakeMessage`, and `rewakeSummary`
/// for the terminal line). Both fields are `@internal` in claude 2.1.300, the
/// version they were measured live on (ov-563): a later claude may rename or
/// drop them, and then the message arrives under claude's default wording,
/// "Stop hook blocking error from command …", which is still delivered.
/// What claude prints before a message: its hook feedback would otherwise
/// read as an error ("Stop hook blocking error"), and the model may take an
/// error for something to ignore. Measured live on 2.1.300 (`rewakeMessage`).
pub const REWAKE_PREFIX: &str = "Message from Far Cooler:";

/// The runner's mailboxes, beside its hook socket.
pub fn root_of(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("mailbox")
}

/// Where each dropped message records the pane and session it went to
/// (`deliver::retract_dropped`), beside the mailboxes rather than in them: a
/// mailbox is removed when its session goes.
pub fn routes_of(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("mailbox-routes")
}

/// The waiter exits 42 when it has a message (`hook_wait::WOKE`), and this is
/// what the hook command makes of that: 2, which claude reads as a wake, for
/// 42 alone. Anything else is 0. An older `farcooler` that doesn't know `hook
/// wait` exits 2 with a usage error, and claude would wake the model with it.
pub fn wrapped(command: &str) -> String {
    format!("{command}; c=$?; [ \"$c\" = 42 ] && exit 2; exit 0")
}

/// How long a mailbox whose session no pane names is left before it's
/// removed: a session is bound to its pane a moment after it starts.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(600);

/// How long a taken message, or a route, is kept.
const KEPT_FOR: std::time::Duration = std::time::Duration::from_secs(3600);

/// Remove what a finished session left. A mailbox whose session `current`
/// doesn't know (after `/clear` or a resume moved the pane to another, or the
/// pane closed), and that has been quiet for `STALE_AFTER`, goes with its
/// waiter, which leaves once its directory is gone (`hook_wait`). Taken
/// messages older than `KEPT_FOR` go from the rest, and so do old routes.
pub fn sweep(runtime_dir: &Path, current: &dyn Fn(&str) -> bool) {
    let old = |path: &Path, than: std::time::Duration| {
        std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age > than)
    };
    // Sessions a message was dropped for and not yet settled.
    let routed: Vec<String> = std::fs::read_dir(routes_of(runtime_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|route| std::fs::read_to_string(route.path()).ok())
        .filter_map(|text| text.trim().split_once(' ').map(|(_, session)| session.to_string()))
        .collect();
    let holds_a_file = |dir: &Path| std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    for entry in std::fs::read_dir(root_of(runtime_dir)).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_dir() {
            continue;
        }
        for taken in std::fs::read_dir(path.join("taken")).into_iter().flatten().flatten() {
            if old(&taken.path(), KEPT_FOR) {
                let _ = std::fs::remove_file(taken.path());
            }
        }
        // A directory's own mtime doesn't move when a message is dropped or
        // taken (those change `new/` and `taken/`), so quiet is judged by what
        // is inside: nothing pending, nothing just taken, nothing routed here.
        let busy = holds_a_file(&path.join("new")) || holds_a_file(&path.join("taken")) || routed.contains(&name);
        if !current(&name) && !busy && old(&path, STALE_AFTER) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    for route in std::fs::read_dir(routes_of(runtime_dir)).into_iter().flatten().flatten() {
        if old(&route.path(), KEPT_FOR) {
            let _ = std::fs::remove_file(route.path());
        }
    }
}

/// Add the waiter to claude's SessionStart (so a fresh, idle session is armed)
/// and Stop (so each turn's end arms the next), beside the entries already
/// there. `socket` is the runner's hook socket; the mailboxes sit by it.
pub fn arm(hooks: &mut serde_json::Map<String, Value>, socket: &Path) {
    let Some(runtime) = socket.parent() else { return };
    let command = wrapped(&format!(
        "{} hook wait --mailbox {}",
        crate::service::shell_quote(&crate::hook_install::binary_path()),
        crate::service::shell_quote(&root_of(runtime).display().to_string()),
    ));
    let entry = json!({ "hooks": [ {
        "type": "command",
        "command": command,
        "asyncRewake": true,
        "timeout": WAITER_TIMEOUT_S,
        "rewakeMessage": REWAKE_PREFIX,
        "rewakeSummary": "Message from Far Cooler",
    } ] });
    for event in ["SessionStart", "Stop"] {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            list.push(entry.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SessionStart and Stop each carry the waiter beside the report hook,
    /// as an `asyncRewake` hook with a day to run, and no other event does.
    #[test]
    fn the_waiter_is_armed_on_session_start_and_stop_only() {
        let settings = crate::hook_install::claude_settings(Path::new("/run/fc/h.sock"));
        let hooks = settings["hooks"].as_object().unwrap();
        for (event, entries) in hooks {
            let waiters: Vec<&Value> =
                entries.as_array().unwrap().iter().flat_map(|e| e["hooks"].as_array().unwrap()).filter(|h| h["asyncRewake"] == true).collect();
            if event == "SessionStart" || event == "Stop" {
                assert_eq!(entries.as_array().unwrap().len(), 2, "{event}: the report hook stays");
                assert_eq!(waiters.len(), 1, "{event}");
                let waiter = waiters[0];
                let command = waiter["command"].as_str().unwrap();
                assert!(command.contains(" hook wait --mailbox '/run/fc/mailbox'; c=$?;"), "{waiter}");
                assert!(command.ends_with("exit 0"), "{waiter}");
                assert_eq!(waiter["timeout"], WAITER_TIMEOUT_S);
                assert_eq!(waiter["rewakeMessage"], REWAKE_PREFIX);
                assert!(waiter["rewakeSummary"].is_string());
            } else {
                assert!(waiters.is_empty(), "{event} carries a waiter");
            }
        }
    }

    /// A path with a space (and a quote) is one word to the shell.
    #[test]
    fn a_path_with_a_space_survives_the_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("it's a dir").join("exit 42");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "#!/bin/sh\nexit 42\n").unwrap();
        std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let command = wrapped(&crate::service::shell_quote(&program.display().to_string()));
        assert_eq!(std::process::Command::new("sh").arg("-c").arg(command).status().unwrap().code(), Some(2));
    }

    /// Only the waiter's 42 is a wake: the exit of a binary that doesn't know
    /// `hook wait` (a usage error, 2) and any failure are not.
    #[test]
    fn only_a_forty_two_wakes_claude() {
        let exit = |code: i32| {
            std::process::Command::new("sh").arg("-c").arg(wrapped(&format!("sh -c 'exit {code}'"))).status().unwrap().code()
        };
        assert_eq!(exit(42), Some(2));
        for other in [0, 1, 2, 3, 41, 43, 127] {
            assert_eq!(exit(other), Some(0), "{other}");
        }
    }

    /// Stale mailboxes go, live and recent ones stay; old taken messages and
    /// routes go.
    #[test]
    fn a_sweep_removes_what_a_finished_session_left() {
        let dir = tempfile::tempdir().unwrap();
        let root = root_of(dir.path());
        let ago = |path: &Path, secs: u64| {
            let file = std::fs::File::open(path).unwrap();
            file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs)).unwrap();
        };
        for name in ["live", "gone-old", "gone-new"] {
            std::fs::create_dir_all(root.join(name).join("taken")).unwrap();
        }
        std::fs::write(root.join("live/taken/old"), "x").unwrap();
        std::fs::write(root.join("live/taken/fresh"), "x").unwrap();
        std::fs::create_dir_all(routes_of(dir.path())).unwrap();
        std::fs::write(routes_of(dir.path()).join("old"), "x").unwrap();
        std::fs::write(routes_of(dir.path()).join("fresh"), "x").unwrap();
        ago(&root.join("gone-old"), 7200);
        ago(&root.join("live/taken/old"), 7200);
        ago(&routes_of(dir.path()).join("old"), 7200);
        // A stale mailbox holding a pending message, one holding a message just
        // taken, and one a route still names, are all left: removing any would
        // let the wake type what a waiter delivers or already delivered.
        for name in ["pending", "justtaken", "routed"] {
            std::fs::create_dir_all(root.join(name).join("new")).unwrap();
            std::fs::create_dir_all(root.join(name).join("taken")).unwrap();
            ago(&root.join(name), 7200);
        }
        std::fs::write(root.join("pending/new/0001-a"), "x").unwrap();
        std::fs::write(root.join("justtaken/taken/0001-a"), "x").unwrap();
        std::fs::write(routes_of(dir.path()).join("live-route"), "11111111-1111-1111-1111-111111111111 routed").unwrap();
        sweep(dir.path(), &|session| session == "live");
        for kept in ["pending", "justtaken", "routed"] {
            assert!(root.join(kept).is_dir(), "{kept} was removed");
        }
        assert!(root.join("live").is_dir() && root.join("gone-new").is_dir(), "a live or recent one stays");
        assert!(!root.join("gone-old").exists(), "a stale one goes");
        assert!(!root.join("live/taken/old").exists() && root.join("live/taken/fresh").exists());
        assert!(!routes_of(dir.path()).join("old").exists() && routes_of(dir.path()).join("fresh").exists());
    }
}
