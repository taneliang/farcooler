use super::*;

/// Only string conversion and the existing C two-call buffer convention live here.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativeWorkflowPresentation(
    mut env: JNIEnv, _class: JClass, request: JString,
) -> jstring {
    guarded(std::ptr::null_mut(), || {
        let Some(request) = c_string(&mut env, &request) else { return std::ptr::null_mut(); };
        ceremony_json(&mut env, |out, capacity| unsafe {
            client::farcooler_client_workflow_presentation(request.as_ptr(), out, capacity)
        })
    })
}

/// The plan's view model (ov-1016): `plan.get`'s answer and the app's context in,
/// the sections and ticket cards out. Pure, like the presentation above.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_farcooler_core_NativeClient_nativePlanView(
    mut env: JNIEnv, _class: JClass, request: JString,
) -> jstring {
    guarded(std::ptr::null_mut(), || {
        let Some(request) = c_string(&mut env, &request) else { return std::ptr::null_mut(); };
        ceremony_json(&mut env, |out, capacity| unsafe {
            client::farcooler_client_plan_view(request.as_ptr(), out, capacity)
        })
    })
}
