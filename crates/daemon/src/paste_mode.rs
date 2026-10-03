//! Whether a pane's program asked for bracketed paste, read from its output,
//! for a tmux too old to say.
//!
//! tmux 3.7 reports it as `#{bracket_paste_flag}`. tmux 3.4 (Ubuntu 24.04's)
//! renders that as nothing, so `answer_wake` could never prove it there and
//! never typed. What a program asks for, it asks for in its output: DECSET
//! 2004 (`ESC[?2004h`) turns bracketing on and DECRST 2004 (`ESC[?2004l`)
//! turns it off. tmux 3.4 changes the mode on nothing else a program writes
//! except RIS (`ESC c`), which resets it off (tmux `input.c`).
//!
//! So this follows a pane's output and keeps the last of those it saw. That
//! counts only while it has seen every byte since: the record starts
//! `Unknown`, becomes known at the first 2004 set or reset or RIS it reads,
//! and is worth nothing once its stream breaks. Bytes it never saw (before it
//! started following, or across a gap) can only leave it `Unknown`, never
//! wrongly `On`.
//!
//! **Where the bytes come from.** The daemon reads no pane's output of its
//! own accord: the pane's fanout (`fanout`) exists only while a client
//! watches, and exits `IDLE_GRACE` after the last leaves. So on a tmux
//! without the flag, the daemon subscribes to the fanout of each agent pane
//! itself (`Service::follow_paste_mode`): right after it starts the agent,
//! and on each sample for a running one it isn't following yet (a pane
//! started before this daemon). A subscription that fails is retried after
//! `RETRY_FIRST`, doubling to `RETRY_MOST`.
//!
//! **What following costs**, below tmux 3.7 only, and only for a pane whose
//! terminal runs an agent preset in its TUI (never a shell, a chat or a
//! Changes pane): the pane's `pipe-pane` and its `farcoolerd --fanout`
//! process stay up for as long as the pane lives, where before they lived
//! only while a client watched. That is one more process per agent pane,
//! a second copy of everything the agent writes, and `pipe-pane`, which a
//! client opening the pane already takes today, held for good: a pipe
//! someone sets on an agent pane by hand is replaced. In the daemon a
//! follower holds `READ_CHUNK` bytes and a parser, nothing else.
//!
//! **Two corners left open.** A subscription starts at a chunk boundary,
//! not a sequence boundary, and the record reads its first byte as if at
//! ground. Joined inside a DCS string, where tmux reads `ESC[?2004h` as
//! text, it would count that as a set: a program would have to write a
//! 2004 inside a DCS (tmux passthrough, say) in the instant the daemon
//! subscribes. And the fanout drops anything starting like its size marker
//! (`ESC P >farcooler-size`) for up to 64 bytes, so a program that wrote
//! that prefix unterminated and then a DECRST in those bytes would hide it.
//! Both need a program writing sequences no agent writes; neither is
//! guarded against.
//!

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// What the stream says about bracketed paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing seen yet that sets it, or something seen that could be read
    /// two ways.
    Unknown,
    On,
    Off,
}

/// tmux 3.4's input parser (`input.c`), cut down to the states that decide
/// whether a byte can change bracketed paste.
///
/// Where this and tmux could part ways, it says `Unknown` rather than guess:
/// a private-mode sequence tmux would refuse or might read differently (odd
/// parameters, too many of them), and a CSI written inside a DCS string, which
/// tmux reads as CSI if it gave up on the string after its five-second timer.
#[derive(Debug, Clone)]
pub struct Decsets {
    state: State,
    /// The CSI's parameter bytes, `?` aside.
    params: Vec<u8>,
    /// Whether the CSI started with `?` and has no intermediates.
    private: bool,
    mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParameter,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParameter,
    DcsIntermediate,
    DcsIgnore,
    DcsString,
    /// ESC inside a DCS string: `\` ends the string, anything else is more
    /// of it.
    DcsEscape,
    Osc,
    /// APC, PM, SOS and tmux's rename string: left only by ESC, CAN or SUB.
    OtherString,
}

/// How much of a followed stream the daemon reads at a time.
const READ_CHUNK: usize = 1024;
/// How long after a failed subscription the pane is tried again, at first.
pub const RETRY_FIRST: Duration = Duration::from_secs(1);
/// The longest wait between tries, which the doubling stops at.
pub const RETRY_MOST: Duration = Duration::from_secs(60);

/// tmux's `param_buf` is 64 bytes; it discards a sequence that fills it.
const LONGEST_PARAMS: usize = 62;
/// tmux's `param_list` holds 24.
const MOST_PARAMS: usize = 23;

impl Default for Decsets {
    fn default() -> Self {
        Self { state: State::Ground, params: Vec::new(), private: false, mode: Mode::Unknown }
    }
}

impl Decsets {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Read the next bytes of the stream, in order.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        use State::*;
        // A DCS string is the one state ESC, CAN and SUB don't leave.
        match self.state {
            DcsString => {
                if b == 0x1b {
                    self.state = DcsEscape;
                }
                return;
            }
            DcsEscape => {
                self.state = match b {
                    b'\\' => Ground,
                    // tmux reads these as CSI or as RIS if it timed the
                    // string out, and as text if it didn't.
                    b'[' | b'c' => {
                        self.mode = Mode::Unknown;
                        DcsString
                    }
                    _ => DcsString,
                };
                return;
            }
            _ => {}
        }
        match b {
            0x18 | 0x1a => {
                self.state = Ground;
                return;
            }
            0x1b => {
                self.state = Escape;
                return;
            }
            _ => {}
        }
        self.state = match (self.state, b) {
            (Ground, _) => Ground,
            (Escape, 0x20..=0x2f) => EscapeIntermediate,
            (Escape, b'P') => DcsEntry,
            (Escape, b'[') => {
                self.params.clear();
                self.private = false;
                CsiEntry
            }
            (Escape, b']') => Osc,
            (Escape, b'X' | b'^' | b'_' | b'k') => OtherString,
            (Escape, b'c') => {
                self.mode = Mode::Off;
                Ground
            }
            (Escape, 0x30..=0x7e) => Ground,
            (EscapeIntermediate, 0x30..=0x7e) => Ground,
            (CsiEntry, 0x20..=0x2f) => CsiIntermediate,
            (CsiEntry, 0x30..=0x3b) => {
                self.params.push(b);
                CsiParameter
            }
            (CsiEntry, 0x3c..=0x3f) => {
                self.private = b == b'?';
                CsiParameter
            }
            (CsiEntry | CsiParameter, 0x40..=0x7e) => {
                self.dispatch(b);
                Ground
            }
            (CsiParameter, 0x20..=0x2f) => {
                self.private = false;
                CsiIntermediate
            }
            (CsiParameter, 0x30..=0x3b) => {
                self.params.push(b);
                CsiParameter
            }
            (CsiParameter, 0x3c..=0x3f) => CsiIgnore,
            (CsiIntermediate, 0x30..=0x3f) => CsiIgnore,
            (CsiIntermediate | CsiIgnore, 0x40..=0x7e) => Ground,
            (DcsEntry, 0x20..=0x2f) | (DcsParameter, 0x20..=0x2f) => DcsIntermediate,
            (DcsEntry, 0x30..=0x39 | 0x3b | 0x3c..=0x3f) => DcsParameter,
            (DcsEntry | DcsParameter, b':') => DcsIgnore,
            (DcsParameter, 0x30..=0x39 | 0x3b) => DcsParameter,
            (DcsParameter, 0x3c..=0x3f) | (DcsIntermediate, 0x30..=0x3f) => DcsIgnore,
            (DcsEntry | DcsParameter | DcsIntermediate, 0x40..=0x7e) => DcsString,
            (Osc, 0x07) => Ground,
            // C0 controls run without leaving a sequence, and every other
            // byte is more of whatever this state is collecting.
            (state, _) => state,
        };
    }

    /// A CSI ended with `final_byte`: if it's DECSET or DECRST, apply each
    /// 2004 in it, in order, as tmux does.
    fn dispatch(&mut self, final_byte: u8) {
        let on = match final_byte {
            b'h' => true,
            b'l' => false,
            _ => return,
        };
        if !self.private {
            return;
        }
        let Some(params) = private_modes(&self.params) else {
            self.mode = Mode::Unknown;
            return;
        };
        for p in params {
            if p == Some(2004) {
                self.mode = if on { Mode::On } else { Mode::Off };
            }
        }
    }
}

/// A private mode sequence's parameters as tmux reads them (`input_split`),
/// or `None` where tmux would refuse the sequence or might read it
/// differently from this.
fn private_modes(params: &[u8]) -> Option<Vec<Option<u32>>> {
    if params.len() > LONGEST_PARAMS {
        return None;
    }
    let mut out = Vec::new();
    for token in params.split(|&b| b == b';') {
        if out.len() >= MOST_PARAMS || token.contains(&b':') || token.len() > 9 {
            return None;
        }
        if token.is_empty() {
            out.push(None);
            continue;
        }
        // Only digits are left: `<=>?` sent the sequence to `CsiIgnore`.
        out.push(std::str::from_utf8(token).ok()?.parse().ok());
    }
    Some(out)
}

/// One pane's record: its mode as its stream says, for one program.
pub struct Record {
    /// `#{pane_pid}` when the record started: the program it's about.
    pub pid: u32,
    decsets: Mutex<Decsets>,
    /// False once the stream has ended: the record has stopped seeing bytes.
    live: AtomicBool,
    /// The task reading the stream, stopped when the record is ended.
    reader: Mutex<Option<tokio::task::AbortHandle>>,
}

impl Record {
    pub fn new(pid: u32) -> Arc<Self> {
        Arc::new(Self {
            pid,
            decsets: Mutex::new(Decsets::default()),
            live: AtomicBool::new(true),
            reader: Mutex::new(None),
        })
    }

    /// Follow `stream`, a subscription to the pane's fanout, until it ends
    /// or the record is ended.
    pub fn read(self: &Arc<Self>, mut stream: tokio::net::UnixStream) {
        use tokio::io::AsyncReadExt;
        let me = self.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; READ_CHUNK];
            // The fanout's own size markers out, so one written between two
            // halves of a program's sequence can't split it.
            let mut strip = crate::fanout::MarkerStrip::holding();
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => me.feed(&strip.strip(&buf[..n])),
                }
            }
            me.live.store(false, Ordering::SeqCst);
        });
        *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
        // Ended before the task was recorded: stop it now.
        if !self.is_live() {
            task.abort();
        }
    }

    pub fn feed(&self, bytes: &[u8]) {
        self.decsets.lock().unwrap_or_else(|e| e.into_inner()).feed(bytes);
    }

    /// Stop following: nothing after this is seen.
    pub fn end(&self) {
        self.live.store(false, Ordering::SeqCst);
        if let Some(reader) = self.reader.lock().unwrap_or_else(|e| e.into_inner()).take() {
            reader.abort();
        }
    }

    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    /// What the record knows about the program with pid `pid`: `None`
    /// unless it's still following the stream, it's about that program,
    /// and it has seen bracketing set or reset.
    pub fn bracketed(&self, pid: u32) -> Option<bool> {
        if !self.is_live() || pid != self.pid {
            return None;
        }
        match self.decsets.lock().unwrap_or_else(|e| e.into_inner()).mode() {
            Mode::On => Some(true),
            Mode::Off => Some(false),
            Mode::Unknown => None,
        }
    }
}

/// Every followed pane's record, by pane id, and when a pane whose
/// subscription failed is next tried.
#[derive(Default)]
pub struct Following {
    records: Mutex<HashMap<String, Arc<Record>>>,
    /// Whether this runner's tmux reports `bracket_paste_flag`: 0 not yet
    /// known, 1 it does, 2 it doesn't. Read once, from the first pane asked.
    tmux_reports: AtomicU8,
    /// Per pane: failures in a row, and when it may be tried again.
    retry: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Following {
    /// Whether tmux has been found to report the mode itself.
    pub fn tmux_reports(&self) -> bool {
        self.tmux_reports.load(Ordering::SeqCst) == 1
    }

    /// Whether `pane` should be followed and isn't: tmux can't report the
    /// mode (or hasn't been asked), no live record covers it, and it isn't
    /// waiting out a failure.
    pub fn wants(&self, pane: &str) -> bool {
        if self.tmux_reports() {
            return false;
        }
        if self.records.lock().unwrap_or_else(|e| e.into_inner()).get(pane).is_some_and(|r| r.is_live()) {
            return false;
        }
        self.retry.lock().unwrap_or_else(|e| e.into_inner()).get(pane).is_none_or(|(_, at)| Instant::now() >= *at)
    }

    pub fn record(&self, pane: &str) -> Option<Arc<Record>> {
        self.records.lock().unwrap_or_else(|e| e.into_inner()).get(pane).cloned()
    }

    /// End `record` and forget it, if it's still `pane`'s.
    pub fn forget(&self, pane: &str, record: &Arc<Record>) {
        record.end();
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.get(pane).is_some_and(|r| Arc::ptr_eq(r, record)) {
            records.remove(pane);
        }
    }

    /// Subscribe to `pane`'s fanout and follow it, unless tmux reports the
    /// mode or a live record already covers the program in it.
    pub async fn follow(self: Arc<Self>, runtime: crate::runtime::Runtime, pane: String) {
        match self.tmux_reports.load(Ordering::SeqCst) {
            1 => return,
            2 => {}
            _ => match runtime.tmux.pane_bracketed_paste(&pane).await {
                Ok(Some(_)) => {
                    self.tmux_reports.store(1, Ordering::SeqCst);
                    return;
                }
                Ok(None) => self.tmux_reports.store(2, Ordering::SeqCst),
                Err(_) => return self.failed(&pane),
            },
        }
        // The pid first: a respawn between this and the subscription leaves
        // a record about the program before it, which is never used.
        let Ok(pid) = runtime.tmux.pane_pid(&pane).await else { return self.failed(&pane) };
        let record = Record::new(pid);
        {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            records.retain(|_, r| r.is_live());
            if let Some(old) = records.get(&pane) {
                if old.pid == pid {
                    return;
                }
                old.end();
            }
            records.insert(pane.clone(), record.clone());
        }
        match runtime.attach_to_fanout(&pane).await {
            Ok(stream) => {
                record.read(stream);
                self.retry.lock().unwrap_or_else(|e| e.into_inner()).remove(&pane);
            }
            Err(_) => {
                record.end();
                self.failed(&pane);
            }
        }
    }

    fn failed(&self, pane: &str) {
        let mut retry = self.retry.lock().unwrap_or_else(|e| e.into_inner());
        let failures = retry.get(pane).map_or(0, |(n, _)| *n) + 1;
        retry.insert(pane.to_string(), (failures, Instant::now() + retry_after(failures)));
    }
}

/// How long to wait after `failures` failed subscriptions in a row.
pub fn retry_after(failures: u32) -> Duration {
    RETRY_FIRST.saturating_mul(1 << failures.saturating_sub(1).min(16)).min(RETRY_MOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_after(chunks: &[&[u8]]) -> Mode {
        let mut d = Decsets::default();
        for chunk in chunks {
            d.feed(chunk);
        }
        d.mode()
    }

    #[test]
    fn nothing_seen_is_unknown_not_off() {
        assert_eq!(mode_after(&[b"hello \x1b[1mworld\x1b[0m\r\n"]), Mode::Unknown);
    }

    #[test]
    fn the_last_set_or_reset_wins() {
        assert_eq!(mode_after(&[b"\x1b[?2004h"]), Mode::On);
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"text\x1b[?2004l"]), Mode::Off);
        assert_eq!(mode_after(&[b"\x1b[?2004l\x1b[?2004h"]), Mode::On);
    }

    #[test]
    fn a_sequence_split_across_reads_still_counts() {
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b", b"[?20", b"04", b"l"]), Mode::Off);
    }

    #[test]
    fn mode_2004_among_others_counts_in_order() {
        assert_eq!(mode_after(&[b"\x1b[?1049;2004;1006h"]), Mode::On);
        assert_eq!(mode_after(&[b"\x1b[?2004h\x1b[?25;2004l"]), Mode::Off);
        assert_eq!(mode_after(&[b"\x1b[?02004h"]), Mode::On, "tmux reads 02004 as 2004");
    }

    #[test]
    fn a_reset_turns_it_off() {
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1bc"]), Mode::Off);
        // ESC # c isn't RIS.
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b#c"]), Mode::On);
    }

    #[test]
    fn what_isnt_decset_2004_changes_nothing() {
        for seq in [
            &b"\x1b[2004h"[..], // ANSI SM, not private
            b"\x1b[>2004h",     // another prefix
            b"\x1b[?2004$h",    // an intermediate
            b"\x1b[?20?04h",    // ignored by tmux
            b"\x1b[?20041h",
            b"\x1b[?2004p",
        ] {
            assert_eq!(mode_after(&[b"\x1b[?2004h", seq]), Mode::On, "{seq:?}");
        }
    }

    #[test]
    fn a_sequence_tmux_might_read_otherwise_is_unknown() {
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b[?2004:1l"]), Mode::Unknown);
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b[?99999999999l"]), Mode::Unknown);
        let long = format!("\x1b[?{}2004l", "1;".repeat(40));
        assert_eq!(mode_after(&[b"\x1b[?2004h", long.as_bytes()]), Mode::Unknown);
    }

    #[test]
    fn a_cancelled_sequence_does_nothing() {
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b[?20\x1804l"]), Mode::On);
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b[?20\x1b[0m04l"]), Mode::On);
    }

    #[test]
    fn strings_carry_no_modes() {
        // An OSC title, ended by BEL and by ST, with 2004 in its text.
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b]0;?2004l\x07"]), Mode::On);
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b]2;[?2004l\x1b\\"]), Mode::On);
        // ESC leaves an OSC or APC at once, in tmux and here.
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1b_abc\x1b[?2004l"]), Mode::Off);
        // A DCS ends at ST, and what follows counts.
        assert_eq!(mode_after(&[b"\x1bP1$r0m\x1b\\\x1b[?2004h"]), Mode::On);
    }

    #[test]
    fn a_csi_inside_a_dcs_is_unknown() {
        // tmux reads it as DCS text, or as CSI if it had given up on the
        // string after five seconds: either way, not proof.
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1bPq#0\x1b[?2004l"]), Mode::Unknown);
        assert_eq!(mode_after(&[b"\x1bPq#0", b"\x1b[?2004h\x1b\\"]), Mode::Unknown);
    }

    #[test]
    fn a_reset_inside_a_dcs_is_unknown() {
        // RIS, if tmux had given up on the string: off. Text if it hadn't.
        assert_eq!(mode_after(&[b"\x1b[?2004h", b"\x1bPq#0\x1bc"]), Mode::Unknown);
    }

    #[test]
    fn a_failed_subscription_waits_longer_each_time() {
        let waits: Vec<u64> = (1..=9).map(|n| retry_after(n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(retry_after(u32::MAX), RETRY_MOST);
    }

    #[test]
    fn a_size_marker_is_a_dcs_and_changes_nothing() {
        let marker = farcooler_vt::size_marker(120, 40);
        assert_eq!(mode_after(&[b"\x1b[?2004h", &marker]), Mode::On);
    }

    #[test]
    fn a_record_counts_only_while_live_and_for_its_program() {
        let r = Record::new(42);
        assert_eq!(r.bracketed(42), None, "nothing seen");
        r.feed(b"\x1b[?2004h");
        assert_eq!(r.bracketed(42), Some(true));
        assert_eq!(r.bracketed(43), None, "a respawned pane is another program");
        r.end();
        assert_eq!(r.bracketed(42), None, "a broken stream proves nothing");
    }
}
