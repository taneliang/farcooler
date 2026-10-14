//! The first run's repository survey (ov-519): a local, read-only look at a
//! repository that answers the charter's questions, each answer with the
//! signal it came from, or none, when it is the default.
//!
//! **Split in two.** `survey_read` gathers the signals: git, a handful of
//! named files, the runner's own `gh`, and which agents are installed. This
//! file turns signals into answers, with no I/O, so every answer's rule is
//! tested on signals made by hand (`survey_tests.rs`).
//!
//! **What it never does.** No model reads any of it, and nothing leaves the
//! runner except the `gh` calls, which go to GitHub as the owner. It reads no
//! source file: only the paths git lists, and the named files in `NAMED`.
//!
//! **Weak signals stay defaults.** An answer cites a signal only when the
//! signal is strong enough to say so to the owner's face ("12 of the last 40
//! commits on main are merges"). Autonomy, who decides and how to reach the
//! owner are never guessed: they are the skill's own defaults, said plainly.
//!
//! **Threat model.** The repository is hostile: anyone who can push to it, or
//! wrote the third-party repository being added, chooses every path (any
//! bytes but NUL and `/`, newlines and escapes included), every branch name
//! (`-->` is a legal refname), every commit subject, and the named files'
//! contents and kinds (a link, a pipe, a huge file). So, through `gh`, does
//! whoever administers the GitHub repository: its default branch's name, its
//! reviewers' logins, the rules `landing` quotes. What the survey writes goes
//! where it is obeyed or rendered: the charter the orchestrator reads as the
//! owner's word, the wire to every app, and the CLI's output in a terminal.
//! So no check may depend on an HTML comment hiding text from a model, on git
//! refusing an odd name, on `gh` being present or honest, on the repository
//! being small, or on the disk being fast. Every string the repository or
//! GitHub chose leaves this file through `farcooler_core::phrase` (`signal`,
//! `quoted`, and `answers`' last pass), and paths are counted, never named,
//! except a package's, which goes out as a phrase.

use std::collections::BTreeMap;

use farcooler_core::phrase::{phrase, phrase_within};

use farcooler_store::landing::LandingMode;

use crate::landing::{Decision, Facts};

/// Agents at once, when nobody chose: the owner's pick is 1-4 or no limit.
pub const DEFAULT_LANES: u32 = 2;
/// At or under this many commits, a repository is new: what history there
/// is says nothing about how its owner works.
pub const NEW_REPOSITORY: u32 = 10;
/// The share of first-parent commits that are merges at which a repository
/// is read as landing through pull requests (or merge requests elsewhere).
const MERGE_HABIT_PERCENT: u32 = 20;
/// The share of first-parent subjects ending in GitHub's ` (#N)` at which
/// a repository is read as squash- or rebase-merging pull requests.
const PR_SUBJECT_PERCENT: u32 = 20;
/// The most characters an answer's text keeps: the longest the survey writes
/// is about 200.
pub const TEXT_MAX: usize = 600;
/// Merged pull requests, of the last 50, that make a habit.
const PR_HABIT: u32 = 5;

/// The named files the survey reads, at the repository's root. Nothing else
/// is ever opened; every other file is only listed.
pub const NAMED: &[&str] = &["package.json", "Makefile", "justfile", "CLAUDE.md", "AGENTS.md"];

/// Whether the runner's `gh` could be asked, and whether it answered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Gh {
    /// No gh on this runner.
    #[default]
    Missing,
    /// Installed, but no answer for this repository: signed out, a remote
    /// that isn't GitHub's, or a read that timed out.
    Unanswered,
    Answered,
}

/// The last merged pull requests, as `gh pr list` read them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    pub count: u32,
    pub reviewed: u32,
    /// Who reviewed most often, most first, at most three.
    pub reviewers: Vec<String>,
}

/// Everything read, before any of it is an answer.
#[derive(Debug, Clone, Default)]
pub struct Signals {
    /// Every path git lists, tracked or untracked and not ignored.
    pub files: Vec<String>,
    /// The named files' text, by name, when they could be read.
    pub texts: BTreeMap<String, String>,
    /// The branch work lands on: GitHub's default, else `origin/HEAD`'s, else
    /// the checkout's own.
    pub branch: String,
    /// No commit yet.
    pub empty: bool,
    pub shallow: bool,
    /// Commits reachable from HEAD, counted up to `survey_read::MOST_COMMITS`.
    pub commits: u32,
    /// Subjects of the last 90 days' commits, merges left out.
    pub subjects: Vec<String>,
    /// How many parents each of the last first-parent commits has.
    pub parents: Vec<u32>,
    /// How many of those commits' subjects end in GitHub's ` (#N)`: a pull
    /// request squashed or rebased onto the branch.
    pub pr_subjects: u32,
    pub gh: Gh,
    /// What GitHub says of the branch, and what `landing::decide` makes of
    /// it, when `gh` answered.
    pub landing: Option<(Facts, Decision)>,
    pub merged: Option<Merged>,
    /// As `Host.agents_found`.
    pub agents: Vec<String>,
}

/// Where an answer came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A signal, as a phrase the owner reads beside the answer.
    Signal(String),
    /// Nothing was found: the default.
    Default,
    /// The owner chose it on the confirm screen.
    Chosen,
}

/// One answer, under the charter heading it is written under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub key: &'static str,
    /// `## Workflow`, or empty for one the charter doesn't carry.
    pub heading: &'static str,
    /// A word a client switches on; see `SurveyAnswer.value` on the wire.
    pub value: String,
    /// The answer as the charter writes it.
    pub text: String,
    pub source: Source,
}

/// What the owner changed on the confirm screen. Each `None` is left to the
/// survey.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Choices {
    pub landing: Option<LandingMode>,
    /// `Some(0)` is no limit.
    pub lanes: Option<u32>,
    pub initiative: Option<bool>,
}

/// A signal, as one safe phrase (`farcooler_core::phrase`).
pub fn signal(text: impl AsRef<str>) -> Source {
    Source::Signal(phrase(text.as_ref()))
}

/// A name the repository or GitHub chose, quoted in an answer's text.
fn quoted(name: &str) -> String {
    phrase(name)
}

/// Whether `subject` ends the way GitHub's squash and rebase merges end it:
/// ` (#123)`.
pub fn from_pull_request(subject: &str) -> bool {
    let Some(rest) = subject.trim_end().strip_suffix(')') else { return false };
    let Some((before, number)) = rest.rsplit_once("(#") else { return false };
    before.ends_with(' ') && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

/// The landing mode and why, before any choice.
fn workflow_signal(s: &Signals) -> (LandingMode, Source) {
    let decision = s.landing.as_ref().map(|(_, d)| d);
    if let Some(d) = decision.filter(|d| d.suggested == Some(LandingMode::PullRequests)) {
        let why = d.reasons.first().map_or("the branch's rules on GitHub", |r| r.trim_end_matches('.'));
        return (LandingMode::PullRequests, signal(why));
    }
    if let Some(m) = s.merged.as_ref().filter(|m| m.count >= PR_HABIT) {
        return (LandingMode::PullRequests, signal(format!("{} of the last 50 pull requests were merged", m.count)));
    }
    if let Some(d) = decision.filter(|d| d.suggested == Some(LandingMode::Direct)) {
        let why = d.reasons.last().map_or("nothing on GitHub asks for pull requests", |r| r.trim_end_matches('.'));
        return (LandingMode::Direct, signal(why));
    }
    let seen = s.parents.len() as u32;
    let merges = merges(s);
    let base = quoted(&s.branch);
    if seen > NEW_REPOSITORY && merges * 100 >= seen * MERGE_HABIT_PERCENT {
        return (LandingMode::PullRequests, signal(format!("{merges} of the last {seen} commits on {base} are merges")));
    }
    let prs = s.pr_subjects;
    if seen > NEW_REPOSITORY && prs * 100 >= seen * PR_SUBJECT_PERCENT {
        return (LandingMode::PullRequests, signal(format!("{prs} of the last {seen} commits on {base} came from pull requests")));
    }
    // A straight history alone is no signal: squash merges leave one too,
    // and without GitHub nothing tells them apart (review 2a, M4).
    (LandingMode::Direct, Source::Default)
}

fn merges(s: &Signals) -> u32 {
    s.parents.iter().filter(|p| **p > 1).count() as u32
}

/// `Some(true)` when the history is linear, `Some(false)` with merges in it,
/// `None` when there is too little to say.
fn linear(s: &Signals) -> Option<bool> {
    (s.parents.len() as u32 > NEW_REPOSITORY).then(|| merges(s) == 0)
}

fn workflow(s: &Signals, c: &Choices) -> (LandingMode, Answer) {
    let (found, why) = workflow_signal(s);
    let (mode, source) = match c.landing {
        Some(m) if m != found || why == Source::Default => (m, Source::Chosen),
        _ => (found, why),
    };
    let base = quoted(&s.branch);
    let method = s.landing.as_ref().and_then(|(_, d)| d.merge_method);
    let text = match mode {
        LandingMode::PullRequests => {
            let how = match method {
                Some("squash") => " Squash each one when it merges.",
                Some("rebase") => " Rebase each one when it merges.",
                Some("merge") => " Merge each one with a merge commit.",
                _ => "",
            };
            format!("PR: each change is a pull request into {base} that a person approves.{how}")
        }
        LandingMode::Direct => {
            let how = match linear(s) {
                Some(false) => " Merge commits are fine.",
                _ => " Rebase onto it: no merge commits.",
            };
            format!("Direct: work lands on {base} and you push it.{how}")
        }
    };
    let value = match mode {
        LandingMode::PullRequests => "pull_requests",
        LandingMode::Direct => "direct",
    };
    (mode, Answer { key: "workflow", heading: "## Workflow", value: value.into(), text, source })
}

/// A command each manifest at the root implies, and the manifest it came from.
fn test_commands(s: &Signals) -> Vec<(String, &'static str)> {
    let has = |path: &str| s.files.iter().any(|f| f == path);
    let mut out: Vec<(String, &'static str)> = Vec::new();
    if has("Cargo.toml") {
        out.push(("cargo test".into(), "Cargo.toml"));
    }
    if has("go.mod") {
        out.push(("go test ./...".into(), "go.mod"));
    }
    if has("Package.swift") {
        out.push(("swift test".into(), "Package.swift"));
    }
    if ["pyproject.toml", "pytest.ini", "setup.py", "tox.ini"].iter().any(|f| has(f)) {
        let from = ["pyproject.toml", "pytest.ini", "setup.py", "tox.ini"].into_iter().find(|f| has(f)).unwrap_or("pyproject.toml");
        out.push(("pytest".into(), from));
    }
    if let Some(scripts) = s.texts.get("package.json").and_then(|t| package_scripts(t)) {
        let runner = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun run"
        } else {
            "npm"
        };
        for name in ["test", "lint", "typecheck"] {
            if scripts.iter().any(|s| s == name) {
                let command = match (runner, name) {
                    ("npm", "test") => "npm test".to_string(),
                    ("npm", other) => format!("npm run {other}"),
                    (r, other) => format!("{r} {other}"),
                };
                out.push((command, "package.json"));
            }
        }
    }
    if s.texts.get("Makefile").is_some_and(|t| t.lines().any(|l| l.starts_with("test:"))) {
        out.push(("make test".into(), "Makefile"));
    }
    if s.texts.get("justfile").is_some_and(|t| t.lines().any(|l| l.starts_with("test:") || l.starts_with("test "))) {
        out.push(("just test".into(), "justfile"));
    }
    out
}

/// `scripts`' names in a package.json, leaving out npm's placeholder test.
fn package_scripts(text: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let scripts = v.get("scripts")?.as_object()?;
    Some(
        scripts
            .iter()
            .filter(|(_, run)| !run.as_str().is_some_and(|r| r.contains("no test specified")))
            .map(|(name, _)| name.clone())
            .collect(),
    )
}

/// The CI configuration files git lists: GitHub's workflows and the other
/// common services'.
pub fn ci_files(files: &[String]) -> Vec<&str> {
    files
        .iter()
        .map(String::as_str)
        .filter(|f| {
            let workflow = f.strip_prefix(".github/workflows/").is_some_and(|name| {
                !name.contains('/') && (name.ends_with(".yml") || name.ends_with(".yaml"))
            });
            workflow
                || matches!(*f, ".gitlab-ci.yml" | ".circleci/config.yml" | "azure-pipelines.yml" | "Jenkinsfile" | "bitbucket-pipelines.yml")
                || f.starts_with(".buildkite/")
        })
        .collect()
}

fn done_means(s: &Signals) -> Answer {
    let ci = ci_files(&s.files);
    let commands = test_commands(s);
    let mut seen = Vec::new();
    let mut named: Vec<String> = Vec::new();
    for (command, from) in &commands {
        named.push(format!("`{command}`"));
        if !seen.contains(from) {
            seen.push(*from);
        }
    }
    let list = join_and(&named);
    let mut sources: Vec<String> = Vec::new();
    if !ci.is_empty() {
        let github = ci.iter().filter(|f| f.starts_with(".github/workflows/")).count();
        sources.push(if github == ci.len() {
            let n = github;
            format!("{n} GitHub workflow{}", if n == 1 { "" } else { "s" })
        } else {
            let others = ci.len() - github;
            let other = format!("{others} other CI file{}", if others == 1 { "" } else { "s" });
            if github == 0 {
                other.replace("other ", "")
            } else {
                format!("{github} GitHub workflow{}, {other}", if github == 1 { "" } else { "s" })
            }
        });
    }
    sources.extend(seen.iter().map(|f| f.to_string()));
    let text = match (ci.is_empty(), named.is_empty()) {
        (false, false) => format!("CI passes, and {list} pass{} first.", if named.len() == 1 { "es" } else { "" }),
        (false, true) => "CI passes.".to_string(),
        (true, false) => format!("{list} pass{}.", if named.len() == 1 { "es" } else { "" }),
        (true, true) => "I try it first.".to_string(),
    };
    let source = if sources.is_empty() { Source::Default } else { signal(sources.join(", ")) };
    Answer { key: "done_means", heading: "## Done means", value: String::new(), text: capitalized(text), source }
}

pub fn codeowners(files: &[String]) -> bool {
    files.iter().any(|f| matches!(f.as_str(), "CODEOWNERS" | ".github/CODEOWNERS" | "docs/CODEOWNERS"))
}

fn review(s: &Signals, mode: LandingMode) -> Answer {
    let mut lines: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let approvals = s.landing.as_ref().and_then(|(f, _)| f.rules.as_ref()).filter(|r| r.pull_request).map_or(0, |r| r.approvals);
    if approvals > 0 {
        let n = approvals;
        lines.push(format!("Each pull request needs {n} approving review{}.", if n == 1 { "" } else { "s" }));
        sources.push(format!("{}'s rules on GitHub", s.branch));
    }
    if codeowners(&s.files) {
        lines.push("Code owners review the files they own.".into());
        sources.push("CODEOWNERS".into());
    }
    if let Some(m) = s.merged.as_ref().filter(|m| m.count >= PR_HABIT && m.reviewed * 2 >= m.count) {
        let who = if m.reviewers.is_empty() { String::new() } else { format!(", most often by {}", join_and(&m.reviewers.iter().map(|r| quoted(r)).collect::<Vec<_>>())) };
        lines.push(format!("Pull requests are reviewed before they merge{who}."));
        sources.push(format!("{} of {} merged pull requests were reviewed", m.reviewed, m.count));
    }
    if lines.is_empty() {
        let text = match mode {
            LandingMode::PullRequests => "I review each pull request before it merges. An agent reviews it first.",
            LandingMode::Direct => "An agent reviews each change before it lands, and I look at what landed.",
        };
        return Answer { key: "review", heading: "## Review", value: String::new(), text: text.into(), source: Source::Default };
    }
    lines.push("An agent reviews each change first.".into());
    Answer { key: "review", heading: "## Review", value: String::new(), text: lines.join(" "), source: signal(sources.join(", ")) }
}

fn lanes(c: &Choices) -> Answer {
    let n = c.lanes.unwrap_or(DEFAULT_LANES);
    let (value, at_once) = match n {
        0 => ("no_limit".to_string(), "As many agents at once as the work needs".to_string()),
        1 => ("1".to_string(), "One agent at a time".to_string()),
        n => (n.to_string(), format!("Up to {n} agents at once")),
    };
    let later = match n {
        0 => "as many as the work needs".to_string(),
        1 => "one at a time".to_string(),
        n => format!("up to {n}"),
    };
    let first = if n == 1 {
        String::new()
    } else {
        format!(" This is the first run: one lane at a time until the first lane lands, then {later}.")
    };
    let text = format!("{at_once}, each in a worktree of its own: two tasks never share one.{first}");
    let source = if c.lanes.is_some() { Source::Chosen } else { Source::Default };
    Answer { key: "lanes", heading: "## Lanes", value, text, source }
}

fn autonomy(mode: LandingMode, branch: &str) -> Answer {
    let text = match mode {
        LandingMode::PullRequests => "Agents may commit and push their own branches, and open a pull request for each \
                                      card. Nothing merges without a person's approval. Ask me before adding a dependency."
            .to_string(),
        LandingMode::Direct => format!(
            "Agents may commit on their own branches. You land their work on {branch} once the gates pass. Ask me \
             before adding a dependency."
        ),
    };
    Answer { key: "autonomy", heading: "## Autonomy", value: String::new(), text, source: Source::Default }
}

fn initiative(c: &Choices) -> Answer {
    let on = c.initiative.unwrap_or(true);
    let text = if on { "Initiative: on." } else { "Initiative: off." };
    let source = if c.initiative.is_some() { Source::Chosen } else { Source::Default };
    Answer { key: "initiative", heading: "## Autonomy", value: if on { "on" } else { "off" }.into(), text: text.into(), source }
}

/// The instruction files an agent already reads, with their line counts.
pub fn instructions(s: &Signals) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for name in ["CLAUDE.md", "AGENTS.md"] {
        if s.files.iter().any(|f| f == name) {
            let lines = s.texts.get(name).map_or(0, |t| t.lines().count() as u32);
            out.push((name.to_string(), lines));
        }
    }
    let rules = s.files.iter().filter(|f| f.starts_with(".cursor/rules/")).count() as u32;
    if rules > 0 {
        out.push((".cursor/rules".to_string(), rules));
    }
    out
}

fn instructions_answer(s: &Signals) -> Answer {
    let files = instructions(s);
    if files.is_empty() {
        return Answer {
            key: "instructions",
            heading: "## Anything else",
            value: String::new(),
            text: "Nothing yet.".into(),
            source: Source::Default,
        };
    }
    let names: Vec<String> = files.iter().map(|(f, _)| f.clone()).collect();
    let text = format!(
        "Agents follow {}, which they already read: don't copy {} here.",
        join_and(&names),
        if names.len() == 1 { "it" } else { "them" }
    );
    let sources: Vec<String> = files
        .iter()
        .map(|(f, n)| {
            let unit = if f == ".cursor/rules" { "file" } else { "line" };
            format!("{f} ({n} {unit}{})", if *n == 1 { "" } else { "s" })
        })
        .collect();
    Answer { key: "instructions", heading: "## Anything else", value: String::new(), text, source: signal(sources.join(", ")) }
}

/// Whether a subject is `<type>: <subject>` or `<type>(<scope>): <subject>`.
pub fn prefixed(subject: &str) -> bool {
    let Some((head, rest)) = subject.split_once(": ") else { return false };
    if rest.trim().is_empty() {
        return false;
    }
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) if !scope.is_empty() && !scope.contains(['(', ')']) => kind,
            _ => return false,
        },
        None => head,
    };
    !kind.is_empty()
        && kind.len() <= 20
        && kind.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
}

/// The share of `subjects` that are prefixed, rounded, and the median
/// subject's length in characters.
pub fn commit_style(subjects: &[String]) -> (u32, u32) {
    if subjects.is_empty() {
        return (0, 0);
    }
    let n = subjects.len();
    let share = (subjects.iter().filter(|s| prefixed(s)).count() * 100 + n / 2) / n;
    let mut lengths: Vec<usize> = subjects.iter().map(|s| s.chars().count()).collect();
    lengths.sort_unstable();
    (share as u32, lengths[n / 2] as u32)
}

fn commit_style_answer(s: &Signals) -> Option<Answer> {
    let (share, _) = commit_style(&s.subjects);
    let n = s.subjects.len() as u32;
    (n > NEW_REPOSITORY && share >= 60).then(|| Answer {
        key: "commit_style",
        heading: "## Anything else",
        value: String::new(),
        text: "Commit subjects start with what they touch: `<area>: <what changed>`.".into(),
        source: signal(format!("{share}% of the last {n} commit subjects")),
    })
}

fn harness(s: &Signals) -> Answer {
    let first = s.agents.first().cloned().unwrap_or_default();
    let source = if first.is_empty() { Source::Default } else { signal(format!("{first} is installed")) };
    Answer { key: "harness", heading: "", value: first, text: String::new(), source }
}

/// The text of an answer whose words are the skill's own defaults.
fn fixed(key: &'static str, heading: &'static str, text: &str) -> Answer {
    Answer { key, heading, value: String::new(), text: text.into(), source: Source::Default }
}

/// Every answer the signals and the owner's choices make, in the charter's
/// order, and `harness` last.
pub fn answers(s: &Signals, c: &Choices) -> Vec<Answer> {
    let (mode, workflow) = workflow(s, c);
    let mut out = vec![
        workflow,
        done_means(s),
        review(s, mode),
        fixed(
            "who_decides",
            "## Who decides",
            "Make a reversible call yourself and record it as a ruling. Anything irreversible or outward \
             (publishing, deleting, spending money, other people's data) comes to me first.",
        ),
        fixed(
            "reaching_me",
            "## Reaching me",
            "Ask me on the board with `task ask`, and say the question in your reply too. It shows in Needs \
             You, and on my phone when one is paired.",
        ),
        lanes(c),
        autonomy(mode, &quoted(&s.branch)),
        initiative(c),
        instructions_answer(s),
    ];
    out.extend(commit_style_answer(s));
    out.push(harness(s));
    // The last pass, whatever a rule above forgot: every text one line,
    // every signal one phrase.
    for a in &mut out {
        a.text = phrase_within(&a.text, TEXT_MAX);
        if let Source::Signal(why) = &a.source {
            a.source = signal(why);
        }
    }
    out
}

/// "a", "a and b", "a, b and c".
pub fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn capitalized(text: String) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => first.to_ascii_uppercase().to_string() + chars.as_str(),
        _ => text,
    }
}

/// A language by extension, for the ones worth naming.
pub fn language(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "rs" => "Rust",
        "swift" => "Swift",
        "kt" | "kts" => "Kotlin",
        "java" => "Java",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "rb" => "Ruby",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "C++",
        "m" | "mm" => "Objective-C",
        "cs" => "C#",
        "php" => "PHP",
        "scala" => "Scala",
        "ex" | "exs" => "Elixir",
        "erl" => "Erlang",
        "hs" => "Haskell",
        "ml" | "mli" => "OCaml",
        "clj" | "cljs" => "Clojure",
        "dart" => "Dart",
        "lua" => "Lua",
        "zig" => "Zig",
        "sh" | "bash" | "zsh" | "fish" => "Shell",
        "sql" => "SQL",
        "vue" => "Vue",
        "svelte" => "Svelte",
        _ => return None,
    })
}

/// Paths a survey of languages leaves out: other people's code.
fn vendored(path: &str) -> bool {
    path.split('/').any(|part| matches!(part, "node_modules" | "vendor" | "third_party" | "Pods" | "dist" | "build"))
}

/// The top three languages in `files`, by count, with their rounded share of
/// the files in a language.
pub fn languages(files: &[String]) -> Vec<(String, u32)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for f in files.iter().filter(|f| !vendored(f)) {
        if let Some(l) = language(f) {
            *counts.entry(l).or_default() += 1;
        }
    }
    let total: usize = counts.values().sum();
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.into_iter().take(3).map(|(l, n)| (l.to_string(), ((n * 100 + total / 2) / total) as u32)).collect()
}

/// The manifests that make a folder a package of its own.
const MANIFESTS: &[&str] =
    &["package.json", "Cargo.toml", "go.mod", "pyproject.toml", "Package.swift", "build.gradle", "build.gradle.kts", "pom.xml"];
/// The most packages listed.
const MOST_PACKAGES: usize = 20;

/// The folders, up to three deep, holding a manifest of their own and inside
/// no other such folder, each with its main language, the path as a phrase.
/// Empty unless there are at least three: one or two is a repository with a
/// helper, not a monorepo.
pub fn packages(files: &[String]) -> Vec<(String, String)> {
    let mut dirs: Vec<&str> = files
        .iter()
        .filter(|f| !vendored(f) && !f.split('/').any(|p| matches!(p, "fixtures" | "examples" | "testdata")))
        .filter_map(|f| f.rsplit_once('/').filter(|(_, name)| MANIFESTS.contains(name)).map(|(dir, _)| dir))
        .filter(|dir| dir.split('/').count() <= 3)
        .collect();
    dirs.sort_unstable();
    dirs.dedup();
    // A package inside another (a Gradle project's module) is part of it.
    let outer = dirs.clone();
    dirs.retain(|d| !outer.iter().any(|o| d.len() > o.len() && d.starts_with(o) && d.as_bytes()[o.len()] == b'/'));
    if dirs.len() < 3 {
        return Vec::new();
    }
    dirs.into_iter()
        .take(MOST_PACKAGES)
        .map(|dir| {
            let prefix = format!("{dir}/");
            let inside: Vec<String> = files.iter().filter(|f| f.starts_with(&prefix)).cloned().collect();
            let main = languages(&inside).into_iter().next().map(|(l, _)| l).unwrap_or_default();
            (phrase(dir), main)
        })
        .collect()
}

#[cfg(test)]
#[path = "survey_tests.rs"]
mod tests;
