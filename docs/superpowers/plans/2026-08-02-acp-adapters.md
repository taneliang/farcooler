# ACP Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chat mode works for codex, opencode and cursor as well as claude, with user-supplied adapters via `~/.config/farcooler/config.toml`.

**Architecture:** The two hand-maintained lists that must agree — `activity::RULES` (what is recognized) and `CHAT_CAPABLE` (what can be hosted) — merge into one `Registry` in `crates/core`. An agent is chat-capable exactly when its registry entry carries an `AdapterSpec`, so the lists can no longer drift. The `Registry` is a value held by `Service`, not a process global. The daemon passes `--preset` to the shim, and both resolve the same registry from the same config file.

**Tech Stack:** Rust (tokio, serde, toml, clap), Swift/SwiftUI for the Mac app, tmux for pane capture.

**Spec:** `docs/superpowers/specs/2026-08-02-acp-adapters-design.md`

## Global Constraints

- Adapter packages, verified against npm on 2026-08-02. Use these exact strings:
  - claude → `npx` `["-y", "@agentclientprotocol/claude-agent-acp"]`
  - codex → `npx` `["-y", "@agentclientprotocol/codex-acp"]`
  - opencode → `opencode` `["acp"]`
  - cursor → `npx` `["-y", "cursor-agent-acp"]`
- **Never** use a `@zed-industries/*` ACP package. All are deprecated in favour of `@agentclientprotocol/*`. `@zed-industries/codex-acp` is deprecated and stalled at 0.16.0 against the live 1.1.9.
- Config path resolution, in order: `$FARCOOLER_CONFIG` → `$XDG_CONFIG_HOME/farcooler/config.toml` → `~/.config/farcooler/config.toml`. Never `ProjectDirs::config_dir()` (identical to `data_dir()` on macOS), never inside `FARCOOLER_HOME`.
- A missing config file is not an error. A malformed one is reported and then ignored in favour of built-ins.
- No `#[ignore]` tests. The workspace has zero today and this work adds none.
- Comments explain *why*, referencing the concrete failure they prevent. Match the surrounding style in `activity.rs` and `service.rs`.
- Run `cargo fmt` and `cargo clippy --workspace --all-targets` before each commit.

---

### Task 1: Registry types and built-ins in core

**Files:**
- Modify: `crates/core/src/activity.rs` (`AgentRules` at 35-63, `RULES` at 74-121, `rules_for_command` at 129-139, `identify` at 149-155, `describe` at 162-182, `classify` at ~245)
- Modify: `crates/core/Cargo.toml`
- Modify: `Cargo.toml` (workspace — add `toml`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct AdapterSpec { pub program: String, pub args: Vec<String>, pub env: BTreeMap<String, String> }`
  - `pub struct AgentRules { pub preset: String, pub commands: Vec<String>, pub identity: Vec<String>, pub blocked: Vec<String>, pub working: Vec<String>, pub adapter: Option<AdapterSpec> }`
  - `pub struct Registry { rules: Vec<AgentRules> }`
  - `Registry::built_in() -> Registry`
  - `Registry::rules_for_command(&self, command: &str) -> Option<&AgentRules>`
  - `Registry::identify(&self, command: &str, screen: &str) -> Option<&AgentRules>`
  - `Registry::describe(&self, command: &str, screen: &str) -> String`
  - `Registry::classify(&self, command: &str, screen: &str) -> AgentActivity`
  - `Registry::adapter(&self, preset: &str) -> Option<&AdapterSpec>`
  - `Registry::chat_capable(&self, preset: &str) -> bool`
  - `pub fn plain_text(screen: &str) -> String` stays a free function, unchanged.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/core/src/activity.rs`:

```rust
#[test]
fn an_agent_is_chat_capable_exactly_when_it_has_an_adapter() {
    // The bug this replaces: `CHAT_CAPABLE` was a second list maintained by
    // hand beside the rules, and the two disagreed. Derived, they cannot.
    let r = Registry::built_in();
    for preset in ["claude", "codex", "opencode", "cursor"] {
        assert!(r.chat_capable(preset), "{preset} ships an adapter");
        assert!(r.adapter(preset).is_some());
    }
    assert!(!r.chat_capable("zsh"));
    assert!(r.adapter("zsh").is_none());
}

#[test]
fn every_adapter_belongs_to_an_agent_that_can_be_detected() {
    // An adapter on a preset nothing can identify is dead config: `⌃B a`
    // chooses the adapter from the DETECTED preset, so an entry no screen and
    // no process name can reach could never be selected.
    let r = Registry::built_in();
    for rules in r.all() {
        if rules.adapter.is_none() {
            continue;
        }
        assert!(
            !rules.commands.is_empty() || !rules.identity.is_empty(),
            "{} has an adapter but nothing that can detect it",
            rules.preset
        );
    }
}

#[test]
fn no_built_in_adapter_uses_a_deprecated_zed_package() {
    // Every `@zed-industries/*` ACP package has moved to
    // `@agentclientprotocol/*`. The old codex one is deprecated and stalled at
    // 0.16.0 while the live package is at 1.1.9 — the identical trap already
    // recorded for the Claude adapter.
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        for arg in &spec.args {
            assert!(
                !arg.contains("@zed-industries/"),
                "{} uses a deprecated package: {arg}",
                rules.preset
            );
        }
    }
}

#[test]
fn opencode_needs_no_npm_package() {
    // Its ACP mode is a native subcommand, which is the whole reason it is
    // preferred over the `opencode-acp` package on npm.
    let spec = Registry::built_in().adapter("opencode").expect("opencode ships an adapter").clone();
    assert_eq!(spec.program, "opencode");
    assert_eq!(spec.args, vec!["acp".to_string()]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p farcooler-core activity 2>&1 | tail -20`
Expected: FAIL — `cannot find type Registry in this scope`.

- [ ] **Step 3: Add the workspace `toml` dependency**

In the root `Cargo.toml`, in `[workspace.dependencies]`, beside `serde`:

```toml
toml = "0.8"
```

In `crates/core/Cargo.toml` under `[dependencies]`:

```toml
serde.workspace = true
toml.workspace = true
```

`toml` is unused until Task 2; adding both here keeps the manifest edits in one place.

- [ ] **Step 4: Convert `AgentRules` to owned fields and add `AdapterSpec`**

Replace the `AgentRules` struct (currently at `crates/core/src/activity.rs:34-63`), keeping every existing doc comment on the fields it still has:

```rust
/// How to launch an agent's ACP adapter.
///
/// Three real shapes have to fit: an npm package run through `npx`, a native
/// subcommand on an installed binary, and a script with flags. The previous
/// representation — a bare program with no arguments — could express none of
/// them, so a user-supplied adapter silently lost everything after the program
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Set in the adapter's environment before it starts. For secrets and
    /// endpoints an agent needs and Far Cooler has no opinion about.
    ///
    /// A map, not a list of pairs: TOML writes this as `env = { KEY = "v" }`,
    /// which is a table and will not deserialize into tuples. `BTreeMap` rather
    /// than `HashMap` so the order an adapter's environment is built in is
    /// stable, and a test can assert on it.
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct AgentRules {
    /// What to call this agent.
    pub preset: String,

    /// Process-name PREFIXES, as `pane_current_command` reports them.
    ///
    /// Prefixes, not exact names, because tmux truncates the field: codex
    /// arrives as `codex-aarch64-a`. And necessary but never sufficient —
    /// Claude Code renames itself to its version (`2.1.220`) and cursor-agent
    /// runs as plain `node`, which is far too generic to claim. Screen identity
    /// is what actually carries this.
    pub commands: Vec<String>,

    /// Screen text that means "this IS this agent".
    ///
    /// Furniture the agent always draws, never a phrase a user could type, so a
    /// shell echoing "do you want to proceed?" is not promoted to an agent.
    pub identity: Vec<String>,

    /// Waiting on the user.
    ///
    /// The list that has to be right. A missed blocked state is a notification
    /// that never arrives, which is the one failure that makes the whole
    /// feature pointless — so these are deliberately generous.
    pub blocked: Vec<String>,

    /// Actively doing something.
    pub working: Vec<String>,

    /// How to host this agent as a native chat, when Far Cooler can.
    ///
    /// `None` means recognized-but-terminal-only, which is a real and honest
    /// state rather than a gap: an agent with no adapter renders as the TUI it
    /// is. This field replaces the separate `CHAT_CAPABLE` list in the daemon,
    /// which was maintained by hand beside these rules and disagreed with them.
    pub adapter: Option<AdapterSpec>,
}
```

- [ ] **Step 5: Replace `RULES` with `Registry`**

`RULES` cannot stay a `const` — a `const` cannot hold a `String`. Replace the `pub const RULES: &[AgentRules] = &[…]` block (currently `crates/core/src/activity.rs:65-121`) with a `Registry` whose constructor builds the same table. Keep the existing doc comment above `RULES` verbatim on `built_in`, and keep every per-entry comment.

```rust
/// A short-hand so the built-in table reads as it did when it was a `const`.
fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn npx(package: &str) -> Option<AdapterSpec> {
    Some(AdapterSpec {
        program: "npx".to_string(),
        args: vec!["-y".to_string(), package.to_string()],
        env: Default::default(),
    })
}

/// Every agent Far Cooler knows, and how to host the ones it can.
///
/// One table rather than two. Recognition and hostability were separate lists
/// maintained by hand, and they drifted: codex was recognized in a pane and
/// absent from the chat list, so `⌃B a` on a codex pane did nothing and said
/// nothing. Hostability is now a field on the entry, so there is no second list
/// to disagree with.
#[derive(Debug, Clone)]
pub struct Registry {
    rules: Vec<AgentRules>,
}

impl Registry {
    /// The built-in rules.
    ///
    /// Every signature below was read off a running agent rather than guessed.
    /// The first version of this file was guesswork and it matched no real
    /// screen.
    ///
    /// There is deliberately no `idle` list. An agent that is identified, not
    /// blocked and not working IS idle, and requiring positive idle furniture
    /// meant a version bump renaming a footer left an agent stuck on `unknown`
    /// — never reaching `done`, never notifying.
    pub fn built_in() -> Self {
        Registry {
            rules: vec![
                AgentRules {
                    preset: "claude".to_string(),
                    commands: s(&["claude"]),
                    identity: s(&[
                        "? for shortcuts",
                        "Claude Code",
                        "auto-accept edits",
                        "esc to interrupt",
                    ]),
                    blocked: s(&[
                        "Do you want to",
                        "Do you want me to",
                        "❯ 1. Yes",
                        "1. Yes, and don't ask again",
                        // The footer under every approval prompt.
                        "Esc to cancel · Tab to amend",
                        "[y/n]",
                        "(y/N)",
                    ]),
                    working: s(&["esc to interrupt", "Thinking…"]),
                    adapter: npx("@agentclientprotocol/claude-agent-acp"),
                },
                AgentRules {
                    preset: "codex".to_string(),
                    // Truncated by tmux to `codex-aarch64-a`, hence the prefix.
                    commands: s(&["codex"]),
                    identity: s(&["OpenAI Codex", "/model to change"]),
                    blocked: s(&[
                        // Codex draws every choice as a numbered list under a `›` marker.
                        "\u{203a} 1.",
                        "Press enter to continue",
                        "Allow command",
                        "Do you want to",
                        "[y/n]",
                        "(y/N)",
                    ]),
                    working: s(&["esc to interrupt", "Working ("]),
                    // NOT `@zed-industries/codex-acp`: npm reports it deprecated
                    // and replaced by this one, and it stalled at 0.16.0 against
                    // this package's 1.1.9. The same rename that caught the
                    // Claude adapter.
                    adapter: npx("@agentclientprotocol/codex-acp"),
                },
                AgentRules {
                    preset: "cursor".to_string(),
                    // cursor-agent runs as `node`, which cannot be claimed —
                    // matching it would label every node process a coding
                    // agent. Kept for installs that expose a real name; screen
                    // identity is what finds it here.
                    commands: s(&["cursor-agent"]),
                    identity: s(&["Cursor Agent", "cursor-agent", "Press any key to sign in"]),
                    blocked: s(&["Do you want to", "Allow?", "[y/n]", "(y/N)", "\u{203a} 1."]),
                    working: s(&["esc to interrupt", "Generating"]),
                    // Best-effort, and knowingly so. `cursor-agent-acp` is
                    // third-party, sits at 0.1.1 and was last published in
                    // September 2025; there is no first-party alternative and
                    // no `@agentclientprotocol/cursor-acp`. It is shipped
                    // because failing to start is already handled honestly —
                    // `AdapterMissing` keeps the pane alive and says terminal
                    // mode is unaffected — and a toggle that might work beats
                    // one that certainly does not.
                    adapter: npx("cursor-agent-acp"),
                },
            ],
        }
    }

    /// Every entry, for callers that need the whole table.
    pub fn all(&self) -> &[AgentRules] {
        &self.rules
    }

    /// How to host this preset as a chat, if Far Cooler can.
    pub fn adapter(&self, preset: &str) -> Option<&AdapterSpec> {
        self.rules.iter().find(|r| r.preset == preset).and_then(|r| r.adapter.as_ref())
    }

    /// Whether Far Cooler can render this agent as a chat.
    ///
    /// Derived, never listed. This being a second hand-maintained list is what
    /// let it disagree with the rules it was supposed to match.
    pub fn chat_capable(&self, preset: &str) -> bool {
        self.adapter(preset).is_some()
    }
}
```

Note: `opencode` is deliberately absent from this table. Its detection rules are read off a real screen in Task 5, following this file's own rule that signatures are observed and not guessed.

- [ ] **Step 6: Move the four lookup functions onto `Registry`**

Convert `rules_for_command`, `identify`, `describe` and `classify` from free functions to methods, keeping every doc comment and every line of logic. Only the signature and the `RULES` reference change. For example:

```rust
impl Registry {
    /// Which agent, if any, is running in a pane.
    ///
    /// Matched on the foreground process. That is the only thing that answers
    /// the question honestly: a terminal launched as a shell in which someone
    /// typed `claude` IS a Claude Code terminal, and one launched as an agent
    /// that has since exited to a prompt is not.
    pub fn rules_for_command(&self, command: &str) -> Option<&AgentRules> {
        // The program, not the whole command line: identity comes from what was
        // run, and `claude --model opus` is still claude.
        let first = command.split_whitespace().next().unwrap_or(command);
        let name = first.rsplit('/').next().unwrap_or(first).trim();
        if name.is_empty() {
            return None;
        }
        // Prefix, because tmux truncates: `codex-aarch64-a` must match `codex`.
        self.rules.iter().find(|r| r.commands.iter().any(|c| name.starts_with(c.as_str())))
    }
}
```

Apply the same treatment to `identify`, `describe` and `classify`. `plain_text` stays a free `pub fn` — it takes no rules.

- [ ] **Step 7: Update the existing tests in this file**

The four tests at `crates/core/src/activity.rs:499-530` iterate `RULES`; change them to iterate `Registry::built_in().all()`. Every other test calls `classify`, `describe` or `rules_for_command` directly; give each a `let r = Registry::built_in();` and call through it. Change no assertion — the existing expectations must all still hold, and any that stops holding is a regression, not a test to update.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p farcooler-core 2>&1 | tail -20`
Expected: PASS, including every pre-existing test in the file.

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy -p farcooler-core --all-targets
git add Cargo.toml crates/core/Cargo.toml crates/core/src/activity.rs
git commit -m "feat(core): one registry for recognition and hostability

Recognition and chat-hostability were two lists maintained by hand, and they
drifted — codex was recognized in a pane and missing from the chat list, so
the toggle did nothing. Hostability is now a field, so there is no second
list to disagree with."
```

---

### Task 2: Load `~/.config/farcooler/config.toml`

**Files:**
- Create: `crates/core/src/config.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod config;`)
- Modify: `crates/core/src/activity.rs` (add `Registry::merge`)

**Interfaces:**
- Consumes: `Registry`, `AgentRules`, `AdapterSpec` from Task 1.
- Produces:
  - `pub fn config_path() -> Option<PathBuf>`
  - `pub fn load_registry() -> Registry` — built-ins overlaid with the config file, never failing
  - `Registry::merge(&mut self, entries: Vec<(String, ConfigAdapter)>)`
  - `pub struct ConfigAdapter { pub program: String, pub args: Vec<String>, pub env: Vec<(String,String)>, pub commands: Vec<String>, pub identity: Vec<String>, pub blocked: Vec<String>, pub working: Vec<String> }`

- [ ] **Step 1: Write the failing tests**

Create `crates/core/src/config.rs` with only the test module first:

```rust
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
        let spec = registry_from(&path).adapter("codex").expect("still there").clone();
        assert_eq!(spec.args, vec!["-y".to_string(), "@agentclientprotocol/codex-acp@1.1.9".to_string()]);
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
        assert_eq!(r.identify("node", "My Agent v1.2").map(|x| x.preset.as_str()), Some("my-agent"));
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
    fn an_entry_with_no_program_is_refused_rather_than_launched() {
        let dir = scratch("noprogram");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[adapters.broken]\nargs = [\"--acp\"]\n").unwrap();
        let r = registry_from(&path);
        assert!(!r.chat_capable("broken"), "an adapter with no program cannot start");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p farcooler-core config 2>&1 | tail -20`
Expected: FAIL — `cannot find function registry_from`.

- [ ] **Step 3: Implement `config.rs`**

```rust
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

use crate::activity::{AdapterSpec, AgentRules, Registry};

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
    Some(Path::new(&home).join(".config").join("farcooler").join("config.toml"))
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
    let Ok(text) = std::fs::read_to_string(path) else { return registry };

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
```

- [ ] **Step 4: Implement `Registry::merge` in `activity.rs`**

```rust
impl Registry {
    /// Overlay configured adapters onto the built-in table.
    ///
    /// By preset name: a name that already exists replaces that entry's
    /// adapter and any detection field the user supplied; a new name appends a
    /// new entry. Appended rather than prepended so a user cannot accidentally
    /// shadow a built-in's detection by adding a broad `identity` string.
    pub fn merge(&mut self, entries: Vec<(String, crate::config::ConfigAdapter)>) {
        for (preset, cfg) in entries {
            // An adapter with no program cannot start, and offering the toggle
            // for one would produce exactly the silent failure this work exists
            // to remove.
            if cfg.program.trim().is_empty() {
                tracing::warn!(%preset, "ignoring a configured adapter with no program");
                continue;
            }
            let spec = AdapterSpec { program: cfg.program, args: cfg.args, env: cfg.env };
            match self.rules.iter_mut().find(|r| r.preset == preset) {
                Some(existing) => {
                    existing.adapter = Some(spec);
                    // Only what was actually supplied. An empty list in TOML is
                    // indistinguishable from an absent one, and wiping a
                    // built-in's identity strings would make it undetectable.
                    if !cfg.commands.is_empty() {
                        existing.commands = cfg.commands;
                    }
                    if !cfg.identity.is_empty() {
                        existing.identity = cfg.identity;
                    }
                    if !cfg.blocked.is_empty() {
                        existing.blocked = cfg.blocked;
                    }
                    if !cfg.working.is_empty() {
                        existing.working = cfg.working;
                    }
                }
                None => self.rules.push(AgentRules {
                    preset,
                    commands: cfg.commands,
                    identity: cfg.identity,
                    blocked: cfg.blocked,
                    working: cfg.working,
                    adapter: Some(spec),
                }),
            }
        }
    }
}
```

Add `pub mod config;` to `crates/core/src/lib.rs`, and `use crate::activity::Registry;` plus `registry_from` in scope for the tests.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p farcooler-core 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy -p farcooler-core --all-targets
git add crates/core/src/config.rs crates/core/src/lib.rs crates/core/src/activity.rs
git commit -m "feat(core): adapters configurable from ~/.config/farcooler/config.toml

Config, not state: FARCOOLER_HOME holds a database, sockets and worktrees in
a directory nobody browses and dotfiles do not track. A malformed file is
reported and ignored so one typo cannot cost every agent its chat."
```

---

### Task 3: Wire the registry into the daemon

**Files:**
- Modify: `crates/daemon/src/service.rs` (`CHAT_CAPABLE` 123-159, `Service` struct 161-178, `open_in` 210-222, call site 808, call site 1195, refusal 1215-1229, tests 1737-1748)
- Modify: `crates/daemon/src/watch.rs` (289, 302, 314, 323-329, 332)

**Interfaces:**
- Consumes: `Registry`, `farcooler_core::config::load_registry` from Tasks 1-2.
- Produces: `Service::registry(&self) -> &Registry`. `farcooler_daemon::service::chat_capable` is **deleted**; callers use `service.registry().chat_capable(preset)`.

- [ ] **Step 1: Write the failing test**

Replace the existing test at `crates/daemon/src/service.rs:1737-1748` (which asserts against `CHAT_CAPABLE`) with:

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p farcooler-daemon recognition_and 2>&1 | tail -20`
Expected: FAIL — `Registry` does not exist yet from this crate's view until Task 1 lands, or `CHAT_CAPABLE` still compiles alongside it.

- [ ] **Step 3: Give `Service` a registry and delete `CHAT_CAPABLE`**

Delete the `CHAT_CAPABLE` const and the `chat_capable` free function (`service.rs:123-129` and `156-159`). Add to the `Service` struct, after `root`:

```rust
    /// Which agents are recognized, and which can be hosted as a chat.
    ///
    /// Held rather than re-read per call, for the same reason `root` is: the
    /// config file and the environment that locates it are process-global, and
    /// a service that consulted them on every call could be moved out from
    /// under itself — which is what happens when two tests run in parallel.
    registry: farcooler_core::activity::Registry,
```

In `open_in`, before the `Ok(Self { … })`:

```rust
        let registry = farcooler_core::config::load_registry();
```

and add `registry` to the struct literal. Add the accessor beside `agents()`:

```rust
    /// Which agents are recognized, and which can be hosted as a chat.
    pub fn registry(&self) -> &farcooler_core::activity::Registry {
        &self.registry
    }
```

- [ ] **Step 4: Update the four daemon call sites**

`service.rs:808` — `farcooler_core::activity::identify(&pane.command, &screen)` becomes `self.registry.identify(&pane.command, &screen)`, and the `.is_some_and(|rules| chat_capable(rules.preset))` on line 809 becomes `.is_some_and(|rules| self.registry.chat_capable(&rules.preset))`.

`service.rs:1195` — `farcooler_core::activity::identify(…)` becomes `self.registry.identify(…)`; `.map(|rules| rules.preset.to_string())` becomes `.map(|rules| rules.preset.clone())`.

`service.rs:1215-1229` — replace the `match harness.as_deref()` arms, keeping the entire existing comment block above them verbatim:

```rust
        if pane_mode == models::PaneMode::Agent {
            match harness.as_deref() {
                Some(h) if self.registry.chat_capable(h) => {}
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
```

`watch.rs:289, 302, 314, 323-329, 332` — every `activity::describe`, `activity::classify`, `activity::identify` and `crate::service::chat_capable` call becomes a method on `self.service.registry()`. Bind it once at the top of the loop body to keep the lines readable:

```rust
        let registry = self.service.registry();
```

- [ ] **Step 5: Run the whole daemon suite**

Run: `cargo test -p farcooler-daemon 2>&1 | tail -30`
Expected: PASS. Every existing test must still pass; `set_pane_mode` behavior is unchanged for claude and still refuses a shell.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy -p farcooler-daemon --all-targets
git add crates/daemon/src/service.rs crates/daemon/src/watch.rs
git commit -m "refactor(daemon): chat capability is derived, not listed

CHAT_CAPABLE was a second list beside the rules it was supposed to match.
Service holds a Registry the way it already holds root, and for the same
reason: process-global state moves out from under a service that re-reads it."
```

---

### Task 4: The shim takes `--preset`, not `--adapter`

**Files:**
- Modify: `crates/cli/src/main.rs` (`AgentHost` variant 124-136, dispatch 648-649)
- Modify: `crates/cli/src/agent_host.rs` (`default_adapter` 59-71, `claude_executable` 73-95, `run` 97-124, test 476-481)
- Modify: `crates/daemon/src/service.rs` (the `format!` at 1171-1176)

**Interfaces:**
- Consumes: `farcooler_core::config::load_registry`, `AdapterSpec` from Tasks 1-2.
- Produces: `agent_host::run(terminal: Uuid, socket: PathBuf, worktree: PathBuf, session: Option<String>, preset: Option<String>) -> Fallible`. `default_adapter()` is **deleted**.

- [ ] **Step 1: Write the failing tests**

In `crates/cli/src/agent_host.rs`, replace the test at 476-481:

```rust
#[test]
fn the_preset_chooses_the_adapter() {
    let registry = farcooler_core::activity::Registry::built_in();
    let spec = resolve(&registry, Some("codex")).expect("codex has an adapter");
    assert_eq!(spec.program, "npx");
    assert!(spec.args.iter().any(|a| a == "@agentclientprotocol/codex-acp"));
    // The maintained package, not the renamed one. npm reports
    // `@zed-industries/codex-acp` deprecated and it stalled at 0.16.0.
    assert!(!spec.args.iter().any(|a| a.contains("@zed-industries/")));
}

#[test]
fn an_unknown_preset_resolves_to_nothing_rather_than_to_claude() {
    // The failure this prevents: the shim knew one adapter, so a codex pane
    // switched to chat started a brand new CLAUDE session in the same worktree
    // and drew that, with nothing saying the agent had been swapped.
    let registry = farcooler_core::activity::Registry::built_in();
    assert!(resolve(&registry, Some("nothing-by-this-name")).is_none());
    assert!(resolve(&registry, None).is_none());
}

#[test]
fn only_claude_gets_the_claude_executable_treatment() {
    // It probes the filesystem for Claude Code specifically. Applying it to
    // every adapter would set a Claude variable in codex's environment.
    assert!(wants_claude_executable("claude"));
    assert!(!wants_claude_executable("codex"));
    assert!(!wants_claude_executable("opencode"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p farcooler-cli agent_host 2>&1 | tail -20`
Expected: FAIL — `cannot find function resolve`.

- [ ] **Step 3: Replace `default_adapter` with `resolve`**

Delete `default_adapter` (59-71). Add:

```rust
/// The adapter for a preset, or nothing.
///
/// Nothing rather than a default, deliberately. The shim used to know exactly
/// one adapter, so a pane hosting any other agent got Claude: a codex pane
/// switched to chat started a brand new Claude session in the same worktree and
/// rendered that, with nothing anywhere saying the agent had been swapped.
/// Refusing is a small disappointment; a different agent wearing the same pane
/// is a much larger one.
pub fn resolve(
    registry: &farcooler_core::activity::Registry,
    preset: Option<&str>,
) -> Option<farcooler_core::activity::AdapterSpec> {
    registry.adapter(preset?).cloned()
}

/// Whether this preset needs `CLAUDE_CODE_EXECUTABLE` resolved for it.
///
/// Claude alone. `claude_executable` probes the filesystem for Claude Code, so
/// applying it generally would put a Claude-specific variable into every other
/// agent's environment.
pub fn wants_claude_executable(preset: &str) -> bool {
    preset == "claude"
}
```

- [ ] **Step 4: Rework `run`**

Replace the head of `run` (97-124), keeping the rest of the function untouched:

```rust
pub async fn run(
    terminal: Uuid,
    socket: PathBuf,
    worktree: PathBuf,
    session: Option<String>,
    preset: Option<String>,
) -> Fallible {
    let registry = farcooler_core::config::load_registry();
    let Some(spec) = resolve(&registry, preset.as_deref()) else {
        // Reached only if the daemon's check and this one disagree, which
        // means a config file changed between them. Loud, because the symptom
        // is otherwise an empty pane.
        let name = preset.unwrap_or_else(|| "this pane".to_string());
        println!(
            "farcooler: no ACP adapter is configured for `{name}`.\n\
             Switch this terminal back to terminal mode — it needs no adapter \
             and is unaffected."
        );
        std::future::pending::<()>().await;
        unreachable!()
    };
    let (program, args) = (spec.program.clone(), spec.args.clone());

    for (key, value) in &spec.env {
        // SAFETY: set before any thread is spawned that reads the environment,
        // and this process exists to host exactly one adapter.
        unsafe { std::env::set_var(key, value) };
    }

    if preset.as_deref().is_some_and(wants_claude_executable) {
        if let Some(executable) = claude_executable() {
            // SAFETY: as above.
            unsafe { std::env::set_var("CLAUDE_CODE_EXECUTABLE", executable) };
        }
    }
```

- [ ] **Step 5: Rename the CLI flag**

In `crates/cli/src/main.rs`, in the `AgentHost` variant, replace `adapter: Option<String>` with:

```rust
        /// Which agent this pane hosts. The shim resolves it to an adapter
        /// through the same registry and config file the daemon used, so the
        /// two can never disagree about what a preset means — and so a program
        /// and its argument vector never have to survive tmux's shell quoting.
        #[arg(long)]
        preset: Option<String>,
```

and at 648-649:

```rust
        Command::AgentHost { terminal, socket, worktree, session, preset } => {
            agent_host::run(terminal, socket, worktree, session, preset).await
        }
```

- [ ] **Step 6: Pass `--preset` from the daemon**

In `crates/daemon/src/service.rs`, the `harness` is computed at 1195, *after* the command string is built at 1171. Move the `harness` computation above the `let command = match pane_mode {` block so the preset is available, then in the `PaneMode::Agent` arm:

```rust
                let preset = harness
                    .as_deref()
                    .map(|h| format!(" --preset {}", shell_quote(h)))
                    .unwrap_or_default();
                format!(
                    "{} agent-host --terminal {id} --socket {} --worktree {}{session}{preset}",
                    shell_quote(&binary),
                    shell_quote(&socket),
                    shell_quote(&ws.worktree_path),
                )
```

Quoted like every other interpolation here, for the reason the existing comment gives: tmux hands this string to a shell.

- [ ] **Step 7: Run both suites**

Run: `cargo test -p farcooler-cli -p farcooler-daemon 2>&1 | tail -30`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cargo fmt && cargo clippy --workspace --all-targets
git add crates/cli/src/main.rs crates/cli/src/agent_host.rs crates/daemon/src/service.rs
git commit -m "feat(cli): the shim hosts whichever agent the pane was running

--preset rather than --adapter: the shim resolves through the same registry
and config file the daemon used, so neither a meaning nor an argument vector
has to survive tmux's shell quoting. An unknown preset resolves to nothing
rather than to Claude."
```

---

### Task 5: Detection rules for opencode, and verified cursor rules

**Files:**
- Modify: `crates/core/src/activity.rs` (`Registry::built_in`)

**Interfaces:**
- Consumes: `Registry` from Task 1.
- Produces: an `opencode` entry with observed `identity`, `blocked` and `working` strings.

This task is empirical. `activity.rs` records that the first version of that file was guesswork and matched no real screen, and the cursor entry still carries an `UNVERIFIED` warning because that install could not get past sign-in. `opencode`, `cursor-agent`, `codex` and `claude` are all installed on this machine, so nothing here gets guessed.

- [ ] **Step 1: Capture a real opencode screen in each state**

```bash
tmux -L rules new-session -d -s cap -x 120 -y 40 -c /tmp 'opencode'
sleep 20 && tmux -L rules capture-pane -p -t cap > /tmp/opencode-idle.txt
tmux -L rules send-keys -t cap 'list the files here, then run git status' Enter
sleep 4 && tmux -L rules capture-pane -p -t cap > /tmp/opencode-working.txt
sleep 12 && tmux -L rules capture-pane -p -t cap > /tmp/opencode-blocked.txt
tmux -L rules kill-server
```

Read all three. Also record what tmux reports as the process name, which is what `commands` must prefix-match:

```bash
tmux -L rules new-session -d -s cap -c /tmp 'opencode'
sleep 15 && tmux -L rules display-message -p -t cap '#{pane_current_command}'
tmux -L rules kill-server
```

- [ ] **Step 2: Do the same for cursor-agent**

Identical procedure with `cursor-agent`. The existing entry's `identity`, `blocked` and `working` strings were never observed; confirm or correct each against the captures, and delete the `UNVERIFIED` comment only for the lists actually verified.

- [ ] **Step 3: Write the tests from the captures**

Add to `mod tests` in `activity.rs`, using **real substrings from the captured files**, not the illustrative ones below:

```rust
#[test]
fn opencode_states_come_from_its_real_screen() {
    let r = Registry::built_in();
    let idle = "<paste from /tmp/opencode-idle.txt>";
    assert_eq!(r.classify("opencode", idle), Idle);
    let working = "<paste from /tmp/opencode-working.txt>";
    assert_eq!(r.classify("opencode", working), Working);
    let blocked = "<paste from /tmp/opencode-blocked.txt>";
    assert_eq!(r.classify("opencode", blocked), Blocked);
}

#[test]
fn opencode_is_recognized_by_process_name() {
    let r = Registry::built_in();
    assert_eq!(r.describe("<observed pane_current_command>", ""), "opencode");
}
```

- [ ] **Step 4: Run to verify they fail**

Run: `cargo test -p farcooler-core opencode 2>&1 | tail -20`
Expected: FAIL — no opencode entry, so `describe` returns the process name.

- [ ] **Step 5: Add the opencode entry**

Insert into `Registry::built_in`'s vector, after codex, with the observed strings. `identity` must be furniture opencode always draws and never a phrase a user could type — the rule the whole table follows.

```rust
                AgentRules {
                    preset: "opencode".to_string(),
                    commands: s(&["opencode"]),
                    identity: s(&[/* observed */]),
                    blocked: s(&[/* observed */]),
                    working: s(&[/* observed */]),
                    // Native ACP subcommand, so no npm package to be renamed
                    // or deprecated out from under it — unlike every other
                    // adapter in this table.
                    adapter: Some(AdapterSpec {
                        program: "opencode".to_string(),
                        args: vec!["acp".to_string()],
                        env: Default::default(),
                    }),
                },
```

- [ ] **Step 6: Run the full core suite**

Run: `cargo test -p farcooler-core 2>&1 | tail -20`
Expected: PASS, including `every_adapter_belongs_to_an_agent_that_can_be_detected` from Task 1.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy -p farcooler-core --all-targets
git add crates/core/src/activity.rs
git commit -m "feat(core): opencode recognized, cursor's rules verified

Read off running agents, not guessed — the first version of this file was
guesswork and matched no real screen. cursor's UNVERIFIED marker goes only
for the lists actually confirmed against a capture."
```

---

### Task 6: Contract tests against the real adapters

**Files:**
- Create: `crates/core/tests/adapters.rs`
- Modify: `.github/workflows/ci.yml` (the Rust job, beside the existing tmux install step at line 52)

**Interfaces:**
- Consumes: `Registry::built_in`, `AdapterSpec` from Task 1; the opencode entry from Task 5.
- Produces: nothing consumed by later tasks.

No `#[ignore]`. The workspace has none, and a test that skips itself on the machines where it matters is `#[ignore]` wearing a different hat.

- [ ] **Step 1: Write the package-health test**

```rust
//! The built-in adapters, checked against the world they depend on.
//!
//! Both tests here exist because of one concrete near-miss: every current
//! search result and third-party document names `@zed-industries/codex-acp`
//! for codex, npm reports it deprecated in favour of
//! `@agentclientprotocol/codex-acp`, and it stalled at 0.16.0 against the live
//! 1.1.9. No amount of unit testing can catch that — only asking the outside
//! world can.

use farcooler_core::activity::Registry;

/// The npm package an adapter runs, if it runs one.
fn npm_package(spec: &farcooler_core::activity::AdapterSpec) -> Option<&str> {
    if spec.program != "npx" {
        return None;
    }
    spec.args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str())
}

#[test]
fn no_built_in_adapter_is_deprecated_on_npm() {
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        let Some(package) = npm_package(spec) else { continue };

        let out = std::process::Command::new("npm")
            .args(["view", package, "deprecated"])
            .output()
            .expect("npm must be installed to verify the adapters");
        assert!(
            out.status.success(),
            "{} names a package npm cannot resolve: {package}",
            rules.preset
        );
        let notice = String::from_utf8_lossy(&out.stdout);
        assert!(
            notice.trim().is_empty(),
            "{} uses a DEPRECATED package {package}: {}",
            rules.preset,
            notice.trim()
        );
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p farcooler-core --test adapters no_built_in_adapter_is_deprecated 2>&1 | tail -20`
Expected: PASS. If it fails naming a package, the adapter table is wrong — fix the table, not the test.

- [ ] **Step 3: Write the handshake test**

Appended to the same file. It asserts `initialize` and deliberately not `session/new`: the handshake proves the adapter exists, starts and speaks ACP, which is the question. `session/new` additionally needs the underlying CLI authenticated and able to reach a model provider, which CI cannot have, and a failure there would say nothing about whether Far Cooler wired the adapter up correctly.

```rust
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Spawn an adapter, send `initialize`, and read until it answers.
fn initialize(spec: &farcooler_core::activity::AdapterSpec) -> Result<serde_json::Value, String> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", spec.program))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": 1, "clientCapabilities": {} }
    });
    let mut stdin = child.stdin.take().expect("piped");
    writeln!(stdin, "{request}").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;

    let stdout = child.stdout.take().expect("piped");
    let mut lines = BufReader::new(stdout).lines();
    let answer = loop {
        match lines.next() {
            // Adapters may log before answering; skip anything that is not the
            // response to id 1.
            Some(Ok(line)) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                if value.get("id") == Some(&serde_json::json!(1)) {
                    break Ok(value);
                }
            }
            Some(Err(e)) => break Err(e.to_string()),
            None => break Err("the adapter closed without answering initialize".to_string()),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    answer
}

#[test]
fn every_built_in_adapter_completes_an_acp_handshake() {
    // A cold `npx` fetches a package on first use, so this is slow the first
    // time and fast afterwards. A missing program is a FAILURE, not a skip: on
    // a machine without the agent installed, silently passing would mean the
    // one test that can catch a broken adapter never runs where it matters.
    let mut failures = Vec::new();
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        match initialize(spec) {
            Ok(v) if v.get("result").is_some() => {}
            Ok(v) => failures.push(format!("{}: {v}", rules.preset)),
            Err(e) => failures.push(format!("{}: {e}", rules.preset)),
        }
    }
    assert!(failures.is_empty(), "adapters that could not handshake:\n{}", failures.join("\n"));
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p farcooler-core --test adapters every_built_in 2>&1 | tail -30`
Expected: PASS for all four. If cursor's stale `cursor-agent-acp` cannot handshake, that is real information: report it and ask before changing the table — the design accepted it as best-effort on the understanding that it works.

- [ ] **Step 5: Install opencode in CI**

In `.github/workflows/ci.yml`, in the Rust job beside the existing `Install tmux` step:

```yaml
      - name: Install opencode
        run: npm install -g opencode-ai
```

The other three adapters are `npx` packages, which need only node and a network — both of which the runners already have. opencode's is a native subcommand on a binary, so the binary has to exist.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/core/tests/adapters.rs .github/workflows/ci.yml
git commit -m "test(core): the adapter table, checked against npm and a real handshake

The deprecated @zed-industries/codex-acp is what these catch, and nothing
short of asking the outside world could. initialize, not session/new — the
latter needs auth CI cannot have and would say nothing about our wiring."
```

---

### Task 7: The Mac app stops refusing silently

**Files:**
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (`toggleAgentPane` 860-891, and wherever the root view's overlays live)
- Modify: `apps/macos/Sources/FarCooler/Shortcuts.swift` (62-63)

**Interfaces:**
- Consumes: nothing from earlier tasks — this is independent and can land in any order.
- Produces: nothing later tasks rely on.

- [ ] **Step 1: Fix the shortcuts sheet**

`Shortcuts.swift` lists `a` twice in the same group: line 62 "Add this terminal to the layout" and line 77 "Toggle the focused pane between terminal and chat". `Prefix.swift:204` records that the first meaning was removed. Delete lines 62-63 — `⇧T` on line 63 is stale for the same reason, as neither `t` nor `T` appears in `Prefix.binding(for:)`.

- [ ] **Step 2: Add the missing error renderer**

`client.lastError` is written in three places (`ContentView.swift:72, 879, 888`) and read in exactly one — `fleetPlaceholder` at 488, which draws only while `!client.hasLoaded`. So in normal use every one of those messages is written and discarded, which is why the keystroke that started this work did nothing and said nothing.

Add a banner over the main content, dismissible, that appears whenever `lastError` is set:

```swift
/// Errors the app writes but nothing else shows.
///
/// `client.lastError` had one reader — the fleet placeholder — which draws only
/// before the fleet has loaded. Every message written after that point was
/// discarded, so a refused keystroke was indistinguishable from a broken
/// feature. That is the exact failure `toggleAgentPane` writes its message to
/// prevent.
private struct ErrorBanner: View {
    @ObservedObject var client: DaemonClient

    var body: some View {
        if let error = client.lastError, client.hasLoaded {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(error)
                    .font(.callout)
                    .textSelection(.enabled)
                Spacer(minLength: 8)
                Button {
                    client.lastError = nil
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08)))
            .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .transition(.opacity.combined(with: .move(edge: .top)))
        }
    }
}
```

Attach it as a top-aligned overlay on the detail content, guarded by `client.hasLoaded` so it never competes with `fleetPlaceholder`, which still owns the pre-load case. Animate on `client.lastError` with `.snappy(duration: 0.22)`, matching `PrefixHintOverlay`.

- [ ] **Step 3: Make the refusal message name the real reason**

At `ContentView.swift:888`, the message hard-codes Claude as the only chat-capable agent. That is no longer true:

```swift
                client.lastError =
                    "\(agent) has no chat adapter, so it stays a terminal. "
                    + "Add one in ~/.config/farcooler/config.toml."
```

- [ ] **Step 4: Build and verify by hand**

```bash
apps/macos/build-app.sh
```

Then, in the running app: focus a terminal, run something that is not an agent (`zsh`), press `⌃B a`, and confirm a banner appears saying so rather than nothing happening. Press `⌃B ?` and confirm `a` is listed once.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/ContentView.swift apps/macos/Sources/FarCooler/Shortcuts.swift
git commit -m "fix(macos): a refused keystroke says why

lastError was written in three places and rendered in one, which drew only
before the fleet had loaded — so every message after startup was discarded and
the comment promising 'never silent' described the opposite of what happened.
Also drops the duplicate 'a' row from the shortcuts sheet."
```

---

### Task 8: Document the adapters

**Files:**
- Create: `docs/adapters.md`
- Modify: `README.md` (link it from wherever agent modes are described)

- [ ] **Step 1: Write `docs/adapters.md`**

Cover: the four built-ins and their exact packages; that `opencode` needs no npm package; that cursor's adapter is third-party, stale and best-effort, and what the pane shows when it fails; the config file path and resolution order; a worked `[adapters.<name>]` example including `identity`, with the rule that identity strings must be furniture the agent always draws; and that an agent with no adapter is a terminal, not a bug.

- [ ] **Step 2: Commit**

```bash
git add docs/adapters.md README.md
git commit -m "docs: which agents render as a chat, and how to add one"
```

---

## Self-Review

**Spec coverage.** Registry merge → Task 1. Owned fields and the `const`→function change → Task 1. `crates/core` gaining `serde`/`toml` → Task 1. Config path, resolution order, merge semantics, malformed-file behavior → Task 2. `chat_capable` derived and `CHAT_CAPABLE` deleted → Task 3. `Registry` as a held value rather than a global → Task 3. `--preset` replacing `--adapter` → Task 4. `claude_executable` scoped to claude → Task 4. Built-in adapter table → Tasks 1 and 5. opencode and cursor rules read off real screens → Task 5. Package-health and handshake tests, not ignored, plus the CI install → Task 6. The `lastError` renderer and the duplicate shortcut row → Task 7. The cursor comment recording why it ships anyway → Task 1, Step 5.

**Ordering.** Tasks 1→2→3→4 are a dependency chain. Task 5 needs Task 1. Task 6 needs Tasks 1 and 5. Task 7 is independent of all of them and can land at any point. Task 8 comes last.

**Type consistency.** `AdapterSpec { program, args, env }` is used identically in Tasks 1, 2, 4 and 6, with `env` a `BTreeMap<String, String>` throughout — TOML writes it as a table, which will not deserialize into a `Vec` of pairs. `Command::envs` and `for (k, v) in &spec.env` both accept that map unchanged. `Registry::adapter` returns `Option<&AdapterSpec>` everywhere; `agent_host::resolve` clones it to own it, which is why it returns `Option<AdapterSpec>`. `preset` is `String` on `AgentRules` and `Option<String>` across the CLI boundary. `ConfigAdapter::program` is a required `String`, so the empty-program guard in `merge` catches `""` rather than a missing key, which is what the Task 2 test asserts.
