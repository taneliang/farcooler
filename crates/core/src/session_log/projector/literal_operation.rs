//! Literal invocation intent only. No process, filesystem, environment or receipt access.
use serde::Serialize;
use serde_json::Value;
use std::hash::{Hash, Hasher};
use super::detail;

const MAX_SOURCE: usize = 65_536;
const EXECUTABLE: &str = "/Applications/Far Cooler Canary.app/Contents/Resources/farcooler";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandLifecycle { Running, Completed, Failed, Declined, Unconfirmed }

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteralOperation {
    pub version: u8,
    pub provenance: &'static str,
    pub kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    pub preview_clipped: bool,
    pub lifecycle: CommandLifecycle,
    pub input_clipped: bool,
    pub output_available: bool,
    pub output_clipped: bool,
}

pub(super) fn completed(item: &Value) -> CommandLifecycle {
    use CommandLifecycle::*;
    match (item.get("exit_code").and_then(Value::as_i64), item.get("status").and_then(Value::as_str)) {
        (Some(0), Some("completed")) => Completed,
        (Some(n), Some("completed")) if n != 0 => Unconfirmed,
        (Some(0), Some("failed" | "declined")) => Unconfirmed,
        (_, Some("declined")) => Declined,
        (_, Some("failed")) => Failed,
        (Some(n), None) if n != 0 => Failed,
        _ => Unconfirmed,
    }
}

/// Each word's end in the original script, before any display transformation.
fn literal_words(script: &str) -> Option<Vec<(String, usize)>> {
    if script.len() > MAX_SOURCE || script.contains('\0') { return None; }
    let mut out = Vec::new();
    let mut chars = script.char_indices().peekable();
    while let Some(&(_, c)) = chars.peek() {
        if c == ' ' || c == '\t' { chars.next(); continue; }
        if c == '\n' || c == '\r' { return None; }
        let mut word = String::new();
        let mut end = 0;
        let mut quote = None;
        while let Some(&(at, c)) = chars.peek() {
            if quote.is_none() && (c == ' ' || c == '\t') { break; }
            chars.next();
            end = at + c.len_utf8();
            match quote {
                Some('\'') => if c == '\'' { quote = None; } else { word.push(c); },
                Some('"') => match c {
                    '"' => quote = None,
                    '$' | '`' | '\\' => return None,
                    _ => word.push(c),
                },
                None => match c {
                    '=' if word.is_empty() => return None,
                    '\'' | '"' => quote = Some(c),
                    '\\' => {
                        let (at, escaped) = chars.next()?;
                        if matches!(escaped, '\n' | '\r') { return None; }
                        word.push(escaped); end = at + escaped.len_utf8();
                    }
                    ';' | '|' | '&' | '<' | '>' | '(' | ')' | '`' | '$' | '*' | '?' | '[' | ']' | '{' | '}' | '!' | '#' | '~' | '\n' | '\r' => return None,
                    _ => word.push(c),
                },
                _ => return None,
            }
        }
        if quote.is_some() { return None; }
        out.push((word, end));
        if out.len() > 512 { return None; }
    }
    Some(out)
}

/// Bounded original input projection and the selected field's value offset.
fn projection(args: &Value, key: &str) -> Option<(String, usize)> {
    let fields = args.as_object()?;
    if fields.len() > 16 { return None; }
    let mut source = String::new();
    let mut offset = None;
    for (k, v) in fields {
        if source.len() > MAX_SOURCE { return None; }
        if !source.is_empty() { source.push('\n'); }
        source.push_str(k); source.push(':');
        match v {
            Value::String(s) => {
                if s.len() > MAX_SOURCE { return None; }
                source.push(if s.contains('\n') { '\n' } else { ' ' });
                if k == key { offset = Some(source.len()); }
                source.push_str(if s.contains('\n') { s.trim_end() } else { s });
            }
            Value::Array(a) if k == key && a.len() <= 512 && a.iter().all(|s| s.as_str().is_some_and(|s| s.len() <= MAX_SOURCE)) => {
                if a.iter().filter_map(Value::as_str).map(str::len).sum::<usize>() > MAX_SOURCE { return None; }
                source.push(' '); offset = Some(source.len()); source.push_str(&v.to_string());
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => { source.push(' '); source.push_str(&v.to_string()); }
            _ => return None,
        }
    }
    if source.len() > MAX_SOURCE { return None; }
    Some((source, offset?))
}

/// Returns bounded display metadata plus a transient canonical-argument fingerprint.
/// The fingerprint never enters row JSON/cache and is not execution identity.
pub(super) fn recognize(name: &str, args: &Value) -> Option<(LiteralOperation, u64)> {
    if !matches!(name, "exec_command" | "shell_command") { return None; }
    let key = if args.get("cmd").is_some() { "cmd" } else { "command" };
    if args.get("cmd").is_some() && args.get("command").is_some() { return None; }
    let value = args.get(key)?;
    let (source, offset) = projection(args, key)?;
    let (words, ends): (Vec<String>, Vec<usize>) = if let Some(script) = value.as_str() {
        let parsed = literal_words(script)?;
        parsed.into_iter().map(|(s, end)| (s, offset + end)).unzip()
    } else {
        let values = value.as_array()?;
        let argv: Vec<&str> = values.iter().map(Value::as_str).collect::<Option<_>>()?;
        if let ["/bin/zsh", "-lc", script] = argv.as_slice() {
            let parsed = literal_words(script)?;
            let prefix = serde_json::to_string(&argv[..2]).ok()?;
            let base = offset + prefix.len(); // replace closing ] with comma; next byte opens script string
            parsed.into_iter().map(|(s, end)| {
                let encoded_prefix = serde_json::to_string(&script[..end]).ok()?;
                Some((s, base + encoded_prefix.len() - 1))
            }).collect::<Option<Vec<_>>>()?.into_iter().unzip()
        } else {
            if argv.len() > 512 { return None; }
            let mut ends = Vec::new();
            for i in 0..argv.len() { ends.push(offset + serde_json::to_string(&argv[..=i]).ok()?.len() - 1); }
            (argv.into_iter().map(str::to_string).collect(), ends)
        }
    };
    if words.first()?.as_str() != EXECUTABLE { return None; }
    let mut hasher = std::collections::hash_map::DefaultHasher::new(); words.hash(&mut hasher);
    let fingerprint = hasher.finish();
    // --json is the only admitted global flag; do not remove a message/body's data.
    let mut at = 1;
    if words.get(at).is_some_and(|s| s == "--json") { at += 1; }
    let indices: Vec<usize> = (at..words.len()).collect();
    let args: Vec<&str> = indices.iter().map(|&i| words[i].as_str()).collect();
    let (kind, target, body): (String, &str, Option<usize>) = match args.as_slice() {
        ["task", "show", key] if task_key(key) => ("task_show".into(), key, None),
        ["task", "set", key, "--status", status] if task_key(key) && ["backlog", "todo", "needs_decision", "in_progress", "in_review", "done", "cancelled"].contains(status) => (format!("task_set_{status}"), key, None),
        ["task", "note", key, rest @ ..] if task_key(key) => {
            let mut kind = None; let mut body = None;
            for pair in rest.chunks(2) {
                let [flag, value] = pair else { return None; };
                match *flag {
                    "--kind" if kind.is_none() && ["decision", "finding", "question", "answer", "progress", "comment"].contains(value) => kind = Some(*value),
                    "--body" if body.is_none() => body = Some(*value),
                    _ => return None,
                }
            }
            let kind = kind?; let _ = body?;
            let body_at = args.iter().position(|s| *s == "--body")? + 1;
            (format!("task_note_{kind}"), key, Some(indices[body_at]))
        }
        ["message", "orchestrator", text] if !text.starts_with('-') => ("message_orchestrator".into(), "orchestrator", Some(indices[2])),
        ["message", "orchestrator", "--", _] => ("message_orchestrator".into(), "orchestrator", Some(indices[3])),
        _ => return None,
    };
    let kept = detail::retained_end(&source);
    let eligible = body.filter(|&i| ends[i] <= kept);
    let mut preview_clipped = false;
    let preview = eligible.map(|i| {
        let text = &words[i];
        let mut end = text.len();
        if let Some((at, _)) = text.match_indices('\n').nth(3) { end = at; }
        if let Some((at, _)) = text[..end].char_indices().nth(280) { end = at; }
        preview_clipped = end < text.len();
        text[..end].to_string()
    });
    let mut op = LiteralOperation { version: 1, provenance: "literal_intent", kind, target: target.to_string(), preview, preview_clipped,
        lifecycle: CommandLifecycle::Running, input_clipped: kept < source.trim_end().len(), output_available: false, output_clipped: false };
    while serde_json::to_vec(&op).ok()?.len() > 1000 {
        let preview = op.preview.as_mut()?;
        preview.pop()?;
        op.preview_clipped = true;
    }
    Some((op, fingerprint))
}

fn task_key(key: &str) -> bool {
    key.len() <= 64 && key.rsplit_once('-').is_some_and(|(prefix, n)| !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_alphanumeric()) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
