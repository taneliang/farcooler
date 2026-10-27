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
//! runner, which matters because those runners run the daemon with no Mac app.

use std::path::{Path, PathBuf};

use crate::activity::Registry;

mod read_only;
pub use read_only::{load_read_only_folders, read_only_folders_from};

/// One `[adapters.<name>]` table: how to recognize an agent on a screen.
///
/// The name is chat mode's, which launched the agent from the same table
/// (`program`, `args`, `env`, `backend`) until it was retired (ov-377). Those
/// keys are read past rather than refused, so a file written for it still
/// teaches the runner its agents.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConfigAdapter {
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
/// is much more often a tweak to one that already works than sixteen colors
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
    /// than padding out into colors nobody chose.
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
/// `feat/` because the Mac's New Worktree sheet already suggested exactly this,
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
///
/// A test binary that names none gets a file of its own under the temp
/// directory, never this machine's (`test_binary_config`).
pub fn config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("FARCOOLER_CONFIG") {
        if !explicit.trim().is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if let Some(isolated) = test_binary_config() {
        return Some(isolated);
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
            // agent its recognition, and silence would leave a user editing a
            // file that has no effect with nothing to tell them why.
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return registry;
        }
    };

    registry.merge(parsed.adapters.into_iter().collect());
    registry
}

/// The themes this runner defines, in name order.
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
                tracing::warn!(theme = %name, "ignoring a theme with unreadable colors");
                None
            }
        })
        .collect()
}

/// The runner's themes, found the same way the registry is.
pub fn load_themes() -> Vec<crate::theme::Theme> {
    match config_path() {
        Some(path) => themes_from(&path),
        None => Vec::new(),
    }
}

/// The prefix this runner puts in front of a branch name derived from a task.
///
/// Read per call, like `themes_from` and for the same reason stated there: a few
/// hundred bytes of TOML parsed a handful of times per session, in exchange for
/// an edit taking effect without restarting the daemon. It matters more here
/// than it does for themes, because the runner settings editor writes this
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

/// A config file of the process's own when it is a test binary: one cargo
/// put in `target/<profile>/deps/`, where only test harnesses live, and
/// never an installed or built `farcooler` or `farcoolerd`.
///
/// `cfg(test)` can't do this: it is set only for the crate under test, and
/// a daemon or client test reads this crate's `config_path` from a build of
/// it without the flag. One that ran with no `FARCOOLER_CONFIG` read and
/// wrote this machine's shared config.toml, whose `[agents] projector`
/// (ov-372) changed what those tests saw. Debug builds only.
fn test_binary_config() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    (parent.file_name()? == "deps")
        .then(|| crate::scratch::root().join("config.toml"))
}

/// The runner's branch prefix, found the same way the registry is.
pub fn load_branch_prefix() -> String {
    match config_path() {
        Some(path) => branch_prefix_from(&path),
        None => DEFAULT_BRANCH_PREFIX.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------
//
// The riskiest code in this crate, and the reason it is written the way it is:
// this file is documented above as the one path a dotfiles repository tracks,
// hand-edited, on a Mac and on every remote runner. A settings screen that
// rewrote it would be a data-loss bug wearing a feature's clothes.
//
// Three properties, each load-bearing:
//
// 1. **Format-preserving.** `toml_edit` mutates a parsed document in place, so
//    comments, blank lines, key order and spacing survive everywhere the edit
//    does not touch. `toml::to_string` would return a file that parses to the
//    same values and reads like a different file.
// 2. **Atomic.** A sibling temp file and a `rename`, so a crash or a full disk
//    cannot leave a truncated dotfile behind.
// 3. **Surgical.** One table per call. That is what makes a concurrent hand
//    edit to a DIFFERENT table survive untouched, which is the realistic
//    concurrency case here — two edits to the same table race, and the last
//    writer wins.
//
// Default umask, deliberately not 0600: `push.json` is owner-only because it
// holds a bearer token, and this file is the opposite — meant to be readable,
// tracked, and copied between runners.

/// Read a document for editing, or `None` if the file exists and is malformed.
///
/// `None` is not the same as absent. An absent file becomes an empty document
/// and gets written; a MALFORMED one is refused, because the reader above
/// tolerates a broken file by ignoring it, and a writer that overwrote one
/// would turn somebody's typo into lost work.
fn document_for_edit(path: &Path) -> std::io::Result<Option<toml_edit::DocumentMut>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    Ok(text.parse::<toml_edit::DocumentMut>().ok())
}

/// The refusal `document_for_edit` produces, as an error a caller can report.
fn malformed(path: &Path) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "{} is not valid TOML; fix it by hand before editing it from an app, \
             so an unparseable file is never overwritten",
            path.display()
        ),
    )
}

/// Write a document back, atomically.
fn save(path: &Path, document: &toml_edit::DocumentMut) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Beside the target, not in a temp directory: `rename` is only atomic
    // within a filesystem, and `/tmp` is routinely a different one.
    let temp = path.with_extension("toml.new");
    std::fs::write(&temp, document.to_string())?;
    std::fs::rename(&temp, path)
}

/// Set `[branches] prefix`, creating the file if it does not exist.
pub fn write_branch_prefix(path: &Path, prefix: &str) -> std::io::Result<()> {
    let Some(mut doc) = document_for_edit(path)? else { return Err(malformed(path)) };
    doc.entry("branches")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "`branches` exists but is not a table",
            )
        })?
        .insert("prefix", toml_edit::value(prefix.trim()));
    save(path, &doc)
}

/// Write one `[themes.<name>]` table, replacing it if it is already there.
///
/// Every color is written, including the sixteen ANSI ones, even though the
/// reader treats `normal` and `bright` as optional. An editor that omitted them
/// would produce a theme whose ANSI colors silently follow whatever the
/// default theme's are — including through a future change to that default.
pub fn write_theme(path: &Path, theme: &crate::theme::Theme) -> std::io::Result<()> {
    let Some(mut doc) = document_for_edit(path)? else { return Err(malformed(path)) };

    let mut table = toml_edit::Table::new();
    table.insert("dark", toml_edit::value(theme.dark));
    table.insert("background", toml_edit::value(hex(theme.background)));
    table.insert("foreground", toml_edit::value(hex(theme.foreground)));
    table.insert("cursor", toml_edit::value(hex(theme.cursor)));
    table.insert("normal", colors(&theme.ansi[..8]));
    table.insert("bright", colors(&theme.ansi[8..]));

    themes_table(&mut doc, path)?.insert(&theme.name, toml_edit::Item::Table(table));
    save(path, &doc)
}

/// Remove one `[themes.<name>]` table. A name that is not there is success.
///
/// Idempotent because this is how "revert to default" works: the table goes
/// away and the compiled-in theme takes over. Writing the built-in's values
/// back instead would freeze today's defaults into the user's file, so a later
/// shipped improvement would never reach them.
pub fn delete_theme(path: &Path, name: &str) -> std::io::Result<()> {
    let Some(mut doc) = document_for_edit(path)? else { return Err(malformed(path)) };
    let Some(themes) = doc.get_mut("themes").and_then(|t| t.as_table_mut()) else {
        return Ok(());
    };
    themes.remove(name);
    // An empty `[themes]` husk left behind is not wrong, but it is litter in a
    // file people read.
    if themes.is_empty() {
        doc.remove("themes");
    }
    save(path, &doc)
}

/// The `[themes]` table to insert into, created if absent.
///
/// `set_implicit` so the parent renders as `[themes.dim]` rather than an empty
/// `[themes]` header followed by it — which is what a person writing this file
/// by hand would produce.
fn themes_table<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    path: &Path,
) -> std::io::Result<&'a mut toml_edit::Table> {
    named_table(doc, "themes", path)
}

fn named_table<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    name: &str,
    path: &Path,
) -> std::io::Result<&'a mut toml_edit::Table> {
    let entry = doc.entry(name).or_insert(toml_edit::Item::Table({
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        t
    }));
    entry.as_table_mut().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("`{name}` in {} exists but is not a table", path.display()),
        )
    })
}

/// `#rrggbb`, the form `parse_hex` reads and the form the docs use.
fn hex(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xFF_FF_FF)
}

fn colors(rgb: &[u32]) -> toml_edit::Item {
    strings(&rgb.iter().map(|c| hex(*c)).collect::<Vec<_>>())
}

fn strings(values: &[String]) -> toml_edit::Item {
    let mut array = toml_edit::Array::new();
    for value in values {
        array.push(value.as_str());
    }
    toml_edit::value(array)
}

/// Turn one table into a theme, or `None` if any color in it is unreadable.
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
        // A theme that names no cursor gets its own text color, which is what
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

    /// A test binary reads its own config file, not this machine's.
    #[test]
    fn a_test_binary_never_reads_this_machines_config() {
        let isolated = test_binary_config().expect("a test harness, under target/*/deps");
        assert!(isolated.starts_with(crate::scratch::root()), "{isolated:?}");
        if std::env::var_os("FARCOOLER_CONFIG").is_none() {
            assert_eq!(config_path(), Some(isolated));
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        crate::scratch::dir(&format!("config-{tag}"))
    }

    #[test]
    fn a_runner_theme_is_read_whole() {
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
    fn a_short_color_list_is_refused_rather_than_padded() {
        // Seven colors is a mistake, and quietly filling the eighth would put
        // a color on screen that nobody chose and nobody can find in the file.
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
        assert_eq!(registry_from(&path).all().len(), Registry::built_in().all().len());
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

    // ---- writing ----

    /// A file with the shapes a real hand-edited one has: a leading comment, a
    /// comment inside a table, blank lines, and an inline table.
    const HAND_EDITED: &str = concat!(
        "# My Far Cooler config. Tracked in dotfiles.\n",
        "\n",
        "[branches]\n",
        "prefix = \"elt/\"\n",
        "\n",
        "[adapters.my-agent]\n",
        "# npx because the package moves and pinning it here breaks less often.\n",
        "program = \"npx\"\n",
        "args = [\"-y\", \"my-agent-acp\"]\n",
        "env = { MY_KEY = \"secret\" }\n",
        "\n",
        "[themes.paper]\n",
        "dark = false\n",
        "background = \"#fffdf7\"\n",
        "foreground = \"#2b2b2b\"\n",
    );

    #[test]
    fn a_write_leaves_every_byte_outside_its_own_table_alone() {
        // THE test for this feature. config.toml is documented as the one path a
        // dotfiles repository tracks and a person hand-edits, so a settings
        // screen that reformatted it — dropping the comments, reordering the
        // keys — would be a data-loss bug wearing a feature's clothes.
        //
        // Asserts on the whole file rather than on a parse of it, because a
        // parse cannot see the thing being protected.
        let dir = scratch("write-preserves");
        let path = dir.join("config.toml");
        std::fs::write(&path, HAND_EDITED).unwrap();

        write_branch_prefix(&path, "feat/").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        assert!(after.starts_with("# My Far Cooler config. Tracked in dotfiles.\n"));
        assert!(
            after.contains("# npx because the package moves and pinning it here breaks less often."),
            "a comment inside an untouched table must survive:\n{after}"
        );
        assert!(after.contains("env = { MY_KEY = \"secret\" }"), "inline tables stay inline");
        assert!(after.contains("[themes.paper]"), "untouched tables stay");
        assert!(after.contains("prefix = \"feat/\""), "and the edit landed");
        assert!(!after.contains("elt/"), "the old value is gone");

        // Everything except the one line that changed is byte-identical.
        let before_lines: Vec<_> = HAND_EDITED.lines().filter(|l| !l.contains("prefix")).collect();
        let after_lines: Vec<_> = after.lines().filter(|l| !l.contains("prefix")).collect();
        assert_eq!(before_lines, after_lines);
    }

    #[test]
    fn a_first_write_creates_the_directory_and_a_minimal_file() {
        // Almost nobody has this file, per the module doc, so a first write is
        // the common case and must not need the directory to exist.
        let dir = scratch("write-creates");
        let path = dir.join("nested").join("deeper").join("config.toml");
        write_branch_prefix(&path, "elt/").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(branch_prefix_from(&path), "elt/");
        assert!(text.contains("[branches]"));
        assert!(!text.contains("themes"), "only the table being set is written:\n{text}");
    }

    #[test]
    fn a_theme_round_trips_through_the_writer_and_the_reader() {
        let dir = scratch("write-theme");
        let path = dir.join("config.toml");
        let mut theme = crate::theme::default_theme();
        theme.name = "mine".into();
        theme.background = 0x11_22_33;
        theme.ansi[9] = 0xAB_CD_EF;

        write_theme(&path, &theme).unwrap();
        let read = themes_from(&path);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "mine");
        assert_eq!(read[0].background, 0x11_22_33);
        assert_eq!(read[0].ansi[9], 0xAB_CD_EF);
        assert_eq!(read[0].ansi, theme.ansi, "all sixteen are written, not just the changed one");
    }

    #[test]
    fn writing_a_theme_twice_replaces_rather_than_duplicating() {
        let dir = scratch("write-theme-twice");
        let path = dir.join("config.toml");
        let mut theme = crate::theme::default_theme();
        theme.name = "mine".into();
        write_theme(&path, &theme).unwrap();
        theme.background = 0x99_88_77;
        write_theme(&path, &theme).unwrap();

        let read = themes_from(&path);
        assert_eq!(read.len(), 1, "one table, not two");
        assert_eq!(read[0].background, 0x99_88_77);
    }

    #[test]
    fn deleting_a_theme_that_is_not_there_is_success() {
        // Idempotent because it is how "revert to default" works, and a revert
        // on something already reverted must not be an error the UI has to
        // explain.
        let dir = scratch("delete-absent");
        let path = dir.join("config.toml");
        std::fs::write(&path, HAND_EDITED).unwrap();
        delete_theme(&path, "never-existed").unwrap();
        delete_theme(&path.with_file_name("nothing.toml"), "paper").unwrap();
    }

    #[test]
    fn deleting_the_last_theme_leaves_no_empty_husk() {
        let dir = scratch("delete-last");
        let path = dir.join("config.toml");
        let mut theme = crate::theme::default_theme();
        theme.name = "only".into();
        write_theme(&path, &theme).unwrap();
        delete_theme(&path, "only").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("themes"), "an empty [themes] header is litter:\n{text}");
        assert!(themes_from(&path).is_empty());
    }

    #[test]
    fn a_malformed_file_is_refused_rather_than_overwritten() {
        // The reader tolerates a broken file by ignoring it. A writer that
        // overwrote one would turn somebody's typo into lost work — and this is
        // a file people edit by hand, so typos are the expected state.
        let dir = scratch("write-malformed");
        let path = dir.join("config.toml");
        let broken = "[adapters.codex\nprogram = ";
        std::fs::write(&path, broken).unwrap();

        assert!(write_branch_prefix(&path, "elt/").is_err());
        assert!(write_theme(&path, &crate::theme::default_theme()).is_err());
        assert!(delete_theme(&path, "paper").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken, "untouched");
    }

    #[test]
    fn a_write_leaves_no_temp_file_behind() {
        let dir = scratch("write-temp");
        let path = dir.join("config.toml");
        write_branch_prefix(&path, "elt/").unwrap();
        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");
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
        assert!(r.rules_for_command("codex").is_some());
        assert_eq!(r.all().len(), Registry::built_in().all().len());
    }

    #[test]
    fn a_table_written_for_chat_mode_keeps_its_detection() {
        // The exact shape the old adapter editor wrote: launch fields, and all
        // four detection arrays empty. Empty has to mean "leave the built-in's
        // alone", because if it meant "clear them" the agent would stop being
        // recognized in a pane. The launch fields are chat mode's (ov-377),
        // read past without a word.
        let dir = scratch("native-keeps-detection");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[adapters.codex]\nbackend = \"native\"\nprogram = \"codex\"\nargs = []\n\
             commands = []\nidentity = []\nblocked = []\nworking = []\n",
        )
        .unwrap();

        let r = registry_from(&path);
        let rules = r.all().iter().find(|x| x.preset == "codex").expect("codex survives").clone();
        assert!(!rules.commands.is_empty(), "an emptied array must not clear detection");
        assert!(!rules.identity.is_empty(), "same for the screen signatures");
        assert!(
            r.rules_for_command("codex").is_some(),
            "a codex pane still has to be recognized as codex"
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
        // Detectable by what it draws, the reason a table names it at all.
        assert_eq!(
            r.identify("node", "My Agent v1.2")
                .map(|x| x.preset.as_str()),
            Some("my-agent")
        );
    }

    #[test]
    fn a_malformed_file_does_not_take_detection_down() {
        // A typo in one table must not cost every agent its detection.
        // Built-ins stand, and the parse failure is reported rather than
        // swallowed.
        let dir = scratch("broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.codex\nprogram = ").unwrap();
        let r = registry_from(&path);
        assert!(r.rules_for_command("codex").is_some(), "built-ins survive a broken file");
        assert!(r.rules_for_command("claude").is_some());
    }

    #[test]
    fn a_table_with_only_chat_modes_keys_adds_an_agent_nothing_can_detect() {
        // `program` was required while it launched a chat. It's read past now
        // (ov-377), so a file with one parses, and the agent it names is in
        // the table with nothing to recognize it by.
        let dir = scratch("launch-only");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.mine]\nprogram = \"node\"\nargs = [\"--acp\"]\n").unwrap();
        let r = registry_from(&path);
        assert_eq!(r.all().len(), Registry::built_in().all().len() + 1, "the file parsed");
        assert!(r.rules_for_command("codex").is_some(), "and the built-ins are untouched");
    }

    #[test]
    fn a_table_without_detection_does_not_wipe_the_built_ins() {
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
