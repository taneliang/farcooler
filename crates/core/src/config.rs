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
/// agent has to say how to recognize it.
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

/// One `[themes.<name>]` table.
///
/// Everything but the ground and the text is optional, so a file that only
/// wants a different background says only that and inherits the rest. A theme
/// is much more often a tweak to one that already works than sixteen colours
/// chosen from nothing.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConfigTheme {
    pub background: String,
    pub foreground: String,
    /// Whether the app's own surfaces go dark. Defaults to true rather than
    /// being guessed from the background: see `Theme::dark`.
    #[serde(default = "yes")]
    pub dark: bool,
    #[serde(default)]
    pub cursor: Option<String>,
    /// ANSI 0-7. Fewer or more than eight is a mistake worth reporting rather
    /// than padding out into colours nobody chose.
    #[serde(default)]
    pub normal: Option<Vec<String>>,
    /// ANSI 8-15.
    #[serde(default)]
    pub bright: Option<Vec<String>>,
}

fn yes() -> bool {
    true
}

/// The `[branches]` table.
///
/// A table rather than a top-level key, and that is not a style preference:
/// TOML puts a bare top-level scalar written below `[themes.paper]` inside THAT
/// table, so a `prefix = "elt/"` appended to the end of an existing config file
/// would silently become a theme's property and do nothing at all. In a file
/// whose entire purpose is being hand-edited, that is a trap worth one extra
/// line to avoid.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConfigBranches {
    /// Absent and empty are different answers. `None` means "say nothing, use
    /// the default"; `Some("")` means "no prefix at all", which is what makes
    /// opting out possible rather than indistinguishable from silence.
    #[serde(default)]
    pub prefix: Option<String>,
}

/// What a derived branch name gets in front of it when nothing says otherwise.
///
/// `feat/` because `NewWorkspaceSheet` on macOS already suggested exactly this,
/// so it is the default users can already see rather than a new one invented
/// here. The two creation paths disagreed — the sheet hardcoded this and the
/// task composer used nothing — and there cannot be a customizable default
/// until there is one default to customize.
pub const DEFAULT_BRANCH_PREFIX: &str = "feat/";

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFile {
    #[serde(default)]
    adapters: std::collections::BTreeMap<String, ConfigAdapter>,
    #[serde(default)]
    themes: std::collections::BTreeMap<String, ConfigTheme>,
    #[serde(default)]
    branches: ConfigBranches,
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

/// The themes this host defines, in name order.
///
/// Separate from `registry_from` because they answer to different callers: the
/// registry is how the daemon recognizes an agent, and this is what a client
/// asks for to fill its picker. Reading the file twice is cheaper than a type
/// that carries both to satisfy one of them.
pub fn themes_from(path: &Path) -> Vec<crate::theme::Theme> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let parsed: ConfigFile = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return Vec::new();
        }
    };

    parsed
        .themes
        .into_iter()
        .filter_map(|(name, spec)| match resolve_theme(&name, &spec) {
            Some(theme) => Some(theme),
            None => {
                // One theme at a time, never the file. The same rule adapters
                // follow: a typo in one table must not cost the user every
                // other table in it.
                tracing::warn!(theme = %name, "ignoring a theme with unreadable colours");
                None
            }
        })
        .collect()
}

/// The host's themes, found the same way the registry is.
pub fn load_themes() -> Vec<crate::theme::Theme> {
    match config_path() {
        Some(path) => themes_from(&path),
        None => Vec::new(),
    }
}

/// The prefix this host puts in front of a branch name derived from a task.
///
/// Read per call, like `themes_from` and for the same reason stated there: a few
/// hundred bytes of TOML parsed a handful of times per session, in exchange for
/// an edit taking effect without restarting the daemon. It matters more here
/// than it does for themes, because the machine settings editor writes this
/// file — a value cached at startup would not reflect its own writes.
///
/// Applied by the CLIENT, not by the daemon, because the task composer shows
/// you the branch it is about to create. A prefix added on this side would make
/// that preview a lie. The daemon still validates the finished name, which is
/// the check that actually protects git.
pub fn branch_prefix_from(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return DEFAULT_BRANCH_PREFIX.to_string();
    };
    let parsed: ConfigFile = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            // Reported, then ignored — the rule adapters and themes already
            // follow. A typo in one table must not cost the user every other
            // table in the file.
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return DEFAULT_BRANCH_PREFIX.to_string();
        }
    };
    match parsed.branches.prefix {
        // Trimmed, but otherwise literal: `elt-` is as valid a convention as
        // `elt/`, so no slash is added or removed. Only surrounding whitespace
        // goes, which is never intentional in a branch name.
        Some(p) => p.trim().to_string(),
        None => DEFAULT_BRANCH_PREFIX.to_string(),
    }
}

/// The host's branch prefix, found the same way the registry is.
pub fn load_branch_prefix() -> String {
    match config_path() {
        Some(path) => branch_prefix_from(&path),
        None => DEFAULT_BRANCH_PREFIX.to_string(),
    }
}

/// Turn one table into a theme, or `None` if any colour in it is unreadable.
///
/// Anything unspecified comes from the default theme rather than from black:
/// a file that sets only a background should look like the default with a
/// different background, not like an unlit room.
fn resolve_theme(name: &str, spec: &ConfigTheme) -> Option<crate::theme::Theme> {
    let base = crate::theme::default_theme();
    let mut ansi = base.ansi;

    if let Some(normal) = &spec.normal {
        if normal.len() != 8 {
            return None;
        }
        for (slot, text) in ansi[..8].iter_mut().zip(normal) {
            *slot = parse_hex(text)?;
        }
    }
    if let Some(bright) = &spec.bright {
        if bright.len() != 8 {
            return None;
        }
        for (slot, text) in ansi[8..].iter_mut().zip(bright) {
            *slot = parse_hex(text)?;
        }
    }

    let foreground = parse_hex(&spec.foreground)?;
    Some(crate::theme::Theme {
        name: name.to_string(),
        dark: spec.dark,
        background: parse_hex(&spec.background)?,
        foreground,
        // A theme that names no cursor gets its own text colour, which is what
        // every built-in but two does anyway.
        cursor: match &spec.cursor {
            Some(text) => parse_hex(text)?,
            None => foreground,
        },
        ansi,
    })
}

/// `#rrggbb`, or `rrggbb`. Deliberately nothing else — no names, no `rgb()`,
/// no three-digit shorthand — because every one of those is a second syntax to
/// get subtly wrong in a file nobody gets to test before the daemon reads it.
fn parse_hex(text: &str) -> Option<u32> {
    let body = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if body.len() != 6 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(body, 16).ok()
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
    fn a_host_theme_is_read_whole() {
        let dir = scratch("theme-whole");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "[themes.dim]\n",
                "dark = true\n",
                "background = \"#1b1d21\"\n",
                "foreground = \"#c8ccd4\"\n",
                "cursor = \"#e6c05c\"\n",
                "normal = [\"#000000\", \"#111111\", \"#222222\", \"#333333\",",
                " \"#444444\", \"#555555\", \"#666666\", \"#777777\"]\n",
                "bright = [\"#888888\", \"#999999\", \"#aaaaaa\", \"#bbbbbb\",",
                " \"#cccccc\", \"#dddddd\", \"#eeeeee\", \"#ffffff\"]\n",
            ),
        )
        .unwrap();

        let themes = themes_from(&path);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "dim");
        assert_eq!(themes[0].background, 0x1b_1d_21);
        assert_eq!(themes[0].cursor, 0xe6_c0_5c);
        assert_eq!(themes[0].ansi[0], 0x00_00_00);
        assert_eq!(themes[0].ansi[15], 0xff_ff_ff);
    }

    #[test]
    fn a_theme_that_sets_only_a_ground_inherits_the_rest() {
        // The common edit by far. Inheriting from black instead would make the
        // one-line tweak the hardest thing to write in the file.
        let dir = scratch("theme-partial");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[themes.paper]\ndark = false\nbackground = \"#fffdf7\"\nforeground = \"#2b2b2b\"\n",
        )
        .unwrap();

        let themes = themes_from(&path);
        assert_eq!(themes.len(), 1);
        assert!(!themes[0].dark);
        assert_eq!(themes[0].ansi, crate::theme::default_theme().ansi, "ansi inherited");
        assert_eq!(themes[0].cursor, 0x2b_2b_2b, "cursor follows the text when unsaid");
    }

    #[test]
    fn one_bad_theme_does_not_take_the_others_with_it() {
        // The rule adapters already follow. A user editing sixteen hex values
        // WILL mistype one, and losing every other theme over it would make
        // the file feel like it breaks at random.
        let dir = scratch("theme-bad");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "[themes.broken]\nbackground = \"#12345\"\nforeground = \"#ffffff\"\n",
                "[themes.fine]\nbackground = \"#101010\"\nforeground = \"#f0f0f0\"\n",
            ),
        )
        .unwrap();

        let themes = themes_from(&path);
        assert_eq!(themes.len(), 1, "only the good one survives");
        assert_eq!(themes[0].name, "fine");
    }

    #[test]
    fn a_short_colour_list_is_refused_rather_than_padded() {
        // Seven colours is a mistake, and quietly filling the eighth would put
        // a colour on screen that nobody chose and nobody can find in the file.
        let dir = scratch("theme-short");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "[themes.short]\nbackground = \"#101010\"\nforeground = \"#f0f0f0\"\n",
                "normal = [\"#000000\", \"#111111\", \"#222222\", \"#333333\",",
                " \"#444444\", \"#555555\", \"#666666\"]\n",
            ),
        )
        .unwrap();
        assert!(themes_from(&path).is_empty());
    }

    #[test]
    fn a_file_with_no_themes_yields_none_and_is_not_an_error() {
        let dir = scratch("theme-none");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.codex]\nprogram = \"npx\"\n").unwrap();
        assert!(themes_from(&path).is_empty());
        // And the adapters in the same file still parse.
        assert!(registry_from(&path).adapter("codex").is_some());
    }

    #[test]
    fn hex_is_accepted_with_or_without_the_hash_and_nothing_else() {
        assert_eq!(parse_hex("#a1b2c3"), Some(0xa1_b2_c3));
        assert_eq!(parse_hex("a1b2c3"), Some(0xa1_b2_c3));
        assert_eq!(parse_hex("  #A1B2C3  "), Some(0xa1_b2_c3));
        // Every one of these is a syntax somebody would reasonably expect to
        // work, and supporting them is a second parser to get wrong.
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("rebeccapurple"), None);
        assert_eq!(parse_hex("rgb(1,2,3)"), None);
        assert_eq!(parse_hex("#gggggg"), None);
    }

    // ---- the branch prefix ----

    #[test]
    fn a_branch_prefix_is_read_from_its_own_table() {
        let dir = scratch("branch-prefix");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[branches]\nprefix = \"elt/\"\n").unwrap();
        assert_eq!(branch_prefix_from(&path), "elt/");
    }

    #[test]
    fn a_prefix_written_below_another_table_still_belongs_to_branches() {
        // Why this is a table and not a top-level key, as a test. A bare
        // top-level scalar written here would have been swallowed by
        // `[themes.paper]` and done nothing, with nothing to say why.
        let dir = scratch("branch-after-theme");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "[themes.paper]\nbackground = \"#fffdf7\"\nforeground = \"#2b2b2b\"\n",
                "[branches]\nprefix = \"elt/\"\n",
            ),
        )
        .unwrap();
        assert_eq!(branch_prefix_from(&path), "elt/");
        // And the theme in the same file still parses.
        assert_eq!(themes_from(&path).len(), 1);
    }

    #[test]
    fn no_config_file_yields_the_default_prefix() {
        assert_eq!(
            branch_prefix_from(std::path::Path::new("/nonexistent/config.toml")),
            DEFAULT_BRANCH_PREFIX
        );
    }

    #[test]
    fn a_file_with_no_branches_table_yields_the_default() {
        let dir = scratch("branch-none");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.codex]\nprogram = \"npx\"\n").unwrap();
        assert_eq!(branch_prefix_from(&path), DEFAULT_BRANCH_PREFIX);
    }

    #[test]
    fn an_empty_prefix_opts_out_rather_than_falling_back() {
        // The distinction that makes this customizable at all: "" is a choice,
        // and treating it as "unset" would make opting out impossible.
        let dir = scratch("branch-empty");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[branches]\nprefix = \"\"\n").unwrap();
        assert_eq!(branch_prefix_from(&path), "");
    }

    #[test]
    fn a_prefix_is_trimmed_but_otherwise_taken_literally() {
        // `elt-` is as valid a convention as `elt/`, so no slash is added or
        // removed. Only surrounding whitespace goes.
        let dir = scratch("branch-literal");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[branches]\nprefix = \"  elt-  \"\n").unwrap();
        assert_eq!(branch_prefix_from(&path), "elt-");
    }

    #[test]
    fn a_malformed_file_does_not_take_the_prefix_down_with_it() {
        let dir = scratch("branch-broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[branches\nprefix = ").unwrap();
        assert_eq!(branch_prefix_from(&path), DEFAULT_BRANCH_PREFIX);
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
        // actually supplied one. Asserted through behavior, matching how
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
