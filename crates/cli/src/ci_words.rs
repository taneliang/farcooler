//! CI as a person reads it (ov-309, ov-306): one set of words for a train's
//! CI in `plan`, and for a page's CI reference in `page show`, the same words
//! the apps draw.

use farcooler_protocol::v1 as pb;

/// A CI read's status: "Passed", "Failed", "Running", "Queued", "No runs
/// yet", or "CI unknown" when `gh` couldn't say.
pub(crate) fn status_word(status: i32) -> &'static str {
    match pb::BoardCiStatus::try_from(status) {
        Ok(pb::BoardCiStatus::Passed) => "Passed",
        Ok(pb::BoardCiStatus::Failed) => "Failed",
        Ok(pb::BoardCiStatus::Running) => "Running",
        Ok(pb::BoardCiStatus::Queued) => "Queued",
        Ok(pb::BoardCiStatus::None) => "No runs yet",
        _ => "CI unknown",
    }
}

/// Whether it needs the owner: a failed run does.
pub(crate) fn needs_attention(read: &pb::BoardCiRead) -> bool {
    read.status == pb::BoardCiStatus::Failed as i32
}

/// "Failed · 2 of 15 jobs failed", "Running · 9 of 15 jobs done", "Passed ·
/// 15 jobs", or the status alone with no jobs read.
pub(crate) fn summary(read: &pb::BoardCiRead) -> String {
    let word = status_word(read.status);
    let total = read.jobs.len();
    if total == 0 {
        return word.to_string();
    }
    let n = |state: &str| read.jobs.iter().filter(|j| j.state == state).count();
    let jobs = |k: usize| if k == 1 { "job" } else { "jobs" };
    match pb::BoardCiStatus::try_from(read.status) {
        Ok(pb::BoardCiStatus::Failed) => {
            let failed = n("failed") + n("canceled");
            format!("{word} · {failed} of {total} {} failed", jobs(total))
        }
        Ok(pb::BoardCiStatus::Running | pb::BoardCiStatus::Queued) => {
            let done = total - n("running") - n("queued");
            format!("{word} · {done} of {total} {} done", jobs(total))
        }
        _ => format!("{word} · {total} {}", jobs(total)),
    }
}

/// The read a subject names: `sha:<sha>` matches a read of the same commit
/// however short either SHA was written; `run:<id>` and `main` match exactly.
pub(crate) fn read_for<'a>(ci: &'a [pb::BoardCiRead], subject: &str) -> Option<&'a pb::BoardCiRead> {
    let subject = subject.to_ascii_lowercase();
    ci.iter().find(|r| r.subject == subject).or_else(|| {
        let sha = subject.strip_prefix("sha:")?;
        ci.iter().find(|r| {
            let theirs = r.subject.strip_prefix("sha:").unwrap_or_default();
            (!theirs.is_empty() && (theirs.starts_with(sha) || sha.starts_with(theirs))) || (!r.sha.is_empty() && r.sha.starts_with(sha))
        })
    })
}

/// A train's state, as a person reads it.
pub(crate) fn train_state_word(state: i32) -> &'static str {
    match pb::BoardTrainState::try_from(state) {
        Ok(pb::BoardTrainState::Integrating) => "Integrating",
        Ok(pb::BoardTrainState::Gating) => "Gating",
        Ok(pb::BoardTrainState::Pushed) => "Pushed",
        Ok(pb::BoardTrainState::Green) => "Green",
        Ok(pb::BoardTrainState::Red) => "Red",
        Ok(pb::BoardTrainState::Landed) => "Landed",
        Ok(pb::BoardTrainState::Dropped) => "Dropped",
        _ => "Unknown",
    }
}

/// A train's stored word, as `--json` and the apps' fixture carry it.
pub(crate) fn train_state_key(state: i32) -> &'static str {
    match pb::BoardTrainState::try_from(state) {
        Ok(pb::BoardTrainState::Integrating) => "integrating",
        Ok(pb::BoardTrainState::Gating) => "gating",
        Ok(pb::BoardTrainState::Pushed) => "pushed",
        Ok(pb::BoardTrainState::Green) => "green",
        Ok(pb::BoardTrainState::Red) => "red",
        Ok(pb::BoardTrainState::Landed) => "landed",
        Ok(pb::BoardTrainState::Dropped) => "dropped",
        _ => "unknown",
    }
}

/// A CI read's stored word.
pub(crate) fn status_key(status: i32) -> &'static str {
    match pb::BoardCiStatus::try_from(status) {
        Ok(pb::BoardCiStatus::Passed) => "passed",
        Ok(pb::BoardCiStatus::Failed) => "failed",
        Ok(pb::BoardCiStatus::Running) => "running",
        Ok(pb::BoardCiStatus::Queued) => "queued",
        Ok(pb::BoardCiStatus::None) => "none",
        _ => "unknown",
    }
}
