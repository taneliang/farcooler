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
