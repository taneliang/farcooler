use serde::{Deserialize, Serialize};
use crate::{RunnerEntry, signed};

/// The reply: the runners a trusted device granted, addressed to one ceremony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Device signature and runner attestations for a teammate browser (ov-1020).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<signed::Proof>,
    pub v: u8,
    pub ceremony: String,
    pub account: String,
    pub channel: String,
    /// The fingerprint of the Key A this reply answers, so it can be consumed
    /// only by the device that asked.
    pub target: String,
    pub runners: Vec<RunnerEntry>,
    /// Whose runners these are, when they aren't the new device's own
    /// account's: the granting admin's email, so a teammate's device asks
    /// "Join …'s runners?" before it adds them (ov-525). Absent on a
    /// same-account reply, and from older builds.
    ///
    /// A name alone proves nothing. Native QR replies show it as a claim;
    /// the browser also requires `proof` and the relay's independent authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,
}
