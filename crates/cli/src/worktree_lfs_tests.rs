use farcooler_transport::ClientError;

use super::*;

/// A runner that advertises no capabilities and fails the test if it is asked.
struct OldRunner;

impl DispatchLink for OldRunner {
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        panic!("sent {}", req.method)
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

/// The refusal is a sentence a person can read, and the Mac, iOS and Android
/// apps switch on its `code:` word and never show it. The Mac's
/// `LfsNoticeTests.anOldRunnersRefusalReadsAsTheSharedSentence` feeds the
/// Mac's parser exactly the stderr this prints.
#[tokio::test]
async fn an_old_runner_is_refused_in_a_sentence_before_the_wire() {
    let refused = hydrate_lfs(&mut OldRunner, uuid::Uuid::nil()).await.expect_err("an older runner is refused");
    assert_eq!(refused.to_string(), "This runner needs an update to download large files again.");
    assert_eq!(
        crate::error_code_lines(refused.as_ref(), true),
        ["code: capability-unsupported"],
        "a script can still tell"
    );
}
