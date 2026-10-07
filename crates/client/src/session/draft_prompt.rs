//! `terminal.draft_prompt`: Ask the Orchestrator's paste, from a phone (ov-241),
//! held behind a dialog where the runner can (ov-385).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request, result};
use serde_json::json;
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError, require};

/// What the runner did with a draft.
#[derive(Debug)]
pub enum Drafted {
    /// Pasted into the box, unsent.
    Pasted,
    /// Held behind a dialog, to be pasted once it closes (`draft_hold`).
    Held(pb::DraftHold),
}

impl Session {
    /// Ask the runner to paste `text` into a terminal orchestrator's box,
    /// pressing no Enter. The daemon refuses, typing nothing, unless the pane
    /// is provably an agent with an empty box; the refusal comes back as the
    /// usual error and the phone copies the text instead, as the Mac does.
    /// A runner with `draft_hold` holds it behind a dialog rather than refuse
    /// for one, and says so with the hold.
    pub async fn draft_prompt(&self, terminal: Uuid, text: &str) -> Result<Drafted, SessionError> {
        let hold = self.can(farcooler_protocol::capability::DRAFT_HOLD);
        let payload = request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text.to_string())) }],
            hold_behind_dialog: hold,
        });
        // Named, so a runner that lost the capability refuses rather than
        // dropping the flag and refusing for the dialog with no word why.
        let required = if hold { vec![farcooler_protocol::capability::DRAFT_HOLD.to_string()] } else { Vec::new() };
        match self.value_requiring("terminal.draft_prompt", None, Some(payload), required).await? {
            result::Value::Terminal(_) => Ok(Drafted::Pasted),
            result::Value::DraftHold(h) => Ok(Drafted::Held(h)),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Withdraw the draft `hold` held on `terminal`: answers with the hold as
    /// it now is.
    pub async fn draft_withdraw(&self, terminal: Uuid, hold: Uuid) -> Result<pb::DraftHold, SessionError> {
        let payload = request::Payload::DraftWithdraw(pb::DraftWithdraw {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            hold_id: bytes::Bytes::copy_from_slice(hold.as_bytes()),
        });
        let required = vec![farcooler_protocol::capability::DRAFT_HOLD.to_string()];
        match self.value_requiring("terminal.draft_withdraw", None, Some(payload), required).await? {
            result::Value::DraftHold(h) => Ok(h),
            other => Err(wrong("draft_hold", &other)),
        }
    }

    /// `terminal.compose` (ov-372, ov-367): `text`, with its line breaks, and
    /// `images` (MIME type and bytes) typed into claude's box in a terminal
    /// pane and submitted, or refused with nothing typed, past the same gate
    /// as `terminal tell`. True when claude was working and its own queue
    /// took it; either way only once claude said it took it. A runner with
    /// `agent_compose` alone (ov-372) takes one line and no image, so anything
    /// more is refused here rather than flattened there.
    ///
    /// A runner with `compose_upload` (ov-393) is sent each image first, in
    /// chunks (`actions::stage_compose_image`), and the compose names them,
    /// so they may be `MAX_COMPOSE_UPLOAD_BYTES` together; an older one gets
    /// them inside the request, `MAX_COMPOSE_IMAGE_BYTES` together.
    pub async fn compose(&self, terminal: Uuid, text: &str, images: &[(String, Vec<u8>)]) -> Result<bool, SessionError> {
        use farcooler_protocol::capability::{AGENT_COMPOSE, COMPOSE, COMPOSE_UPLOAD};
        require(self.capabilities(), AGENT_COMPOSE, "terminal.compose")?;
        if !images.is_empty() || text.trim_end().contains(['\n', '\r']) {
            require(self.capabilities(), COMPOSE, "terminal.compose")?;
        }
        let upload = !images.is_empty() && self.can(COMPOSE_UPLOAD);
        images_fit(images, upload)?;
        let mut blocks = vec![pb::AgentPromptBlock { content: Some(Content::Text(text.to_string())) }];
        for (mime, data) in images {
            let content = if upload {
                Content::StagedImage(crate::actions::stage_compose_image(&self.client, terminal, mime, data).await?)
            } else {
                Content::Image(pb::ImageBlock { mime_type: mime.clone(), data: bytes::Bytes::copy_from_slice(data) })
            };
            blocks.push(pb::AgentPromptBlock { content: Some(content) });
        }
        let payload = request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            blocks,
            hold_behind_dialog: false,
        });
        // Named, so a runner that lost `compose_upload` refuses rather than
        // dropping the images it can't read.
        let required = if upload { vec![COMPOSE_UPLOAD.to_string()] } else { Vec::new() };
        // Targeted, so it keeps its order against this terminal's other
        // input and runs beside every other pane's calls.
        match self.value_requiring("terminal.compose", Some(terminal), Some(payload), required).await? {
            result::Value::TerminalTold(told) => Ok(told.queued),
            other => Err(wrong("terminal_told", &other)),
        }
    }
}

/// Refused here as the runner would, before anything is sent: one image
/// past the largest file a paste takes when they're `uploaded` first
/// (`image_too_large`); all of them together past `MAX_COMPOSE_UPLOAD_BYTES`
/// then, else past `MAX_COMPOSE_IMAGE_BYTES`, which one request can carry
/// (`images_too_large`).
pub(crate) fn images_fit(images: &[(String, Vec<u8>)], uploaded: bool) -> Result<(), SessionError> {
    let total = images.iter().map(|(_, data)| data.len()).sum::<usize>();
    let refused = |what: &str, message: &str| SessionError::Refused {
        code: farcooler_protocol::v1::ErrorCode::ResourceConflict as i32,
        retryable: false,
        message: message.into(),
        what: what.into(),
    };
    if uploaded && images.iter().any(|(_, data)| data.len() as u64 > farcooler_protocol::MAX_PASTE_FILE_BYTES) {
        return Err(refused("image_too_large", "an image is too large to send"));
    }
    let cap = if uploaded { farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES } else { farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES };
    if total > cap {
        return Err(refused("images_too_large", "the images are too large to send together"));
    }
    Ok(())
}

/// A hold as both phones read it: `id` as a uuid string, `state` as a word
/// (`waiting`, `sent`, `withdrawn`, `expired`, `failed`), and its times in Unix ms.
pub fn draft_hold_json(hold: &pb::DraftHold) -> serde_json::Value {
    let id = Uuid::from_slice(&hold.id).map(|id| id.to_string()).unwrap_or_default();
    json!({
        "id": id,
        "state": draft_hold_state(hold.state),
        "heldMs": hold.held_ms,
        "expiresMs": hold.expires_ms,
        "endedMs": hold.ended_ms,
    })
}

/// The word for a hold's state; an unknown one from a newer runner reads as
/// ended, never as waiting, so no client waits on it forever.
pub fn draft_hold_state(state: i32) -> &'static str {
    match pb::DraftHoldState::try_from(state) {
        Ok(pb::DraftHoldState::Waiting) => "waiting",
        Ok(pb::DraftHoldState::Sent) => "sent",
        Ok(pb::DraftHoldState::Withdrawn) => "withdrawn",
        Ok(pb::DraftHoldState::Failed) => "failed",
        _ => "expired",
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;

    /// Up to 900 KB of images together goes to a runner without
    /// `compose_upload`; a byte more is refused with the runner's word,
    /// whether one image or several. Uploaded first, up to 50 MB together,
    /// each no more than a paste takes.
    #[test]
    fn images_past_the_cap_together_are_refused() {
        let cap = farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES;
        let image = |n: usize| ("image/png".to_string(), vec![0u8; n]);
        assert!(images_fit(&[image(cap)], false).is_ok());
        assert!(images_fit(&[image(cap / 2), image(cap / 2)], false).is_ok());
        let too_large = |result: Result<(), SessionError>| match result {
            Err(SessionError::Refused { what, .. }) => assert_eq!(what, "images_too_large"),
            other => panic!("{other:?}"),
        };
        for over in [vec![image(cap + 1)], vec![image(cap / 2), image(cap / 2 + 1)]] {
            too_large(images_fit(&over, false));
        }
        let ten_mb = image(10 * 1024 * 1024);
        assert!(images_fit(std::slice::from_ref(&ten_mb), true).is_ok());
        too_large(images_fit(&[ten_mb], false));
        let file = farcooler_protocol::MAX_PASTE_FILE_BYTES as usize;
        assert!(images_fit(&[image(file), image(file), image(file)], true).is_ok());
        match images_fit(&[image(file + 1)], true) {
            Err(SessionError::Refused { what, .. }) => assert_eq!(what, "image_too_large", "one image, its own word"),
            other => panic!("{other:?}"),
        }
        let total = farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES;
        too_large(images_fit(&[image(file), image(file), image(file), image(total - 3 * file + 1)], true));
    }
}
