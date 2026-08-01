#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, Role, Sequenced};

    #[test]
    fn a_message_round_trips_through_one_line() {
        let msg = ShimMessage::Events {
            events: vec![Sequenced {
                seq: 3,
                event: AgentEvent::Message { role: Role::Agent, text: "hi".into() },
            }],
        };
        let line = encode_line(&msg).expect("encodes");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: ShimMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, msg);
    }

    #[test]
    fn the_daemon_subscribes_from_a_cursor() {
        // Reconnect after a daemon restart is the whole reason this field
        // exists: the shim outlived the daemon and still holds the history.
        let line = encode_line(&DaemonMessage::Subscribe { from_seq: 12 }).expect("encodes");
        let back: DaemonMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, DaemonMessage::Subscribe { from_seq: 12 });
    }

    #[test]
    fn an_unknown_message_is_an_error_not_a_silent_drop() {
        assert!(decode_line::<DaemonMessage>(r#"{"kind":"from_the_future"}"#).is_err());
    }
}
