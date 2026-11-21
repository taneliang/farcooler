//! Board summaries sent to the relay.

/// One board's glance, as the relay stores and forwards it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoardGlance {
    pub workspace: String,
    #[serde(rename = "needsYou")]
    pub needs_you: u32,
    pub now: Vec<TicketGlance>,
    /// Absent when nothing is queued in the plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// A ticket in Now: its key, and its stage as the store spells it
/// (`building`, `reviewing`, `needs_you`, `landing`). The apps say the word.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TicketGlance {
    pub ticket: String,
    pub stage: &'static str,
}
