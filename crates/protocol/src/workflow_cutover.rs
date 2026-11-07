//! Closed vocabulary of the single `workflow.cutover` method. No principal,
//! grant, quiescence claim or actor appears here: the daemon decides who the
//! caller is and proves quiescence itself.
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Cutover {
    /// Read-only: phase, incarnation, schema and artifact state.
    Inspect,
    /// On a live daemon: drain, mark the attempt and stop. In recovery: capture
    /// the baseline and intent, upgrade and freeze.
    Prepare { attempt: String },
    /// End an uncommitted attempt, restoring the original database if it was upgraded.
    Recover,
    /// Take the closed backup and map a frozen attempt in one proof. After this
    /// the attempt recovers forward only.
    Commit,
    /// Re-check a committed transfer against its original baseline.
    Verify,
    /// Release a verified committed transfer.
    Release,
    /// Record the terminal disposition of a released attempt.
    Finish,
}
pub const METHOD: &str = "workflow.cutover";

pub fn parse(raw: &str) -> Result<Cutover, &'static str> {
    if raw.len() > MAX_BYTES { return Err("oversized cutover command"); }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| "invalid JSON")?;
    let op: Cutover = serde_json::from_value(value.clone()).map_err(|_| "invalid op")?;
    if let Cutover::Prepare { attempt } = &op {
        if attempt.is_empty() || attempt.len() > 100 || !attempt.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return Err("invalid attempt"); }
    }
    if serde_json::to_value(&op).map_err(|_| "invalid encoding")? != value { return Err("unknown cutover fields"); }
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cutover_vocabulary_is_closed_and_carries_no_authority() {
        assert_eq!(parse(r#"{"op":"Inspect"}"#).unwrap(), Cutover::Inspect);
        assert_eq!(parse(r#"{"op":"Recover"}"#).unwrap(), Cutover::Recover);
        assert_eq!(parse(r#"{"op":"Commit"}"#).unwrap(), Cutover::Commit);
        assert!(matches!(parse(r#"{"op":"Prepare","attempt":"a-1"}"#).unwrap(), Cutover::Prepare { .. }));
        for bad in [r#"{"op":"Commit","backup":"x"}"#, r#"{"op":"Inspect","quiescent":true}"#, r#"{"op":"Prepare","attempt":""}"#,
            r#"{"op":"Prepare","attempt":"a/b"}"#, r#"{"op":"Release","actor":"owner"}"#, "{}", "[]"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse(&"x".repeat(2000)).is_err());
    }
}
