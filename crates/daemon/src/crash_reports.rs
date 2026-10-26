//! `crash.report` (ov-797): a crash or hang the owner's own phone recorded,
//! kept once per signature and filed on Far Cooler's own board.
//!
//! The phone's MetricKit hands it each diagnostic on the launch after, and
//! the phone sends the signature and nothing else (`AgentKit`'s
//! `CrashSignature`). This end refuses anything that could carry more: an
//! exception name that isn't a name in code, a binary that is a path, more
//! frames than a signature has, and text with control characters in it.
//!
//! **Where it shows: a card on the board, not Needs You.** Needs You is what
//! a person must act on now, and it pushes; a crash is work for an agent to
//! fix, and the same crash every launch would push every launch. A card is
//! where that work is dispatched from, and the coordinator reads the board
//! at every check-in. So a new signature files one card, labeled `found` and
//! `crash`, with its frames in a Finding note; a repeat raises the count in
//! the card's intent and adds nothing; a repeat on a newer build after the
//! card was closed adds one Finding note, because the fix didn't hold.
//!
//! The card goes on Far Cooler's own board: the Main workspace of a
//! repository whose origin is a `farcooler` repository. A runner with none
//! keeps the count and files nothing, since nobody there can fix the app.

use std::sync::Mutex;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::models::{Actor, NoteKind, Task, TaskStatus, TaskUpdate, Workspace};
use serde_json::json;

use crate::service::Service;
use crate::watch::Watcher;

/// The most frames a signature carries: the phone's top frames and its
/// first own frame.
pub const MAX_FRAMES: usize = 16;
/// The labels on a crash's card.
pub const LABELS: [&str; 2] = ["found", "crash"];
/// The longest a card's title gets (`docs`' ticket titles: 60 at most).
const MAX_TITLE: usize = 60;

/// One report at a time, so two of a new signature file one card.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// `crash.report`: count the report, and file or raise its card.
pub fn handle(svc: &Service, watcher: &Watcher, req: pb::Request) -> Result<result::Value> {
    let Some(request::Payload::CrashReport(report)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    checked(&report)?;
    let digest = signature(&report);
    let detail = detail(&report).to_string();
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let record = svc.store.record_crash(&digest, &detail, &report.app_build)?;
    let task = match record.task.and_then(|id| svc.store.get_task(id).ok()) {
        Some(task) => Some(raise(svc, &report, &task, record.count, record.previous_build.as_deref())?),
        None => match board(svc)? {
            Some(board) => {
                let task = file(svc, &board, &report, &digest, record.count)?;
                svc.store.set_crash_task(&digest, task.id)?;
                Some(task)
            }
            None => None,
        },
    };
    if let Some(task) = &task {
        watcher.announce_task_changed(task, None, Actor::Runner);
    }
    tracing::info!(signature = %&digest[..12], count = record.count, card = task.as_ref().map_or("-", |t| t.key.as_str()), "crash reported");
    Ok(result::Value::CrashRecorded(pb::CrashRecorded {
        count: record.count,
        task_key: task.map(|t| t.key).unwrap_or_default(),
    }))
}

/// Refuse a report that carries more than a signature, or isn't one.
pub fn checked(report: &pb::CrashReport) -> Result<()> {
    if !matches!(report.kind.as_str(), "crash" | "hang") {
        return Err(DomainError::InvalidArgument { what: "kind" });
    }
    if report.frames.is_empty() || report.frames.len() > MAX_FRAMES {
        return Err(DomainError::InvalidArgument { what: "frames" });
    }
    for frame in &report.frames {
        let binary = &frame.binary;
        if binary.is_empty() || binary.contains('/') || !plain(binary, 128) || !plain(&frame.symbol, 256) {
            return Err(DomainError::InvalidArgument { what: "frames" });
        }
    }
    // A class name, never a message: letters, digits, `_`, `.` and `$`.
    let name = &report.exception_name;
    if name.chars().count() > 128 || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')) {
        return Err(DomainError::InvalidArgument { what: "exception_name" });
    }
    if !plain(&report.app_version, 64) || !plain(&report.app_build, 64) || !plain(&report.os, 128) {
        return Err(DomainError::InvalidArgument { what: "build" });
    }
    Ok(())
}

/// At most `max` characters, none of them a control character.
fn plain(text: &str, max: usize) -> bool {
    text.chars().count() <= max && !text.chars().any(char::is_control)
}

/// What makes two reports one: the kind, the exception, and the frames. Not
/// the build or the OS, so the same crash on a newer OS is a repeat; the
/// frames' offsets move with each build of the app's own binary, though, so
/// a fix shipped and a crash in the same place after it is a new signature.
pub fn signature(report: &pb::CrashReport) -> String {
    let frames: Vec<_> = report.frames.iter().map(|f| json!([f.binary, f.offset, f.symbol, f.depth])).collect();
    let key = json!([
        report.kind,
        report.exception_type,
        report.exception_code,
        report.signal,
        report.exception_name,
        frames,
    ]);
    crate::charter_read::sha256_hex(key.to_string().as_bytes())
}

/// What the store keeps of a report: the report itself.
fn detail(report: &pb::CrashReport) -> serde_json::Value {
    json!({
        "kind": report.kind,
        "exception_type": report.exception_type,
        "exception_code": report.exception_code,
        "signal": report.signal,
        "exception_name": report.exception_name,
        "frames": report.frames.iter().map(|f| json!({
            "binary": f.binary, "offset": f.offset, "symbol": f.symbol, "depth": f.depth, "own": f.own,
        })).collect::<Vec<_>>(),
        "app_version": report.app_version,
        "app_build": report.app_build,
        "os": report.os,
    })
}

/// Far Cooler's own board on this runner: the Main workspace of the first
/// repository whose origin's last path part is `farcooler`.
fn board(svc: &Service) -> Result<Option<Workspace>> {
    for workspace in svc.store.list_workspaces(None)? {
        if workspace.is_main && is_farcooler(&svc.store.get_repository(workspace.repository_id)?.remote_summary) {
            return Ok(Some(workspace));
        }
    }
    Ok(None)
}

/// Whether `remote` (an origin URL) is a `farcooler` repository:
/// `git@github.com:taneliang/farcooler.git`, `https://…/farcooler`.
pub fn is_farcooler(remote: &str) -> bool {
    let last = remote.trim().trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("");
    last.trim_end_matches(".git").eq_ignore_ascii_case("farcooler")
}

/// A new card for `report` on `board`, with its frames in a Finding note.
fn file(svc: &Service, board: &Workspace, report: &pb::CrashReport, digest: &str, count: u32) -> Result<Task> {
    let made = svc.store.create_task(board.id, &title(report), Actor::Runner)?;
    let update = TaskUpdate {
        title: made.title.clone(),
        intent: intent(report, count),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: LABELS.iter().map(|l| l.to_string()).collect(),
        worktree_id: None,
    };
    let task = svc.store.fill_in_new_task(made.id, made.resource_version, &update)?;
    svc.store.add_note(task.id, NoteKind::Finding, Actor::Runner, &finding(report, digest), json!({}))?;
    Ok(task)
}

/// A repeat on `task`'s signature: the count in its intent, and a Finding
/// note when it was closed and this is a newer build than the last report's.
fn raise(svc: &Service, report: &pb::CrashReport, task: &Task, count: u32, previous_build: Option<&str>) -> Result<Task> {
    if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) && previous_build != Some(report.app_build.as_str()) {
        let body = format!(
            "Crashed again on {} after this card was closed: the fix didn’t hold, or this is another bug with the same frames.",
            build_line(report)
        );
        svc.store.add_note(task.id, NoteKind::Finding, Actor::Runner, &body, json!({}))?;
    }
    let update = TaskUpdate {
        title: task.title.clone(),
        intent: intent(report, count),
        acceptance: task.acceptance.clone(),
        constraints: task.constraints.clone(),
        labels: task.labels.clone(),
        worktree_id: task.worktree_id,
    };
    match svc.store.update_task(task.id, task.resource_version, &update) {
        Ok(task) => Ok(task),
        // Someone revised the card meanwhile: theirs stands, and the count
        // is in the store for the next report.
        Err(DomainError::Conflict { .. }) => svc.store.get_task(task.id),
        Err(e) => Err(e),
    }
}

/// The card's title: "iOS: crashes with SIGSEGV in FarCooler at +0x1f2e",
/// shortened to fit 60 characters.
pub fn title(report: &pb::CrashReport) -> String {
    let area = if report.os.starts_with("macOS") { "Mac" } else { "iOS" };
    let verb = if report.kind == "hang" { "hangs" } else { "crashes" };
    let mut base = format!("{area}: {verb}");
    if let Some(reason) = reason(report) {
        base = format!("{base} with {reason}");
    }
    let frame = report.frames.iter().find(|f| f.own).or(report.frames.first());
    let mut tries = vec![];
    if let Some(f) = frame {
        tries.push(format!("{base} in {} at +{:#x}", f.binary, f.offset));
        tries.push(format!("{base} in {}", f.binary));
    }
    tries.push(base.clone());
    tries.into_iter().find(|t| t.chars().count() <= MAX_TITLE).unwrap_or_else(|| base.chars().take(MAX_TITLE).collect())
}

/// What went wrong, in a word: the exception's name, else the signal's,
/// else the Mach exception's.
fn reason(report: &pb::CrashReport) -> Option<String> {
    if !report.exception_name.is_empty() {
        return Some(report.exception_name.clone());
    }
    if let Some(signal) = report.signal {
        return Some(signal_name(signal).map_or_else(|| format!("signal {signal}"), str::to_string));
    }
    report.exception_type.map(|t| exception_type_name(t).map_or_else(|| format!("exception {t}"), str::to_string))
}

fn signal_name(signal: i32) -> Option<&'static str> {
    Some(match signal {
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGBUS",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        _ => return None,
    })
}

fn exception_type_name(t: i32) -> Option<&'static str> {
    Some(match t {
        1 => "EXC_BAD_ACCESS",
        2 => "EXC_BAD_INSTRUCTION",
        3 => "EXC_ARITHMETIC",
        5 => "EXC_SOFTWARE",
        6 => "EXC_BREAKPOINT",
        10 => "EXC_CRASH",
        11 => "EXC_RESOURCE",
        12 => "EXC_GUARD",
        _ => return None,
    })
}

/// "1.4 (1093) on iPhone OS 26.1 (23B85)".
fn build_line(report: &pb::CrashReport) -> String {
    let build = match (report.app_version.is_empty(), report.app_build.is_empty()) {
        (false, false) => format!("{} ({})", report.app_version, report.app_build),
        (false, true) => report.app_version.clone(),
        (true, false) => format!("build {}", report.app_build),
        (true, true) => "an unknown build".to_string(),
    };
    if report.os.is_empty() { build } else { format!("{build} on {}", report.os) }
}

/// The card's intent: what happened, how often, and where the frames are.
pub fn intent(report: &pb::CrashReport, count: u32) -> String {
    let what = if report.kind == "hang" { "hung" } else { "crashed" };
    let mut how = Vec::new();
    if !report.exception_name.is_empty() {
        how.push(format!("an uncaught {}", report.exception_name));
    }
    if let Some(signal) = report.signal {
        how.push(signal_name(signal).map_or_else(|| format!("signal {signal}"), str::to_string));
    }
    if let Some(t) = report.exception_type {
        let name = exception_type_name(t).map_or_else(|| format!("exception {t}"), str::to_string);
        how.push(match report.exception_code {
            Some(code) => format!("{name}, code {code}"),
            None => name,
        });
    }
    let how = if how.is_empty() { String::new() } else { format!(" ({})", how.join(", ")) };
    let times = if count == 1 { "once".to_string() } else { format!("{count} times") };
    format!(
        "The app {what}{how}, reported by the owner’s own device through MetricKit on the launch after.\n\n\
         Seen {times}; most recently on {}.\n\n\
         The faulting thread’s top frames are in the first finding. Only the signature reached the runner: \
         no exception message, thread name or anything typed.",
        build_line(report)
    )
}

/// The first finding: the frames, top first, and where the report came from.
fn finding(report: &pb::CrashReport, digest: &str) -> String {
    let frames: Vec<String> = report
        .frames
        .iter()
        .map(|f| {
            let symbol = if f.symbol.is_empty() { String::new() } else { format!(" {}", f.symbol) };
            let own = if f.own { "  (the app)" } else { "" };
            format!("{:<3} {} +{:#x}{symbol}{own}", f.depth, f.binary, f.offset)
        })
        .collect();
    format!(
        "First reported on {}. Signature {}.\n\n```\n{}\n```\n\nOffsets are into each binary’s text segment; \
         symbolicate the app's own against that build's dSYM.",
        build_line(report),
        &digest[..12],
        frames.join("\n")
    )
}

#[cfg(test)]
#[path = "crash_reports_tests.rs"]
mod tests;
