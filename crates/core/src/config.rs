//! User configuration, separate from runtime state.
//!
//! `~/.config/farcooler/config.toml`, on macOS as well as Linux. Deliberately
//! not `ProjectDirs::config_dir()`, which on macOS returns the same directory
//! as `data_dir()` — Apple does not separate the two — so it would put a
//! hand-edited file back among the sockets and the database on the one platform
//! where it is most likely to be edited by hand.
//!
//! And deliberately not inside `FARCOOLER_HOME`: everything there is generated
//! (`farcooler.db`, sockets, `install-id`, `worktrees`) in a 0700 directory
//! nobody browses, dotfiles repositories do not track, and which gets deleted
//! wholesale to recover from a corrupt database. One path on every platform
//! also means one dotfiles-tracked config works on a Mac and on every remote
//! host, which matters because those hosts run the daemon with no Mac app.

use std::path::{Path, PathBuf};

use crate::activity::Registry;

/// One `[adapters.<name>]` table.
///
/// Detection fields as well as launch fields, because `⌃B a` chooses the
/// adapter from the preset a pane was DETECTED as. An adapter belonging to a
/// preset nothing can ever detect could never be selected, so a genuinely new
/// agent has to say how to recognise it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConfigAdapter {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// `env = { KEY = "value" }` — a TOML table, hence a map.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub identity: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
    #[serde(default)]
    pub working: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFile {
    #[serde(default)]
    adapters: std::collections::BTreeMap<String, ConfigAdapter>,
}

/// Where the config file is, if the environment can say.
///
/// `$FARCOOLER_CONFIG` names the file itself; the others name its directory.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("FARCOOLER_CONFIG") {
        if !explicit.trim().is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(Path::new(&xdg).join("farcooler").join("config.toml"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        Path::new(&home)
            .join(".config")
            .join("farcooler")
            .join("config.toml"),
    )
}

/// The built-ins, overlaid with whatever the user configured.
pub fn load_registry() -> Registry {
    match config_path() {
        Some(path) => registry_from(&path),
        None => Registry::built_in(),
    }
}

/// The explicit-path form, so tests need no process-global environment.
pub fn registry_from(path: &Path) -> Registry {
    let mut registry = Registry::built_in();

    // Absent is the common case and not a condition worth reporting: almost
    // nobody has this file, and saying so on every daemon start would be noise.
    let Ok(text) = std::fs::read_to_string(path) else {
        return registry;
    };

    let parsed: ConfigFile = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            // Reported, then ignored. A typo in one table must not cost every
            // agent its chat mode, and silence would leave a user editing a
            // file that has no effect with nothing to tell them why.
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return registry;
        }
    };

    registry.merge(parsed.adapters.into_iter().collect());
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "farcooler-config-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn a_missing_file_leaves_the_built_ins_alone() {
        // The common case, and emphatically not an error: nobody has this file.
        let r = registry_from(std::path::Path::new("/nonexistent/config.toml"));
        assert!(r.chat_capable("codex"));
        assert_eq!(r.all().len(), Registry::built_in().all().len());
    }

    #[test]
    fn a_table_naming_a_built_in_replaces_its_adapter() {
        let dir = scratch("override");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[adapters.codex]\nprogram = \"npx\"\nargs = [\"-y\", \"@agentclientprotocol/codex-acp@1.1.9\"]\n",
        )
        .unwrap();
        let spec = registry_from(&path)
            .adapter("codex")
            .expect("still there")
            .clone();
        assert_eq!(
            spec.args,
            vec![
                "-y".to_string(),
                "@agentclientprotocol/codex-acp@1.1.9".to_string()
            ]
        );
    }

    #[test]
    fn a_table_naming_something_new_adds_an_agent() {
        let dir = scratch("add");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[adapters.my-agent]\nprogram = \"node\"\nargs = [\"/src/a.js\", \"--acp\"]\nidentity = [\"My Agent v\"]\n",
        )
        .unwrap();
        let r = registry_from(&path);
        assert!(r.chat_capable("my-agent"));
        // Detectable, or it could never be selected: the toggle picks an
        // adapter by the preset a pane was DETECTED as.
        assert_eq!(
            r.identify("node", "My Agent v1.2")
                .map(|x| x.preset.as_str()),
            Some("my-agent")
        );
    }

    #[test]
    fn a_malformed_file_does_not_take_chat_mode_down() {
        // A typo in one table must not cost every agent its chat. Built-ins
        // stand, and the parse failure is reported rather than swallowed.
        let dir = scratch("broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.codex\nprogram = ").unwrap();
        let r = registry_from(&path);
        assert!(r.chat_capable("codex"), "built-ins survive a broken file");
        assert!(r.chat_capable("claude"));
    }

    #[test]
    fn a_file_missing_a_required_key_fails_to_parse_and_built_ins_survive() {
        // `program` has no `#[serde(default)]`, so omitting it entirely fails
        // TOML deserialization for the whole file — this is the malformed-file
        // path, not the merge-time guard below. Distinct from
        // `an_entry_with_no_program_is_refused_rather_than_launched`, which
        // supplies a blank `program` so the file parses and the guard is the
        // thing actually being exercised.
        let dir = scratch("missing-key");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.broken]\nargs = [\"--acp\"]\n").unwrap();
        let r = registry_from(&path);
        assert!(
            !r.chat_capable("broken"),
            "never added: the file failed to parse"
        );
        assert!(
            r.chat_capable("codex"),
            "built-ins survive a file that fails to parse"
        );
        assert!(r.chat_capable("claude"));
    }

    #[test]
    fn an_entry_with_no_program_is_refused_rather_than_launched() {
        // `program` present but blank, so the file parses successfully and
        // deserializes into a `ConfigAdapter` — unlike a missing key, which
        // fails TOML deserialization before `Registry::merge` ever runs. This
        // is what actually exercises the `cfg.program.trim().is_empty()`
        // guard in `merge`: deleting that guard makes this test fail.
        let dir = scratch("noprogram");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[adapters.broken]\nprogram = \"   \"\nargs = [\"--acp\"]\n",
        )
        .unwrap();
        let r = registry_from(&path);
        assert!(
            !r.chat_capable("broken"),
            "an adapter with no program cannot start"
        );
    }

    #[test]
    fn overriding_an_adapter_does_not_wipe_the_built_ins_detection() {
        // The subtlest requirement here: TOML has no way to say "leave this
        // field alone", so a table supplying only `program` and `args` must
        // not blank out codex's identity, blocked and working strings —
        // `Registry::merge` only overwrites a detection field when the config
        // actually supplied one. Asserted through behaviour, matching how
        // every other test here checks the registry, rather than reaching
        // into `AgentRules` and checking the fields directly.
        let dir = scratch("override-detection");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[adapters.codex]\nprogram = \"npx\"\nargs = [\"-y\", \"@agentclientprotocol/codex-acp@1.1.9\"]\n",
        )
        .unwrap();
        let r = registry_from(&path);

        // Still found by its original identity marker.
        assert!(
            r.identify("codex-aarch64-a", "OpenAI Codex\n/model to change")
                .is_some()
        );

        // Still classifies through its original blocked/working signatures.
        let working =
            "\u{2022} Working (6s \u{2022} esc to interrupt) \u{b7} 1 background terminal running";
        assert_eq!(
            r.classify("codex-aarch64-a", working),
            farcooler_protocol::v1::AgentActivity::Working
        );
        let blocked = "\u{203a} 1. Update now\n  2. Skip\n  Press enter to continue";
        assert_eq!(
            r.classify("codex-aarch64-a", blocked),
            farcooler_protocol::v1::AgentActivity::Blocked
        );
    }
}
