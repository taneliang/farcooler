//! The core-vocabulary check (docs/engine-pipeline.md 2.3): no identifier in crates/workflow/src/pipeline/
//! names a coding concept. Every type, field, variant, function and local name is split into its snake_case
//! and CamelCase segments; a segment that is a coding word (or its plural or past form) fails the check, so
//! `open_pr` and `PushedAt` fail and `priority` passes. Comments and string literals are not code and are
//! skipped. Names from outside the module (`BTreeMap`) are allowed by name.
use std::path::Path;

const WORDS: [&str; 11] = ["branch", "commit", "candidate", "base", "push", "pr", "ci", "git", "tree", "fold", "gate"];
const EXTERNAL: [&str; 2] = ["BTreeMap", "BTreeSet"];

/// Code with comments and string and char literals blanked out.
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' { i += 1; }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') { i += 1; }
            i += 2;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' { if chars[i] == '\\' { i += 1; } i += 1; }
            i += 1;
            out.push(' ');
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            i += 3;
            out.push(' ');
        } else if c == '\'' && next == Some('\\') {
            i += 2;
            while i < chars.len() && chars[i] != '\'' { i += 1; }
            i += 1;
            out.push(' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}
fn identifiers(code: &str) -> Vec<String> {
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_'))
        .map(str::to_string).collect()
}
/// `open_pr` → [open, pr]; `PullRequest` → [pull, request]; `HTTPServer` → [http, server].
fn segments(ident: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in ident.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<char> = part.chars().collect();
        let mut word = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = c.is_ascii_uppercase() && i > 0 && (chars[i - 1].is_ascii_lowercase()
                || chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase()) && chars[i - 1].is_ascii_uppercase());
            if boundary && !word.is_empty() { out.push(std::mem::take(&mut word)); }
            word.push(c.to_ascii_lowercase());
        }
        if !word.is_empty() { out.push(word); }
    }
    out
}
fn coding_word(segment: &str) -> Option<&'static str> {
    WORDS.iter().copied().find(|w| ["", "s", "es", "ed", "ing"].iter().any(|suffix| segment == format!("{w}{suffix}")))
}
/// Every coding name in `source`, with why.
fn violations(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    // `Vec::push` is the standard library's: a call `.push(` is not a name this module chose. A field, type
    // or function of the module named `push` is still caught.
    let code = code_only(source).replace(".push(", ".(");
    for ident in identifiers(&code) {
        if EXTERNAL.contains(&ident.as_str()) { continue; }
        let segs = segments(&ident);
        if let Some(word) = segs.iter().find_map(|s| coding_word(s)) { found.push(format!("{ident} ({word})")); }
        if segs.windows(2).any(|w| w[0] == "pull" && w[1].starts_with("request")) { found.push(format!("{ident} (pull request)")); }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn the_pipeline_core_names_no_coding_concept() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pipeline");
    let mut files = 0;
    let mut names = 0;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the pipeline module exists") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") { continue; }
        let source = std::fs::read_to_string(&path).unwrap();
        files += 1;
        names += identifiers(&code_only(&source)).len();
        found.extend(violations(&source).into_iter().map(|v| format!("{}: {v}", path.file_name().unwrap().to_string_lossy())));
    }
    assert!(files >= 9 && names > 1000, "the check read the module: {files} files, {names} names");
    assert!(found.is_empty(), "coding words in the pipeline core:\n{}", found.join("\n"));
}

#[test]
fn the_check_turns_red_on_each_coding_name() {
    for source in [
        "pub struct Token { pub branch: String }",
        "pub struct Membership { pub candidate: String }",
        "pub struct ArchivedBatch { pub pushed: Option<String> }",
        "fn open_pr() {}",
        "enum Kind { PullRequest }",
        "let ci_run = 1;",
        "struct GateResult;",
        "fn fold_members() {}",
        "pub struct Work { pub commits: Vec<String> }",
        "let tree = 0;",
        "type BaseRef = String;",
        "fn git() {}",
        "pub struct Batch { pub prs: u8 }",
        "pub struct Work { pub push: bool }",
        "fn push(&mut self) {}",
    ] {
        assert!(!violations(source).is_empty(), "{source} must turn the check red");
    }
}

#[test]
fn the_check_passes_neutral_names_and_ignores_comments_and_strings() {
    for source in [
        "pub struct Token { pub priority: i32, pub principal: String }",
        "fn decide(circle: u8) -> BTreeMap<String, BTreeSet<String>> {}",
        "// a push is outward\nlet label = \"push\";",
        "/* the branch */ let process = 'x';",
        "fn view<'a>(s: &'a str) {}",
        "out.push(c);",
    ] {
        assert!(violations(source).is_empty(), "{source} is neutral: {:?}", violations(source));
    }
}
