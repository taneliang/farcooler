use super::{Binding, Error, HookLine, Result, Uuid, terminal};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeOwner {
    pub(super) binding: Binding,
    pub(super) terminal: Uuid,
    pub(super) pane: String,
    pub(super) tty: String,
    /// The agent runs under its hold (ov-1330), which is what ends it. False for an owner an earlier engine launched.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) hold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) supervisor: Option<crate::effect_runner::Owner>,
}
/// Launched keeps the receipt shape previous engines wrote. A never-launched review records its binding
/// directly rather than inventing a pane owner, and can only supply an interrupted exit, never a verdict.
/// The optional failure preserves startup refusal reasons without changing older launched receipts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum NativeSettlement {
    Launched { owner: NativeOwner, hooks: Vec<HookLine>, completed: Option<bool>, #[serde(default, skip_serializing_if = "Option::is_none")] failure: Option<String> },
    NeverLaunched { binding: Binding },
}
impl NativeSettlement {
    pub(super) fn completion(&self, binding: &Binding) -> Result<Option<bool>> {
        match self {
            Self::Launched { owner, completed, .. } if owner.binding == *binding && owner.terminal == terminal(binding) => Ok(*completed),
            Self::NeverLaunched { binding: original } if original == binding => Ok(None),
            _ => Err(Error::Refused("foreign native settlement")),
        }
    }
}
