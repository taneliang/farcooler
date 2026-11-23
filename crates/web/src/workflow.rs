/// Local pure bridge; the authenticated transport adapter must supply trusted capabilities.
/// Uses the same versioned response and stable error enum as C/JNI, with no JS reducer.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = workflowPresentation))]
pub fn presentation(request: &str) -> String {
    farcooler_client::workflow_presentation::json(request)
}

/// The plan's view model (ov-1016): the same pure function the phones and the
/// Mac call, so the browser sorts and words the plan with no copy of its own.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = planView))]
pub fn plan_view(request: &str) -> String {
    farcooler_client::plan_view::json(request)
}

/// Account-owned recovery copy presentation from the shared client.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = recoveryView))]
pub fn recovery_view(count: usize, current: bool) -> String {
    farcooler_client::recovery_view::view(count, current).to_string()
}

/// Needs You's pure cross-runner presenter, shared with the native view-model bridge.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = needsYouView))]
pub fn needs_you_view(request: &str) -> String {
    farcooler_client::needs_you_view::json(request)
}

#[cfg(test)]
mod needs_you_tests {
    #[test]
    fn wasm_entry_matches_native_view_model_envelope() {
        let request = include_str!("../../../test/fixtures/needs-you-aggregate.json");
        assert_eq!(super::needs_you_view(request), farcooler_client::plan_view::json(request));
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = workspaceLayout))]
pub fn workspace_layout(request: &str) -> String {
    farcooler_client::workspace_layout::json(request)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(js_name = workspaceOrchestrator))]
pub fn workspace_orchestrator(plan: &str) -> Option<String> {
    farcooler_client::workspace_layout::orchestrator(plan)
}
