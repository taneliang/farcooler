//! Who a connection is told a task notice for (ov-705).
//!
//! The `notice` event is the banner a Mac posts for a task when the relay
//! won't reach it. Before ov-705 every connection heard every one, so a
//! member's Mac could post a banner about another member's decision, while
//! the relay pushed that decision to its member alone (ov-633). Now the event
//! goes as the push does:
//!
//! - the owner's connections hear every notice, as they always did;
//! - a member's hear only their own (`Notice.member` is their handle);
//! - a device that was a removed member's hears none.
//!
//! The owner's and nobody's tasks (an empty `member`) go to the owner alone,
//! as their pushes do. Who may answer is unchanged: any member may still
//! answer any item from Needs You, which lists everyone's. On a runner with
//! no members every connection is the owner's, so nothing changes.

use farcooler_protocol::v1::{Event, event::Payload};

use crate::people::Person;

/// Whether `viewer` is told a notice about the task of `member` (empty for
/// the owner's and nobody's).
pub fn admits(viewer: &Person, member: &str) -> bool {
    match viewer {
        Person::Owner => true,
        Person::Member(handle) => !member.is_empty() && handle == member,
        Person::Released => false,
    }
}

/// Whether the connection whose person `viewer` reads as is told `event`:
/// every event but a task notice is everyone's. `viewer` is asked only for a
/// notice, so fleet news costs no lookup.
pub fn admits_event(event: &Event, viewer: impl FnOnce() -> Option<Person>) -> bool {
    let Some(Payload::Notice(notice)) = &event.payload else { return true };
    // A person the store can't read hears nothing rather than everything.
    viewer().is_some_and(|person| admits(&person, &notice.member))
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::{Notice, Terminal};

    fn notice(member: &str) -> Event {
        Event { payload: Some(Payload::Notice(Notice { member: member.into(), ..Notice::default() })), ..Event::default() }
    }

    #[test]
    fn the_owner_hears_every_notice_and_a_member_their_own() {
        let alice = Person::Member("alice".into());
        for member in ["", "alice", "bob"] {
            assert!(admits(&Person::Owner, member), "{member}");
        }
        assert!(admits(&alice, "alice"));
        assert!(!admits(&alice, "bob"));
        assert!(!admits(&alice, ""), "the owner's and nobody's are the owner's to hear");
        assert!(!admits(&Person::Released, "alice"));
        assert!(!admits(&Person::Released, ""));
    }

    #[test]
    fn fleet_news_is_everyones_and_reads_no_person() {
        let fleet = Event { payload: Some(Payload::TerminalChanged(Terminal::default())), ..Event::default() };
        assert!(admits_event(&fleet, || panic!("no lookup for fleet news")));
        assert!(!admits_event(&notice("bob"), || Some(Person::Member("alice".into()))));
        assert!(admits_event(&notice("bob"), || Some(Person::Owner)));
        assert!(!admits_event(&notice(""), || None));
    }
}
