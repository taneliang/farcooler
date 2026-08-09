//! The half of a conversation no vendor owns: what is queued, and whose turn it is.
//!
//! Everything here was inside `RunningSession`, reachable only through a live
//! ACP subprocess. It is Far Cooler's own behavior rather than any protocol's —
//! a prompt held HERE is one that can still be shown, rewritten, or taken back,
//! which is the entire reason it is not handed to the agent to sit on.

use std::collections::VecDeque;

use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities};
use farcooler_agent_core::event::{AgentEvent, PromptImage, QueuedPrompt, Role};

pub struct ChatSession<B: AgentBackend> {
    backend: B,
    /// Prompts written while a turn was running, in the order they were
    /// written. See `QueuedPrompt`.
    queue: VecDeque<QueuedPrompt>,
    /// Names the queued prompts. Monotonic and never reused, so an edit or a
    /// cancel cannot land on a different message than the one being looked at.
    next_queue_id: u64,
    /// Whether a turn is running.
    ///
    /// `RunningSession` tracked this as the ACP request id it was waiting on.
    /// A bool says the same thing, and says it for a backend whose turn ids
    /// look nothing like ACP's.
    in_flight: bool,
}

impl<B: AgentBackend> ChatSession<B> {
    pub fn new(backend: B) -> Self {
        ChatSession { backend, queue: VecDeque::new(), next_queue_id: 0, in_flight: false }
    }

    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub fn turn_in_flight(&self) -> bool {
        self.in_flight
    }

    /// The backend, for the callers that still speak to it directly.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Send a prompt, or hold it until the current turn ends.
    ///
    /// An agent takes one turn at a time, and a prompt sent during one is not a
    /// second conversation — it is at best ignored and at worst interleaved.
    /// This used to fire regardless, so a message typed while the agent was
    /// working looked sent and might simply never have been.
    pub async fn prompt(
        &mut self,
        text: &str,
        images: Vec<PromptImage>,
    ) -> Result<Vec<AgentEvent>, BackendError> {
        // Anything already waiting goes first, even with no turn running.
        //
        // A send that failed leaves its prompt at the head of the queue with
        // nothing in flight — see `send_next_queued`. Without this, the message
        // being written now would overtake it and the conversation would carry
        // the user's words in an order they never wrote them in.
        if !self.in_flight && !self.queue.is_empty() {
            let mut events = self.send_next_queued().await;
            self.push(text, images);
            events.push(self.queue_event());
            return Ok(events);
        }
        if self.in_flight {
            self.push(text, images);
            return Ok(vec![self.queue_event()]);
        }
        self.backend.prompt(text, &images).await?;
        self.in_flight = true;
        Ok(Vec::new())
    }

    /// Rewrite a prompt that has not been sent. Unknown ids are ignored: the
    /// turn may have ended and sent it between the click and the message.
    pub fn edit_queued(&mut self, id: &str, text: &str) -> Vec<AgentEvent> {
        let Some(entry) = self.queue.iter_mut().find(|q| q.id == id) else { return Vec::new() };
        entry.text = text.to_string();
        vec![self.queue_event()]
    }

    /// Take back a prompt that has not been sent.
    pub fn cancel_queued(&mut self, id: &str) -> Vec<AgentEvent> {
        let before = self.queue.len();
        self.queue.retain(|q| q.id != id);
        if self.queue.len() == before { return Vec::new() }
        vec![self.queue_event()]
    }

    /// Send a queued prompt NOW, without waiting for the turn to end.
    ///
    /// Not the default, because the reason the queue exists is that a message
    /// you can still see and still edit is worth more than one already gone.
    /// This is the deliberate escape hatch: you looked at what you wrote and
    /// decided it should interrupt. "Stop, do it this way" is worth nothing
    /// once the wrong thing is done.
    ///
    /// `in_flight` is deliberately left alone. A steering prompt joins the
    /// running turn rather than starting its own — clearing it would leave that
    /// turn with nothing to report its end, and the pane would say Working
    /// forever.
    pub async fn steer_queued(&mut self, id: &str) -> Result<Vec<AgentEvent>, BackendError> {
        let Some(index) = self.queue.iter().position(|q| q.id == id) else {
            return Ok(Vec::new());
        };
        let queued = self.queue.remove(index).expect("index just found");

        // A backend with real steering gets told this is steering. One without
        // it gets an ordinary prompt, which is what the old code did inline and
        // is the closest thing available — the difference reaches the UI
        // through `native_steer` rather than being hidden here.
        let sent = if self.backend.capabilities().native_steer {
            self.backend.steer(&queued.text, &queued.images).await
        } else {
            self.backend.prompt(&queued.text, &queued.images).await
        };
        if let Err(e) = sent {
            self.queue.insert(index, queued);
            return Err(e);
        }

        Ok(vec![
            self.queue_event(),
            AgentEvent::Message { role: Role::User, text: queued.text, parent: None },
        ])
    }

    /// Wait for the backend, and drain the queue when a turn ends.
    pub async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        let events = self.backend.next_events().await?;
        Ok(self.absorb(events).await)
    }

    /// Fold events the caller obtained itself into this session's own state.
    ///
    /// Split out of `next_events` for the callers that cannot use it. A
    /// `select!` branch must receive and handle separately — receiving is
    /// cancellation safe and handling is not, because handling answers the
    /// agent's `fs/*` requests and being cancelled mid-answer leaves a file
    /// written with the agent waiting forever. Those callers drive the backend
    /// directly and pass what comes out through here, so the queue is drained
    /// on exactly the same signal either way.
    pub async fn absorb(&mut self, mut events: Vec<AgentEvent>) -> Vec<AgentEvent> {
        if events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })) {
            self.in_flight = false;
            // The turn is over, so anything held back can go now. This is the
            // only moment it is safe to send one.
            events.extend(self.send_next_queued().await);
        }
        events
    }

    fn push(&mut self, text: &str, images: Vec<PromptImage>) {
        let id = self.next_queue_id;
        self.next_queue_id += 1;
        self.queue.push_back(QueuedPrompt { id: id.to_string(), text: text.to_string(), images });
    }

    fn queue_event(&self) -> AgentEvent {
        AgentEvent::PromptQueue { items: self.queue.iter().cloned().collect() }
    }

    /// The next queued prompt, sent now that the turn is over.
    ///
    /// Returns the user message as well, because this is the moment it truly
    /// becomes part of the conversation — before this it was only waiting.
    async fn send_next_queued(&mut self) -> Vec<AgentEvent> {
        let Some(next) = self.queue.pop_front() else { return Vec::new() };
        let mut events = vec![self.queue_event()];
        match self.backend.prompt(&next.text, &next.images).await {
            Ok(()) => {
                self.in_flight = true;
                events.push(AgentEvent::Message {
                    role: Role::User,
                    text: next.text,
                    parent: None,
                });
                events
            }
            Err(_) => {
                // Put it back rather than lose it silently. A prompt that
                // cannot be sent is still a prompt the user wrote.
                //
                // `in_flight` stays false, which is the honest state: no turn
                // is in flight. That means the NEXT message goes straight out
                // ahead of this one, so the failed send is retried here first —
                // otherwise a stuck entry sits in the queue forever, since
                // nothing but a turn ending ever drains it and no turn is
                // running to end.
                self.queue.push_front(next);
                vec![self.queue_event()]
            }
        }
    }
}

#[cfg(test)]
impl<B: AgentBackend> ChatSession<B> {
    fn backend(&self) -> &B {
        &self.backend
    }

    fn queued_ids(&self) -> Vec<String> {
        self.queue.iter().map(|q| q.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::EndReason;

    /// A backend that records what it was asked to do and says nothing back.
    ///
    /// The point of the seam: every invariant below used to need a live `npx`
    /// subprocess to exercise, and now needs nothing at all.
    struct Fake {
        caps: Capabilities,
        sent: Vec<String>,
        steered: Vec<String>,
        /// Emitted once, then the fake goes quiet.
        end_turn: bool,
        /// Every `prompt` fails, to exercise the requeue path.
        failing: bool,
    }

    impl Fake {
        fn new(native_steer: bool) -> Self {
            Fake {
                caps: Capabilities { native_steer, ..Capabilities::acp() },
                sent: Vec::new(),
                steered: Vec::new(),
                end_turn: true,
                failing: false,
            }
        }

        fn failing() -> Self {
            Fake { failing: true, ..Fake::new(false) }
        }
    }

    impl AgentBackend for Fake {
        fn capabilities(&self) -> Capabilities {
            self.caps
        }
        async fn prompt(&mut self, text: &str, _: &[PromptImage]) -> Result<(), BackendError> {
            if self.failing {
                return Err(BackendError::Closed);
            }
            self.sent.push(text.to_string());
            Ok(())
        }
        async fn steer(&mut self, text: &str, _: &[PromptImage]) -> Result<(), BackendError> {
            self.steered.push(text.to_string());
            Ok(())
        }
        async fn answer(&mut self, _: &str, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        async fn set_config_option(&mut self, _: &str, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
        async fn cancel(&mut self) -> Result<(), BackendError> {
            Ok(())
        }
        async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
            if self.end_turn {
                self.end_turn = false;
                return Ok(vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }]);
            }
            Ok(vec![AgentEvent::SessionInfo { title: "quiet".into() }])
        }
    }

    #[tokio::test]
    async fn a_prompt_with_no_turn_running_goes_straight_out() {
        let mut s = ChatSession::new(Fake::new(false));
        let events = s.prompt("hello", Vec::new()).await.unwrap();
        assert!(events.is_empty(), "nothing to report: it was simply sent");
        assert_eq!(s.backend().sent, vec!["hello".to_string()]);
        assert!(s.turn_in_flight());
    }

    #[tokio::test]
    async fn a_prompt_during_a_turn_is_queued_not_sent() {
        // The bug this exists to prevent: a message typed while the agent was
        // working looked sent and might never have been.
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        let events = s.prompt("second", Vec::new()).await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string()]);
        assert!(
            matches!(events.as_slice(), [AgentEvent::PromptQueue { items }] if items.len() == 1)
        );
    }

    #[tokio::test]
    async fn a_queued_prompt_is_sent_when_the_turn_ends() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("second", Vec::new()).await.unwrap();
        let events = s.next_events().await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string(), "second".to_string()]);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })));
        assert!(events.iter().any(
            |e| matches!(e, AgentEvent::Message { role: Role::User, text, .. } if text == "second")
        ));
    }

    #[tokio::test]
    async fn steering_uses_the_backend_when_it_has_one() {
        let mut s = ChatSession::new(Fake::new(true));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("correction", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.steer_queued(&queued[0]).await.unwrap();
        assert_eq!(s.backend().steered, vec!["correction".to_string()]);
        assert_eq!(s.backend().sent, vec!["first".to_string()], "steering is not a new turn");
    }

    #[tokio::test]
    async fn steering_falls_back_to_an_ordinary_prompt_without_native_support() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("correction", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.steer_queued(&queued[0]).await.unwrap();
        assert!(s.backend().steered.is_empty());
        assert_eq!(s.backend().sent, vec!["first".to_string(), "correction".to_string()]);
    }

    #[tokio::test]
    async fn steering_does_not_end_the_turn() {
        // A steering prompt joins the running turn. Clearing turn state would
        // leave that turn with nothing to report its end, and the pane would
        // say Working forever.
        let mut s = ChatSession::new(Fake::new(true));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("correction", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.steer_queued(&queued[0]).await.unwrap();
        assert!(s.turn_in_flight(), "the original turn is still running");
    }

    #[tokio::test]
    async fn an_edited_prompt_keeps_its_place() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("typo", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        let events = s.edit_queued(&queued[0], "fixed");
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::PromptQueue { items }] if items[0].text == "fixed"
        ));
    }

    #[tokio::test]
    async fn a_cancelled_prompt_is_never_sent() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("regret", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.cancel_queued(&queued[0]);
        s.next_events().await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string()]);
    }

    #[tokio::test]
    async fn withdrawing_one_prompt_does_not_renumber_the_rest() {
        // Ids are monotonic and never reused, so a cancel cannot land on a
        // different message than the one being looked at. Renumbering on
        // removal would break exactly that.
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("second", Vec::new()).await.unwrap();
        s.prompt("third", Vec::new()).await.unwrap();
        let before = s.queued_ids();
        s.cancel_queued(&before[0]);
        assert_eq!(s.queued_ids(), vec![before[1].clone()], "the survivor keeps its own id");
    }

    #[tokio::test]
    async fn an_unknown_queue_id_is_ignored_rather_than_an_error() {
        // The turn may have ended and sent it between the click and the message.
        let mut s = ChatSession::new(Fake::new(false));
        assert!(s.edit_queued("nope", "x").is_empty());
        assert!(s.cancel_queued("nope").is_empty());
        assert!(s.steer_queued("nope").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_send_puts_the_prompt_back_rather_than_losing_it() {
        // A prompt that cannot be sent is still a prompt the user wrote, and
        // in_flight must stay false so the next message retries this one first
        // instead of leaving a stuck entry nothing will ever drain.
        let mut s = ChatSession::new(Fake::failing());
        assert!(s.prompt("doomed", Vec::new()).await.is_err());
        assert!(!s.turn_in_flight());

        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("second", Vec::new()).await.unwrap();
        s.backend_mut().failing = true;
        s.next_events().await.unwrap();
        assert!(!s.turn_in_flight(), "no turn is running, and saying otherwise would be a lie");
        assert_eq!(s.queued_ids().len(), 1, "the prompt is back at the head of the queue");
    }
}
