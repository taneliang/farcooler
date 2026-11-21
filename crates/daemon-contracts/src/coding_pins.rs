//! Pinned operations shared by landing drafts and daemon validation.

use std::collections::BTreeMap;

use farcooler_protocol::workflow_config::CodingConfig;
use farcooler_workflow::{OperationSpec, digest};

pub fn operation(config: &CodingConfig, version: &str, argument: &str) -> OperationSpec {
    OperationSpec { adapter_version: version.into(), arguments: vec![config.pin(), argument.into()], environment: BTreeMap::new() }
}

pub fn landing_resource(config: &CodingConfig) -> Option<String> {
    config.landing.as_ref().map(|l| digest(&("landing-target/v1", &l.repository, &l.target)))
}

pub fn wait_operation(config: &CodingConfig, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec {
    OperationSpec { adapter_version: version.into(), environment: BTreeMap::new(),
        arguments: vec![config.pin(), argument.into(), every_ms.to_string(), patience_ms.to_string()] }
}
