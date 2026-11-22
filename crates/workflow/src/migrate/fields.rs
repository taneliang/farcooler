//! Every `Item` field and what `migrate` does with it (docs/engine-pipeline.md 11.1). A test serializes an item
//! with every field set and checks its keys against this table, and `items.rs` destructures `Item` with no `..`,
//! so a new field fails to build or fails the test until it has a decision here.

/// What becomes of one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Mapped into the pipeline; the text says where.
    Mapped(&'static str),
    /// Not carried: the text says why nothing is lost.
    Dropped(&'static str),
    /// A value here refuses the migration (or quarantines the item); the text is the reason.
    Refused(&'static str),
}

pub const ITEM_FIELDS: &[(&str, Decision)] = &[
    ("start_load", Decision::Dropped("Automatic start evidence remains in the event log")),
    ("execution", Decision::Refused("a read-time view that is never stored; a stored one is refused")),
    ("definition", Decision::Dropped("the published layout is in force for every token; no pinning")),
    ("pin", Decision::Dropped("Published::digest names the one workflow in force")),
    ("policy_pin", Decision::Mapped("the running effect's OpRef::spec is the digest of the operation it runs; Evidence::spec")),
    ("dependencies", Decision::Mapped("Token::dependencies")),
    ("resource", Decision::Dropped("an operation's resources come from the adapter's catalog")),
    ("stage", Decision::Mapped("the station of Location, by step id; a step with no station refuses")),
    ("iteration", Decision::Dropped("derived from visits")),
    ("attempt", Decision::Mapped("Work::attempt")),
    ("fence", Decision::Mapped("Token::fence and Batch::fence")),
    ("phase", Decision::Mapped("Location, by 11.2 and the dispatch status of 10.1")),
    ("run", Decision::Mapped("Decision::run at a decision station; Work::run is the pending effect, which binds answers")),
    ("restart", Decision::Dropped("the legacy log keeps the Start that replaced the run")),
    ("reuse_checkout", Decision::Refused("the replacement must verify and reuse its legacy checkout first")),
    ("carried_progress", Decision::Dropped("the pipeline derives usage from the retained visits")),
    ("history", Decision::Dropped("the legacy log keeps the events of each earlier run")),
    ("reason", Decision::Dropped("the legacy log keeps the observation or verdict that produced the reason")),
    ("pending", Decision::Mapped("Work::pending, or a queued token with its outbox row suppressed")),
    ("candidate", Decision::Mapped("coding data candidate; a member's in Membership::data")),
    ("last_evidence", Decision::Mapped("Token::evidence")),
    ("stopping", Decision::Mapped("Work::stopping of a Stopping or Settling token")),
    ("stopped_from", Decision::Dropped("the legacy log keeps the run targeted by the stop")),
    ("rework_history", Decision::Dropped("its back edges are the visits written from back_edges; the reasons stay in the legacy log")),
    ("back_edges", Decision::Mapped("closed visits ending in the station's back label, which its rework or returns policy counts")),
    ("retries", Decision::Dropped("retries have no cap; the legacy log keeps them")),
    ("members", Decision::Mapped("Batch::members, with each candidate in Membership::data")),
    ("group_of", Decision::Mapped("Batch::station is the members' join step; the archive keeps the group's name")),
    ("pushed", Decision::Mapped("a Crossing on the batch and its members, and coding data pushed")),
    ("handed", Decision::Mapped("an archived batch's members")),
    ("since_ms", Decision::Mapped("Batch::since_ms and ArchivedBatch::since_ms")),
    ("grouped_in", Decision::Mapped("Location::InBatch")),
    ("wait_since", Decision::Mapped("Work::wait_since at a wait station")),
    ("models", Decision::Mapped("Token::models")),
    ("tree", Decision::Mapped("coding data tree")),
    ("sent_back", Decision::Mapped("closed visits ending sent, with the reason")),
    ("brief", Decision::Mapped("Token::brief")),
    ("follows", Decision::Refused("the pipeline has no followers yet (ov-1266 report, item 15)")),
    ("followers", Decision::Refused("the pipeline has no followers yet (ov-1266 report, item 15)")),
    ("cancel_reason", Decision::Mapped("the archive entry's reason, or a stop's Then::Close reason")),
    ("closed", Decision::Mapped("the archive entry's reason; a landed close waiting for its stop refuses")),
    ("failure", Decision::Mapped("Token::failure, and the hold's reason for a failed item")),
    ("ci_reruns", Decision::Mapped("Work::reruns at the CI step it was spent on; elsewhere it is history")),
    ("ci_retry", Decision::Mapped("Work::again and coding data ci_retry; a rerun still being requested quarantines")),
    ("deciding", Decision::Refused("a decision script is running for it, and the pipeline has no scripts")),
    ("hook_calls", Decision::Dropped("script call counts; the pipeline has no scripts")),
    ("hook_refused", Decision::Refused("a script refused its decision, and the pipeline has no scripts")),
    ("group_failure", Decision::Mapped("coding data failure on the batch")),
    ("landed", Decision::Mapped("coding data pushed of the archived token")),
];
