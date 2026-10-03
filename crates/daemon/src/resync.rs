//! A terminal client that falls behind, and how it is made whole again
//! (ov-118).
//!
//! `Runtime::attach` reads a pane's output and hands it to an `AttachSink`;
//! `TerminalSink` is the one `terminal.attach` uses, which puts it on the
//! connection's bounded push queue. What lives here is what happens when that
//! queue is full: the backlog is dropped, and once the connection is draining
//! again the client is sent a `Gap` and a fresh picture of the pane behind a
//! full reset (`reset_then`), in place of everything it missed.

use uuid::Uuid;

/// How long a resync that is owed waits for the output to reach ground before
/// it is taken anyway. See `Runtime::attach`.
pub(crate) const RESYNC_GROUND_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Where `Runtime::attach` puts a pane's output. See `attach`.
pub trait AttachSink: Send {
    /// Take the opening picture. It is never dropped for being large or for
    /// being raced by live output. False when nothing will read it.
    fn open(&mut self, picture: Vec<u8>) -> bool;

    /// Take a run of output. While the sink is behind it drops this and says
    /// so again.
    fn output(&mut self, bytes: Vec<u8>) -> Taken;

    /// Resolves when a sink that fell behind could deliver a picture now
    /// rather than queue it behind a stall. False when nothing will read it.
    fn caught_up(&self) -> impl std::future::Future<Output = bool> + Send;

    /// Put this picture in place of everything that was dropped. False when
    /// nothing will read it.
    fn resync(&mut self, picture: Vec<u8>) -> bool;
}

/// What became of one `AttachSink::output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    Sent,
    /// Dropped, along with whatever the sink was still holding. Every byte
    /// from here is dropped too, until a `resync`.
    Behind,
    /// Nobody is reading. Stop.
    Gone,
}

/// A replay that also clears whatever a client was holding: RIS (`ESC c`)
/// ahead of the picture, inside its synchronized update when it has one.
///
/// The opening replay is written into a fresh emulator, so it can lean on
/// starting from nothing. A resync is written into one that has been fed a
/// stream with a hole in it, and the hole can end anywhere — inside a CSI, an
/// OSC, a UTF-8 character, on the alternate screen with the mouse on. The reset
/// starts with ESC, which abandons any sequence in progress, and then puts back
/// every mode, both screens, the scroll region and the history to their
/// defaults: the same emulator a reattach builds, without the reattach. And it
/// is history the clear has to reach — the replay brings its own scrollback, so
/// without the reset the client's history would hold everything twice.
///
/// Inside the synchronized update, not ahead of it, so the emptied screen is
/// never a frame anyone sees.
pub(crate) fn reset_then(picture: Vec<u8>) -> Vec<u8> {
    const OPEN: &[u8] = b"\x1b[?2026h";
    const RESET: &[u8] = b"\x1bc";
    let at = if picture.starts_with(OPEN) { OPEN.len() } else { 0 };
    let mut out = Vec::with_capacity(picture.len() + RESET.len());
    out.extend_from_slice(&picture[..at]);
    out.extend_from_slice(RESET);
    out.extend_from_slice(&picture[at..]);
    out
}

/// How much of a pane's output one connection may have waiting for its client.
///
/// `MAX_UNACKED_TERMINAL_BYTES`, the protocol's own figure for how far a client
/// may fall behind a terminal: a second of a fast build log, about four
/// thousand lines of `yes`. Past it, `TerminalSink` drops the backlog and
/// resyncs rather than holding more.
pub const TERMINAL_BACKLOG_BYTES: usize = farcooler_protocol::MAX_UNACKED_TERMINAL_BYTES as usize;

/// `terminal.attach`'s side of `Runtime::attach`: a pane's output, as
/// `TerminalFrame`s on this connection's push queue.
///
/// **What happens when the client falls behind** (ov-118). The queue refuses a
/// frame that would take it past `TERMINAL_BACKLOG_BYTES`. This sink then
/// throws away everything still queued — the client is better served by the
/// pane as it is now than by a second of output it has not read — and drops
/// every byte after it until the connection is draining again. Then the
/// runtime captures the pane, and two frames go out in place of everything
/// dropped:
///
/// - a `Gap` (`GAP_REASON_CLIENT_TOO_SLOW`) saying where the stream resumes and
///   how many bytes are missing. No client reads it yet — each one ignores
///   every frame kind but `Output` — and it is sent anyway, because it is the
///   protocol's own word for exactly this and costs a few bytes.
/// - an `Output` carrying a reset and the full replay (`runtime::reset_then`).
///   This is the part the clients act on, and they need no change for it: iOS,
///   Android and the Mac all feed `Output` bytes to the same `farcooler_vt`
///   emulator, and a reset followed by the replay leaves it holding what a
///   fresh attach would. (The Mac streams through `Runtime::stream` on a pipe,
///   not this path, so it never sees one.)
///
/// Bytes are never dropped from the middle of what a client is fed without a
/// resync after them, so an escape sequence cut in half is always followed by
/// the reset that abandons it.
///
/// Sequence numbers go on counting what was dropped, so `start_sequence` jumps
/// across a gap by exactly `Gap.lost_bytes`.
pub(crate) struct TerminalSink {
    push: farcooler_transport::PushSender,
    terminal_id: bytes::Bytes,
    epoch: u64,
    /// The byte offset of the next run, which is what makes
    /// `TerminalOutput.start_sequence` mean anything. Counted here rather than
    /// in `Runtime`, because it is a property of this attachment and not of
    /// the pane: two clients watching the same pane attached at different
    /// moments.
    sequence: u64,
    /// Bytes dropped since the last thing the client was sent, while behind.
    lost: Option<u64>,
}

impl TerminalSink {
    pub(crate) fn new(push: farcooler_transport::PushSender, terminal: Uuid, epoch: u64) -> Self {
        Self {
            push,
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            epoch,
            sequence: 0,
            lost: None,
        }
    }

    fn frame(&self, kind: farcooler_protocol::v1::terminal_frame::Kind) -> farcooler_protocol::v1::Event {
        farcooler_protocol::v1::Event {
            event_id: farcooler_protocol::ids::new_id(),
            // Zero, like every other event. The offset that matters for a
            // terminal is a byte count, and it rides in the frame where it has
            // a documented unit.
            sequence: 0,
            payload: Some(farcooler_protocol::v1::event::Payload::TerminalFrame(
                farcooler_protocol::v1::TerminalFrame {
                    terminal_id: self.terminal_id.clone(),
                    epoch: self.epoch,
                    kind: Some(kind),
                },
            )),
        }
    }

    /// A picture as the frames that carry it, in order, from `start`.
    ///
    /// Cut at `MAX_TERMINAL_PAYLOAD_BYTES`, because a picture can be far larger
    /// than one envelope may be: a colored 2000-line history captures at 2.6
    /// MiB, and `MAX_CONTROL_ENVELOPE_BYTES` refuses anything over 1 MiB — a
    /// refusal `serve_connection` answered by dropping the connection, so a big
    /// enough pane could never be opened from a phone at all. No header is
    /// needed to put it back together: every client writes `Output` payloads
    /// into one byte stream in arrival order, so consecutive frames ARE the
    /// picture. A picture under `SYNCHRONIZED_REPLAY_BUDGET` is still one
    /// synchronized update across its frames, since the emulator holds the
    /// update open between feeds; one over it was never synchronized.
    fn picture_frames(&self, start: u64, picture: Vec<u8>) -> Vec<farcooler_protocol::v1::Event> {
        picture
            .chunks(farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES)
            .scan(start, |at, chunk| {
                let frame = self.output_frame(*at, chunk.to_vec());
                *at += chunk.len() as u64;
                Some(frame)
            })
            .collect()
    }

    fn output_frame(&self, start: u64, bytes: Vec<u8>) -> farcooler_protocol::v1::Event {
        self.frame(farcooler_protocol::v1::terminal_frame::Kind::Output(
            farcooler_protocol::v1::TerminalOutput {
                start_sequence: start,
                payload: bytes::Bytes::from(bytes),
            },
        ))
    }
}

impl AttachSink for TerminalSink {
    fn open(&mut self, picture: Vec<u8>) -> bool {
        let start = self.sequence;
        self.sequence += picture.len() as u64;
        let frames = self.picture_frames(start, picture);
        self.push.push_pinned(frames)
    }

    fn output(&mut self, bytes: Vec<u8>) -> Taken {
        use farcooler_transport::Pushed;

        let len = bytes.len() as u64;
        let start = self.sequence;
        self.sequence += len;
        if let Some(lost) = self.lost.as_mut() {
            *lost += len;
            return Taken::Behind;
        }
        match self.push.push(self.output_frame(start, bytes)) {
            Pushed::Queued => Taken::Sent,
            Pushed::Closed => Taken::Gone,
            Pushed::Full => {
                // Everything queued goes, and is counted: the client resumes
                // from the first byte of the first frame dropped here.
                let dropped: u64 = self
                    .push
                    .clear()
                    .into_iter()
                    .filter_map(|event| match event.payload {
                        Some(farcooler_protocol::v1::event::Payload::TerminalFrame(
                            farcooler_protocol::v1::TerminalFrame {
                                kind: Some(farcooler_protocol::v1::terminal_frame::Kind::Output(output)),
                                ..
                            },
                        )) => Some(output.payload.len() as u64),
                        _ => None,
                    })
                    .sum();
                tracing::debug!(dropped = dropped + len, "a client fell behind a terminal; resyncing");
                self.lost = Some(dropped + len);
                Taken::Behind
            }
        }
    }

    fn caught_up(&self) -> impl std::future::Future<Output = bool> + Send {
        let push = self.push.clone();
        async move { push.drained().await }
    }

    fn resync(&mut self, picture: Vec<u8>) -> bool {
        let lost = self.lost.take().unwrap_or(0);
        let start = self.sequence;
        self.sequence += picture.len() as u64;
        let gap = self.frame(farcooler_protocol::v1::terminal_frame::Kind::Gap(
            farcooler_protocol::v1::Gap {
                resumed_at_sequence: start,
                lost_bytes: Some(lost),
                reason: farcooler_protocol::v1::GapReason::ClientTooSlow as i32,
            },
        ));
        let mut frames = vec![gap];
        frames.extend(self.picture_frames(start, picture));
        // Pinned: live output that races it can be refused behind it, never
        // put in place of it. See `push`.
        self.push.replace(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::replay;
    use farcooler_tmux::windows::PaneModes;
    use farcooler_vt::Terminal;

    fn primary() -> PaneModes {
        PaneModes { cursor_visible: true, wrap: true, ..Default::default() }
    }

    fn opened(bytes: &[u8], rows: u16) -> Terminal {
        let mut t = Terminal::new(80, rows);
        t.feed(bytes);
        t
    }

    fn row(t: &Terminal, index: usize) -> String {
        let snapshot = farcooler_vt::grid::snapshot(t);
        snapshot.rows[index].cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string()
    }

    fn history_size(t: &Terminal) -> u32 {
        farcooler_vt::grid::snapshot(t).history_size
    }

    /// **A resync leaves a client holding exactly what a fresh attach would**
    /// (ov-118). The client has been fed a stream with a hole in it: here it
    /// stopped on the alternate screen, mouse on, inside a half-written CSI,
    /// with an hour of history of its own. What the daemon sends in place of
    /// the hole has to wipe all of that, or the history arrives twice and the
    /// next byte is read as the end of a sequence that was never finished.
    #[test]
    fn a_resync_repaints_a_client_as_if_it_had_just_attached() {
        let history: Vec<String> = (1..=30).map(|i| format!("old{i}")).collect();
        let screen: Vec<String> = (1..=10).map(|i| format!("now{i}")).collect();
        let picture =
            replay(Some(primary()), Some(&history.join("\n")), Some(&screen.join("\n")), Some((3, 9)));

        let fresh = opened(&picture, 10);

        let mut stale = Terminal::new(80, 10);
        for i in 1..=50 {
            stale.feed(format!("stale{i}\r\n").as_bytes());
        }
        // Alternate screen, mouse reporting, and a CSI cut off mid-parameter.
        stale.feed(b"\x1b[?1049h\x1b[?1003h\x1b[?1006hTUI\x1b[3");
        stale.feed(&reset_then(picture.clone()));

        let snapshot = |t: &Terminal| farcooler_vt::grid::snapshot(t);
        assert_eq!(history_size(&stale), history_size(&fresh), "the history, once");
        for index in 0..10 {
            assert_eq!(row(&stale, index), row(&fresh, index), "row {index}");
        }
        assert_eq!(stale.mode(), fresh.mode(), "the modes a fresh emulator would have");
        assert_eq!(snapshot(&stale).cursor_column, snapshot(&fresh).cursor_column);
        assert_eq!(snapshot(&stale).cursor_row, snapshot(&fresh).cursor_row);
    }

    /// Inside the synchronized update, so the reset is never shown on its own.
    #[test]
    fn the_reset_is_inside_the_picture_s_synchronized_update() {
        let picture = replay(Some(primary()), None, Some("now"), Some((0, 0)));
        let reset = reset_then(picture.clone());
        assert!(reset.starts_with(b"\x1b[?2026h\x1bc"), "{reset:?}");
        assert_eq!(&reset[b"\x1b[?2026h\x1bc".len()..], &picture[b"\x1b[?2026h".len()..]);
    }

    /// **A client that stops reading a terminal holds a bounded backlog, and
    /// is resynced when it comes back** (ov-118). 32 MiB of output into an
    /// attachment nobody reads: the queue never holds more than its bound plus
    /// one frame, where it used to hold all 32. When a reader appears, what it
    /// gets first is a `Gap` naming every dropped byte, then a reset and a
    /// fresh picture — never the middle of the stream.
    #[tokio::test]
    async fn a_stalled_terminal_client_holds_a_bounded_backlog_and_is_resynced() {
        use farcooler_protocol::v1::{event::Payload, terminal_frame::Kind};

        const CHUNK: usize = 16 * 1024;
        let (push, mut pushes) = farcooler_transport::push_queue(TERMINAL_BACKLOG_BYTES);
        let mut sink = TerminalSink::new(push.clone(), Uuid::now_v7(), 3);

        let mut most = 0;
        let mut behind = 0;
        let total = 32 * 1024 * 1024;
        for _ in 0..(total / CHUNK) {
            match sink.output(vec![b'y'; CHUNK]) {
                Taken::Sent => {}
                Taken::Behind => behind += 1,
                Taken::Gone => panic!("nobody hung up"),
            }
            most = most.max(push.queued_bytes());
        }
        let bound = TERMINAL_BACKLOG_BYTES + CHUNK + 1024;
        assert!(behind > 0, "the scenario should actually fall behind");
        assert!(most <= bound, "a client that never read held {most} bytes, more than {bound}");

        // Behind, and staying behind until the connection asks for more.
        let early = tokio::time::timeout(std::time::Duration::from_millis(50), sink.caught_up()).await;
        assert!(early.is_err(), "nothing is reading, so a picture would only queue");

        let reader = tokio::spawn(async move {
            let gap = pushes.recv().await.expect("a gap");
            let picture = pushes.recv().await.expect("a picture");
            (gap, picture, pushes)
        });
        assert!(sink.caught_up().await);
        assert!(sink.resync(b"\x1b[?2026h\x1bcPICTURE\x1b[?2026l".to_vec()));
        let (gap, picture, _still_reading) = reader.await.unwrap();

        let Some(Payload::TerminalFrame(gap)) = gap.payload else { panic!("not a frame") };
        let Some(Kind::Gap(gap)) = gap.kind else { panic!("not a gap: {:?}", gap.kind) };
        assert_eq!(gap.reason, farcooler_protocol::v1::GapReason::ClientTooSlow as i32);
        // Nothing was read before the stall, so everything was lost.
        assert_eq!(gap.lost_bytes, Some(total as u64));
        assert_eq!(gap.resumed_at_sequence, total as u64);

        let Some(Payload::TerminalFrame(picture)) = picture.payload else { panic!("not a frame") };
        let Some(Kind::Output(picture)) = picture.kind else { panic!("not output") };
        assert_eq!(picture.start_sequence, total as u64);
        assert!(picture.payload.starts_with(b"\x1b[?2026h\x1bc"));

        // And live output flows again behind it.
        assert_eq!(sink.output(b"next".to_vec()), Taken::Sent);
    }

    /// **A picture larger than the backlog bound arrives whole, however live
    /// output races it** (ov-118 review). A colored 2000-line history captures
    /// at 2.6 MiB, more than twice `TERMINAL_BACKLOG_BYTES`. It used to be the
    /// first thing the next live chunk evicted, leaving the client a `Gap` it
    /// ignores and a frozen screen — and had it survived, its one 2.6 MiB frame
    /// was over the envelope limit, which closed the connection.
    ///
    /// Here the picture is queued, then 3 MiB of live output races it with
    /// nobody reading. What the client then reads is the gap, then the whole
    /// picture in frames the wire accepts, and fed to the emulator every client
    /// runs, it repaints.
    #[tokio::test]
    async fn a_picture_bigger_than_the_bound_survives_the_output_that_races_it() {
        use crate::runtime::replay;
        use farcooler_protocol::v1::{event::Payload, terminal_frame::Kind};

        // Every cell its own color, as in the measured pane.
        let history: Vec<String> = (1..=2000)
            .map(|i| (0..220).map(|c| format!("\x1b[3{}m{}", (i + c) % 8, (b'a' + (c % 26) as u8) as char)).collect())
            .collect();
        let screen: Vec<String> = (1..=10).map(|i| format!("now{i}")).collect();
        let modes = farcooler_tmux::windows::PaneModes { cursor_visible: true, wrap: true, ..Default::default() };
        let picture = reset_then(replay(
            Some(modes),
            Some(&history.join("\n")),
            Some(&screen.join("\n")),
            Some((0, 9)),
        ));
        assert!(picture.len() > 2 * 1024 * 1024, "the picture should be the measured size: {}", picture.len());

        let (push, mut pushes) = farcooler_transport::push_queue(TERMINAL_BACKLOG_BYTES);
        let mut sink = TerminalSink::new(push.clone(), Uuid::now_v7(), 1);

        // Fall behind, catch up, and queue the picture.
        while sink.output(vec![b'y'; 16 * 1024]) != Taken::Behind {}
        let waiting = tokio::spawn(async move {
            let first = pushes.recv().await;
            (first, pushes)
        });
        assert!(sink.caught_up().await);
        assert!(sink.resync(picture.clone()));
        // The parked receiver takes the first frame, the gap, and stops.
        let (gap, mut pushes) = waiting.await.unwrap();
        let Some(Payload::TerminalFrame(gap)) = gap.and_then(|e| e.payload) else { panic!("no gap") };
        assert!(matches!(gap.kind, Some(Kind::Gap(_))));

        // Live output, racing it, with nobody reading.
        let mut refused = 0;
        for _ in 0..(3 * 1024 * 1024 / (16 * 1024)) {
            if sink.output(vec![b'z'; 16 * 1024]) == Taken::Behind {
                refused += 1;
            }
        }
        assert!(refused > 0, "the race should actually overflow the bound");

        // What the client reads: the picture, whole and in order, in frames
        // the wire carries.
        let mut received = Vec::new();
        while received.len() < picture.len() {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), pushes.recv())
                .await
                .expect("the rest of the picture never came: live output evicted it")
                .expect("more of the picture");
            let encoded = farcooler_protocol::v1::WireEnvelope {
                protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                message_id: farcooler_protocol::ids::new_id(),
                body: Some(farcooler_protocol::v1::wire_envelope::Body::Event(event.clone())),
            };
            farcooler_protocol::framing::encode(&encoded).expect("a frame the wire accepts");
            let Some(Payload::TerminalFrame(frame)) = event.payload else { panic!("not a frame") };
            match frame.kind {
                Some(Kind::Gap(_)) => assert!(received.is_empty(), "a gap inside the picture"),
                Some(Kind::Output(output)) => received.extend_from_slice(&output.payload),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(received.len(), picture.len());
        assert!(received == picture, "the picture arrived changed");

        // And it repaints the emulator every client runs.
        let mut client = farcooler_vt::Terminal::new(80, 10);
        client.feed(b"\x1b[?1049hfrozen\x1b[3");
        client.feed(&received);
        let snapshot = farcooler_vt::grid::snapshot(&client);
        let top: String = snapshot.rows[0].cells.iter().map(|c| c.ch).collect();
        assert_eq!(top.trim_end(), "now1");
        assert!(snapshot.history_size > 1000, "the history came with it");
    }
}
