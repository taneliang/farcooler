//! Scripts choose a closed move; only the reducer can apply it. All answers, including timeouts,
//! are inputs, so replay never consults a process or the filesystem.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookPoint {
    #[serde(rename = "group.splits")] GroupSplits,
    #[serde(rename = "ci.result")] CiResult,
    #[serde(rename = "run.fails")] RunFails,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptPolicy {
    pub point: HookPoint,
    /// An absolute file and digest of its exact bytes. Executed directly, never through a shell.
    pub script: String,
    pub digest: String,
    pub timeout_ms: u64,
    pub max_calls: u8,
    pub fallback: HookMove,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookMove {
    Leave, Accept, OwnGroup, Rejoin { group: String }, Retry,
    SendBack { reason: String }, Cancel { reason: String }, Relabel { label: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HookDecision { pub schema: String, pub call: String, #[serde(flatten)] pub choice: HookMove }
impl<'de> Deserialize<'de> for HookDecision {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        // Value's map deserializer keeps the last duplicate, hiding an ambiguous schema,
        // call or move before we can validate it. Reject duplicates at the receipt boundary.
        struct DecisionObject;
        impl<'de> serde::de::Visitor<'de> for DecisionObject {
            type Value = serde_json::Map<String, serde_json::Value>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a decision object with unique fields")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut object = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) { return Err(M::Error::custom("duplicate decision field")); }
                    object.insert(key, map.next_value()?);
                }
                Ok(object)
            }
        }
        let mut object = deserializer.deserialize_map(DecisionObject)?;
        let schema = object.remove("schema").and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| s == "hook-decision/v1").ok_or_else(|| D::Error::custom("unknown decision schema"))?;
        let call = object.remove("call").and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| valid_id(s)).ok_or_else(|| D::Error::custom("missing call"))?;
        let value = serde_json::Value::Object(object);
        let choice: HookMove = serde_json::from_value(value.clone()).map_err(D::Error::custom)?;
        // Serde's internally tagged unit variants otherwise ignore extra fields.
        if serde_json::to_value(&choice).map_err(D::Error::custom)? != value {
            return Err(D::Error::custom("unknown decision fields"));
        }
        Ok(Self { schema, call, choice })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookGroup { pub item: String, pub phase: Phase, pub stage: usize, pub members: Vec<Member> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookInput {
    pub schema: String, pub point: HookPoint, pub call: String, pub revision: u64, pub now_ms: i64,
    pub subject: String, pub fence: u64, pub policy_pin: String,
    pub label: Option<String>, pub candidate: Option<String>, pub failure: Option<String>, pub evidence: Option<Evidence>,
    pub groups: Vec<HookGroup>, pub members: Vec<Member>, pub retries: u8, pub allowed: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookResult { Receipt { raw: String, manifest: String }, Timeout, Missing, Invalid }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookCall {
    pub policy: String, pub input: HookInput, pub deadline_ms: i64, pub attempt: u8,
    /// The CI observation is suspended until the rule answers; its authenticated binding is retained.
    pub observation: Option<Box<Input>>,
}
pub(crate) fn validate_policies(policies: &BTreeMap<String, ScriptPolicy>) -> Result<(), Refusal> {
    if policies.len() > 3 { return Err(Refusal::Definition); }
    let mut points = Vec::new();
    for (name, p) in policies {
        if !valid_id(name) || points.contains(&p.point) || !p.script.starts_with('/') || p.script.len() > 4096
            || p.script.contains('\0') || !p.digest.starts_with("wf1:") || p.digest.len() != 68
            || !p.digest[4..].bytes().all(|b| b.is_ascii_hexdigit())
            || p.timeout_ms == 0 || p.timeout_ms > 600_000 || p.max_calls == 0 || p.max_calls > 3
            || !matches!((&p.point, &p.fallback), (HookPoint::RunFails, HookMove::Leave)
                | (HookPoint::CiResult, HookMove::Accept) | (HookPoint::GroupSplits, HookMove::OwnGroup))
        { return Err(Refusal::Definition); }
        points.push(p.point);
    }
    Ok(())
}
/// Firing is bounded per policy and run. Its immutable input also pins the call to its subject's fence.
pub(crate) fn fire(item: &mut Item, id: &str, input: &Input, point: HookPoint,
    observation: Option<Box<Input>>, effects: &mut Vec<Effect>) -> bool
{
    if item.deciding.is_some() { return false; }
    let Some((name, policy)) = item.definition.script_policies.iter().find(|(_, p)| p.point == point) else { return false };
    let count = item.hook_calls.get(name).copied().unwrap_or(0);
    if count >= policy.max_calls { return false; }
    let (name, policy) = (name.clone(), policy.clone());
    item.hook_calls.insert(name.clone(), count + 1);
    let call = digest(&("hook-call/v1", &input.id, id, &name, item.fence, count));
    let allowed: Vec<&str> = match point {
        HookPoint::RunFails => {
            let mut moves = vec!["leave"];
            if item.group_of.is_none() && item.review_limit_failure().is_none() {
                if item.retries < item.definition.execution_limits().retries { moves.push("retry"); }
                if item.sent_back.len() < usize::from(item.definition.send_back_limit())
                    && item.definition.stages[..=item.stage].iter().rev().take_while(|s| !lands(&s.kind))
                        .any(|s| s.kind == StageKind::Build) { moves.push("send_back"); }
            }
            if item.members.is_empty() || item.pushed.is_none() { moves.push("cancel"); }
            moves
        }
        HookPoint::CiResult => vec!["accept", "relabel"],
        HookPoint::GroupSplits => vec!["own_group", "rejoin"],
    };
    let document = HookInput { schema: "hook-input/v1".into(), point, call,
        revision: input.expected_revision, now_ms: input.now_ms, subject: id.into(), fence: item.fence,
        policy_pin: item.policy_pin.clone(), label: observation.as_ref().and_then(|i| match &i.command {
            Command::Observe { outcome: Outcome::Passed(_), .. } => Some("passed".into()),
            Command::Observe { outcome: Outcome::Rejected(_), .. } => Some("rejected".into()),
            Command::Observe { outcome: Outcome::Labeled(label, _) | Outcome::LabeledWithReason { label, .. }
                | Outcome::CiResult { label, .. }, .. } => Some(label.clone()), _ => None }), candidate: item.candidate.clone(), failure: item.failure.clone(),
        evidence: item.last_evidence.clone(), groups: Vec::new(), members: item.members.clone(), retries: item.retries,
        allowed: allowed.into_iter().map(Into::into).collect() };
    item.deciding = Some(HookCall { policy: name, deadline_ms: input.now_ms.saturating_add(policy.timeout_ms as i64),
        attempt: 1, input: document.clone(), observation });
    crate::reducer::emit(item, id, input, EffectKind::Decide { policy, input: Box::new(document) }, effects);
    true
}
