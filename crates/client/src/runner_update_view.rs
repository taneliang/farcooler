//! Progress shown while an app update waits to replace its runner.

#[derive(serde::Serialize)]
pub struct LandingWaitView {
    event: &'static str,
    pub message: &'static str,
}

pub fn landing_wait() -> LandingWaitView {
    LandingWaitView {
        event: "landing_wait",
        message: "Waiting for in-flight landing checks or pushes to finish before replacing the local runner.",
    }
}
