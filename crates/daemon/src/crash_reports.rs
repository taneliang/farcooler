//! `crash.report` (ov-797): a crash or hang the owner's own phone recorded,
//! kept once per signature and filed on Far Cooler's own board.
//!
//! The phone's MetricKit hands it each diagnostic on the launch after, and
//! the phone sends the signature and nothing else (`AgentKit`'s
//! `CrashSignature`). This end refuses anything that could carry more, and
//! anything that could read as more than a name once it is on a card the
//! coordinator reads (the ov-797 review): each text field has a strict
//! character set (`checked`), so no backtick, newline, Markdown or bidi
//! character reaches a title or a fenced note.
//!
//! **Where it shows: a card on the board, not Needs You.** Needs You is what
//! a person must act on now, and it pushes; a crash is work for an agent to
//! fix, and the same crash every launch would push every launch. A card is
//! where that work is dispatched from, and the coordinator reads the board
//! at every check-in. So a new signature files one card, labeled `found` and
//! `crash`, with its frames in a Finding note; a repeat raises the count in
//! the card's intent and adds nothing; a repeat from another build after the
//! card was closed adds one Finding note with that build's frames, because
//! the fix didn't hold. At most `DAILY_CAP` new signatures a day are kept;
//! past it reports count on one overflow row and file nothing.
//!
//! The card goes on Far Cooler's own board: the Main workspace of a
//! repository whose origin is a `farcooler` repository, the one at the
//! first path when there are several. A runner with none keeps the count and
//! files nothing, since nobody there can fix the app.

use std::sync::Mutex;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::CrashFiling;
use farcooler_store::models::{Actor, Workspace};
use serde_json::json;

use crate::service::Service;
use crate::watch::Watcher;

/// The most frames a signature carries: the phone's top frames and its
/// first own frame.
pub const MAX_FRAMES: usize = 16;
/// The labels on a crash's card.
pub const LABELS: [&str; 2] = ["found", "crash"];
/// New signatures a runner keeps, and files cards for, in a day.
pub const DAILY_CAP: u32 = 10;
/// The longest a card's title gets (`docs`' ticket titles: 60 at most).
const MAX_TITLE: usize = 60;

/// One report at a time, so two of a new signature file one card.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// `crash.report`: count the report, and file or raise its card, in one
/// store transaction (`Store::record_crash`).
pub fn handle(svc: &Service, watcher: &Watcher, req: pb::Request) -> Result<result::Value> {
    let Some(request::Payload::CrashReport(report)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    checked(&report)?;
    let digest = signature(&report);
    let detail = detail(&report).to_string();
    let title = title(&report);
    let labels: Vec<String> = LABELS.iter().map(|l| l.to_string()).collect();
    let finding = finding(&report, &digest);
    let again = again(&report);
    let intent = |count: u32| intent(&report, count);
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let board = board(svc)?;
    let record = svc.store.record_crash(&CrashFiling {
        signature: &digest,
        detail: &detail,
        build: &report.app_build,
        daily_cap: DAILY_CAP,
        board: board.map(|b| b.id),
        title: &title,
        labels: &labels,
        finding: &finding,
        intent: &intent,
        again: &again,
    })?;
    if record.over_cap {
        tracing::warn!(signature = %&digest[..12], overflow = record.count, "crash reported past the day's cap: counted, nothing filed");
    }
    let task = record.task.map(|id| svc.store.get_task(id)).transpose()?;
    if let Some(task) = &task {
        watcher.announce_task_changed(task, None, Actor::Runner);
    }
    tracing::info!(signature = %&digest[..12], count = record.count, card = task.as_ref().map_or("-", |t| t.key.as_str()), "crash reported");
    Ok(result::Value::CrashRecorded(pb::CrashRecorded {
        count: record.count,
        task_key: task.map(|t| t.key).unwrap_or_default(),
    }))
}

/// Refuse a report that carries more than a signature, or isn't one. Every
/// text field has a character set that can't carry a message, a path,
/// Markdown, a newline or a bidi control, so what reaches a card is a name.
pub fn checked(report: &pb::CrashReport) -> Result<()> {
    if !matches!(report.kind.as_str(), "crash" | "hang") {
        return Err(DomainError::InvalidArgument { what: "kind" });
    }
    if report.frames.is_empty() || report.frames.len() > MAX_FRAMES {
        return Err(DomainError::InvalidArgument { what: "frames" });
    }
    for frame in &report.frames {
        // MetricKit's frames have no symbol, and the phone sends none.
        if !binary_name(&frame.binary) || !frame.symbol.is_empty() {
            return Err(DomainError::InvalidArgument { what: "frames" });
        }
    }
    // A class name, never a message: letters, digits, `_`, `.` and `$`.
    let name = &report.exception_name;
    if name.len() > 128 || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')) {
        return Err(DomainError::InvalidArgument { what: "exception_name" });
    }
    if !version_text(&report.app_version, 64) || !version_text(&report.app_build, 64) || !version_text(&report.os, 128) {
        return Err(DomainError::InvalidArgument { what: "build" });
    }
    Ok(())
}

/// A binary's file name: `FarCooler`, `libc++abi.dylib`, `Far Cooler`.
/// ASCII letters, digits, `.`, `_`, `+`, `-` and inner spaces, 1 to 128.
pub fn binary_name(text: &str) -> bool {
    (1..=128).contains(&text.len())
        && text.trim() == text
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-' | ' '))
}

/// A version, build or OS as MetricKit says them: `1.4`, `1093`,
/// `iPhone OS 26.1 (23B85)`. ASCII letters, digits, spaces and `.`, `_`,
/// `(`, `)`, `,`, `-`, at most `max`.
pub fn version_text(text: &str, max: usize) -> bool {
    text.len() <= max && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '(' | ')' | ',' | '-'))
}

/// What makes two reports one: the kind, the exception, and the frames'
/// binaries in order, the app's own among them. Not offsets, which move with
/// every build of the app's binary, and not the build or OS: the same crash
/// on the next build is a repeat, which is what lets a closed card hear that
/// its fix didn't hold. The offsets are in the card's Finding notes.
pub fn signature(report: &pb::CrashReport) -> String {
    let binaries: Vec<&str> = report.frames.iter().map(|f| f.binary.as_str()).collect();
    let key = json!([
        report.kind,
        report.exception_type,
        report.exception_code,
        report.signal,
        report.exception_name,
        binaries,
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

/// Far Cooler's own board on this runner: the Main workspace of the
/// repository whose origin's last path part is `farcooler`, the one whose
/// checkout sorts first when there are several, so the same board every time.
fn board(svc: &Service) -> Result<Option<Workspace>> {
    let mut found = Vec::new();
    for workspace in svc.store.list_workspaces(None)? {
        let repository = svc.store.get_repository(workspace.repository_id)?;
        if workspace.is_main && is_farcooler(&repository.remote_summary) {
            found.push((repository.canonical_git_dir, workspace));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found.into_iter().next().map(|(_, w)| w))
}

/// Whether `remote` (an origin URL) is a `farcooler` repository:
/// `git@github.com:taneliang/farcooler.git`, `https://…/farcooler`.
pub fn is_farcooler(remote: &str) -> bool {
    let last = remote.trim().trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("");
    last.trim_end_matches(".git").eq_ignore_ascii_case("farcooler")
}

/// The Finding a closed card gets when its crash comes back from another
/// build: that build, and its frames.
fn again(report: &pb::CrashReport) -> String {
    format!(
        "Crashed again on {} after this card was closed: the fix didn’t hold, or this is another bug in the same place.\n\n```\n{}\n```",
        build_line(report),
        frames_block(report)
    )
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

/// The frames, one a line, top first: `11  FarCooler +0x403fcc  (the app)`.
/// Safe inside a fence: a binary's name has no backtick (`binary_name`).
fn frames_block(report: &pb::CrashReport) -> String {
    let lines: Vec<String> = report
        .frames
        .iter()
        .map(|f| {
            let own = if f.own { "  (the app)" } else { "" };
            format!("{:<3} {} +{:#x}{own}", f.depth, f.binary, f.offset)
        })
        .collect();
    lines.join("\n")
}

/// The first finding: the frames, top first, and where the report came from.
fn finding(report: &pb::CrashReport, digest: &str) -> String {
    format!(
        "First reported on {}. Signature {}.\n\n```\n{}\n```\n\nOffsets are into each binary’s text segment; \
         symbolicate the app’s own against that build’s dSYM.",
        build_line(report),
        &digest[..12],
        frames_block(report)
    )
}

#[cfg(test)]
#[path = "crash_reports_tests.rs"]
mod tests;
